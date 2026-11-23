//! Offline interposition of libc's *own* internal path calls.
//!
//! The runtime hook rewrites GOT slots, which covers every call that goes
//! through a PLT. A c library calling itself does not: `_IO_file_open` calls
//! `open` with a direct branch, nss_files and the resolver call `fopen` the
//! same way. Those sites therefore kept reaching the *host* file system, which
//! is why `/etc/resolv.conf` produced "Temporary failure in name resolution"
//! while `cat /etc/resolv.conf` in the same process printed the guest file.
//!
//! Rewriting the code page at run time is not allowed here (evidence 29: after
//! touching a file-backed executable page, `mprotect(PROT_EXEC)` comes back
//! EACCES and the next fetch faults). So the redirect is baked into the file:
//!
//!   * a stub page (RX, repurposed PT_NOTE) holds, per function, the six
//!     instructions we overwrite, then `b entry+24`;
//!   * a slot page (RW, appended to the last writable PT_LOAD) holds, per
//!     function, its name, a target address the runtime fills in, and the
//!     stub's offset from the page.
//!   * each function entry becomes
//!
//!         adrp x16, slot_page
//!         ldr  x17, [x16, #target]     ; 0 until the shim fills it
//!         cbnz x17, #12                ; -> br x17
//!         ldr  x17, [x16, #delta]      ; build-time: stub - slot page
//!         add  x17, x16, x17
//!         br   x17
//!
//!     which is the original behaviour (via the stub) until something fills the
//!     slot, and the shim's implementation afterwards. Nothing is written to a
//!     code page at run time, and the loader keeps working before we boot: the
//!     guest loader itself calls libc while relocating, long before `_dl_init`.
//!
//! Only functions whose first six instructions are position independent are
//! patched; anything with a PC-relative instruction in that window, a branch
//! into the window, or a neighbouring symbol closer than 24 bytes is refused
//! by name instead of being patched and hoped for.

use crate::{align, die, put_u32, put_u64, u16le, u32le, u64le, Elf, PT_LOAD, PT_NOTE};

pub const SLOT_MAGIC: [u8; 12] = *b"HUSK-SLOT-1\0";
pub const SLOT_HDR: usize = 24;
pub const SLOT_STRIDE: usize = 48;
pub const SLOT_NAME_MAX: usize = 32;

const ALIGN: u64 = 0x4000; // the module has to be loadable on a 16 KiB page
/// Page the slot segment is aligned to. Android arm64 devices ship 4 KiB, 16 KiB
/// and 64 KiB pages, and glibc rejects a PT_LOAD whose p_align is not a multiple
/// of the running page size, so the segment we add is 64 KiB aligned: a multiple
/// of every page size in use, and the same value Ubuntu's own libc uses.
const SEG_ALIGN: u64 = 0x10000;
const PATCH_LEN: usize = 6; // instructions overwritten at the entry
// One slot per entry, plus one for the branch back, plus two per window
// instruction: a `b.cond`/`cbz`/`tbz` whose target falls out of the ±1 MiB
// reach after the move is re-expressed as an inverted short branch over a
// full-width `b`, which is a two-instruction form.
const STUB_LEN: usize = PATCH_LEN * 3 + 1;
const SHT_GNU_VERSYM: u32 = 0x6FFF_FFFF;

// ---------------------------------------------------------------- encodings

fn adrp(rd: u32, from: u64, to: u64) -> u32 {
    let delta = (((to as i64) & !0xfff) - ((from as i64) & !0xfff)) >> 12;
    if !(-(1 << 20)..(1 << 20)).contains(&delta) {
        die(format!("adrp out of range: {delta} pages"));
    }
    let d = delta as u32;
    0x9000_0000 | ((d & 0x3) << 29) | (((d >> 2) & 0x7ffff) << 5) | rd
}

fn ldr_x(dst: u32, base: u32, off: u64) -> u32 {
    if off % 8 != 0 || off >= 32768 {
        die(format!("ldr offset {off:#x} is not encodable"));
    }
    0xF940_0000 | (((off / 8) as u32) << 10) | (base << 5) | dst
}

fn cbnz_x(rt: u32, bytes: i32) -> u32 {
    if bytes % 4 != 0 {
        die("cbnz offset must be a multiple of 4");
    }
    let imm19 = (bytes / 4) as u32 & 0x7ffff;
    0xB500_0000 | (imm19 << 5) | rt
}

fn b_bytes(from: u64, to: u64) -> u32 {
    let d = (to as i64 - from as i64) / 4;
    if !(-(1 << 25)..(1 << 25)).contains(&d) {
        die(format!("b out of range: {d} instructions"));
    }
    0x1400_0000 | ((d as u32) & 0x03ff_ffff)
}

/// `br x17`
const BR_X17: u32 = 0xD61F_0220;
/// `add x17, x16, x17`
const ADD_X17_X16_X17: u32 = 0x8B11_0211;

// ------------------------------------------------------------ instruction scan

/// Copy the overwritten instructions into the stub page, fixing up every
/// PC-relative reference. AArch64 prologues routinely start with
/// `adrp x3, page; ldr x3, [x3, #off]` (a global such as `__libc_single_threaded`),
/// so refusing those outright would leave most of the libc surface unpatched;
/// relocating them is exact. Anything whose reach does not survive the move is
/// refused with a reason, never patched and hoped for.
fn relocate(words: &[u32], orig_pc: u64, new_pc: u64) -> Result<Vec<u32>, String> {
    let mut out = Vec::with_capacity(words.len());
    let mut k = 0usize;
    while k < words.len() {
        let w = words[k];
        let from = orig_pc + k as u64 * 4;
        let imm21 = |w: u32| -> i64 {
            let immlo = ((w >> 29) & 0x3) as u64;
            let immhi = ((w >> 5) & 0x7ffff) as u64;
            sign_extend((immhi << 2) | immlo, 21)
        };
        let enc_imm21 = |base: u32, imm: i64| -> Result<u32, String> {
            if !(-(1 << 20)..(1 << 20)).contains(&imm) {
                return Err(format!("21-bit immediate out of range: {imm}"));
            }
            let d = imm as u32;
            Ok(base | ((d & 0x3) << 29) | (((d >> 2) & 0x7ffff) << 5) | (w & 0x1f))
        };
        let enc_imm19 = |imm: i64| -> Result<u32, String> {
            if imm % 4 != 0 || !(-(1 << 20)..(1 << 20)).contains(&imm) {
                return Err(format!("19-bit branch out of range: {imm}"));
            }
            let d = ((imm / 4) as u32) & 0x7ffff;
            Ok((w & !(0x7ffff << 5)) | (d << 5))
        };
        let enc_imm14 = |imm: i64| -> Result<u32, String> {
            if imm % 4 != 0 || !(-(1 << 15)..(1 << 15)).contains(&imm) {
                return Err(format!("14-bit branch out of range: {imm}"));
            }
            let d = ((imm / 4) as u32) & 0x3fff;
            Ok((w & !(0x3fff << 5)) | (d << 5))
        };
        let enc_imm26 = |imm: i64| -> Result<u32, String> {
            if imm % 4 != 0 || !(-(1 << 27)..(1 << 27)).contains(&imm) {
                return Err(format!("26-bit branch out of range: {imm}"));
            }
            Ok((w & 0xfc00_0000) | (((imm / 4) as u32) & 0x03ff_ffff))
        };

        let to = new_pc + out.len() as u64 * 4;
        let mut emit = |v: u32, out: &mut Vec<u32>| out.push(v);

        let nw = if (w & 0x1f00_0000) == 0x1000_0000 {
            // adr / adrp: absolute target, then re-express from the new place
            let is_adrp = (w & 0x8000_0000) != 0;
            let target = if is_adrp {
                ((from & !0xfff) as i64 + imm21(w) * 4096) as u64
            } else {
                (from as i64 + imm21(w)) as u64
            };
            let base = if is_adrp { 0x9000_0000 } else { 0x1000_0000 };
            let imm = if is_adrp {
                ((target & !0xfff) as i64 - (to & !0xfff) as i64) >> 12
            } else {
                target as i64 - to as i64
            };
            enc_imm21(base, imm)?
        } else if (w & 0x3b00_0000) == 0x1800_0000 {
            // ldr (literal) / ldrsw: imm19 * 4 from the instruction
            let imm = sign_extend(((w >> 5) & 0x7ffff) as u64, 19) * 4;
            let target = (from as i64 + imm) as u64;
            let delta = target as i64 - to as i64;
            if delta % 4 != 0 || !(-(1 << 20)..(1 << 20)).contains(&delta) {
                return Err(format!("literal load out of range: {delta}"));
            }
            enc_imm19(delta)?
        } else if (w & 0x7c00_0000) == 0x1400_0000 {
            // b / bl
            let imm = sign_extend((w & 0x03ff_ffff) as u64, 26) * 4;
            enc_imm26((from as i64 + imm) - to as i64)?
        } else if (w & 0xff00_0000) == 0x5400_0000 {
            // b.cond: imm19, ±1 MiB around the *new* address
            let target = from as i64 + sign_extend(((w >> 5) & 0x7ffff) as u64, 19) * 4;
            match enc_imm19(target - to as i64) {
                Ok(v) => v,
                Err(_) => {
                    // invert the condition, branch over the full-width `b`
                    let inv = (w & !0xf) | ((w & 0xf) ^ 1);
                    emit((inv & !(0x7ffff << 5)) | ((8 / 4) << 5), &mut out);
                    emit(b_bytes(to + 4, target as u64), &mut out);
                    k += 1;
                    continue;
                }
            }
        } else if (w & 0x7e00_0000) == 0x3400_0000 {
            // cbz / cbnz: imm19, ±1 MiB
            let target = from as i64 + sign_extend(((w >> 5) & 0x7ffff) as u64, 19) * 4;
            match enc_imm19(target - to as i64) {
                Ok(v) => v,
                Err(_) => {
                    let inv = w ^ 0x0100_0000; // cbnz <-> cbz
                    emit((inv & !(0x7ffff << 5)) | ((8 / 4) << 5), &mut out);
                    emit(b_bytes(to + 4, target as u64), &mut out);
                    k += 1;
                    continue;
                }
            }
        } else if (w & 0x7e00_0000) == 0x3600_0000 {
            // tbz / tbnz: imm14, ±32 KiB
            let target = from as i64 + sign_extend(((w >> 5) & 0x3fff) as u64, 14) * 4;
            match enc_imm14(target - to as i64) {
                Ok(v) => v,
                Err(_) => {
                    let inv = w ^ 0x0100_0000; // tbnz <-> tbz
                    emit((inv & !(0x3fff << 5)) | ((8 / 4) << 5), &mut out);
                    emit(b_bytes(to + 4, target as u64), &mut out);
                    k += 1;
                    continue;
                }
            }
        } else {
            w
        };
        out.push(nw);
        k += 1;
    }
    Ok(out)
}

fn sign_extend(v: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((v << shift) as i64) >> shift
}

/// Every address that is the target of a branch or ADR/ADRP in the executable
/// segments. A patched entry must not be inside one of these.
fn branch_targets(e: &Elf) -> Vec<u64> {
    let mut out = Vec::new();
    for (start, end) in e.exec_ranges() {
        let mut a = start;
        while a + 4 <= end {
            let off = match e.try_vaddr_to_off(a) {
                Some(o) if o + 4 <= e.data.len() => o,
                _ => {
                    a += 4;
                    continue;
                }
            };
            let w = u32le(&e.data, off);
            let pc = a;
            if (w & 0x7C00_0000) == 0x1400_0000 {
                out.push(pc.wrapping_add((sign_extend((w & 0x03ff_ffff) as u64, 26) * 4) as u64));
            } else if (w & 0xFF00_0000) == 0x5400_0000 {
                out.push(pc.wrapping_add((sign_extend(((w >> 5) & 0x7ffff) as u64, 19) * 4) as u64));
            } else if (w & 0x7E00_0000) == 0x3400_0000 {
                out.push(pc.wrapping_add((sign_extend(((w >> 5) & 0x7ffff) as u64, 19) * 4) as u64));
            } else if (w & 0x7E00_0000) == 0x3600_0000 {
                out.push(pc.wrapping_add((sign_extend(((w >> 5) & 0x3fff) as u64, 14) * 4) as u64));
            } else if (w & 0x1F00_0000) == 0x1000_0000 {
                let immlo = ((w >> 29) & 0x3) as u64;
                let immhi = ((w >> 5) & 0x7ffff) as u64;
                let imm = sign_extend((immhi << 2) | immlo, 21);
                let page = if (w & 0x8000_0000) != 0 {
                    pc & !0xfff
                } else {
                    0
                };
                out.push((page as i64 + imm * 4096) as u64);
            }
            a += 4;
        }
    }
    out
}

// ------------------------------------------------------------------- symbols

struct Sym {
    name: String,
    value: u64,
    size: u64,
}

fn dynsyms(e: &Elf) -> Vec<Sym> {
    let secs = crate::sections(e);
    let dynsym = secs
        .get(b".dynsym" as &[u8])
        .unwrap_or_else(|| die("libc has no .dynsym"));
    let dynstr = secs
        .get(b".dynstr" as &[u8])
        .unwrap_or_else(|| die("libc has no .dynstr"));
    let nsyms = (dynsym.size / 24) as usize;
    let mut out = Vec::with_capacity(nsyms);
    for i in 0..nsyms {
        let o = dynsym.off as usize + i * 24;
        let st_name = u32le(&e.data, o) as usize;
        let st_info = e.data.get(o + 4).copied().unwrap_or(0);
        let st_shndx = u16le(&e.data, o + 6);
        let value = u64le(&e.data, o + 8);
        let size = u64le(&e.data, o + 16);
        let typ = st_info & 0xf;
        if st_shndx == 0 || (typ != 2 && typ != 10) {
            continue; // undefined, or not a function
        }
        let name = crate::cstr(&e.data, dynstr.off as usize + st_name);
        out.push(Sym {
            name: String::from_utf8_lossy(name).to_string(),
            value,
            size,
        });
    }
    out
}

/// Version index per dynamic symbol, so a compat definition can be skipped in
/// favour of the default one.
fn versyms(e: &Elf) -> Vec<u16> {
    for i in 0..e.e_shnum as usize {
        let s = e.section(i);
        if s.0 == SHT_GNU_VERSYM {
            let (_, _, off, size, _, _) = s;
            let mut out = Vec::with_capacity((size / 2) as usize);
            let mut p = 0;
            while p + 2 <= size {
                out.push(u16le(&e.data, off as usize + p as usize));
                p += 2;
            }
            return out;
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------- main

pub fn main(args: &[String]) {
    if args.len() < 3 {
        die("usage: nr-tools libc-interpose <libc.so.6> <out.so.6> <names.txt> [--expect N] [--report]");
    }
    let (input, out_path, names_path) = (&args[0], &args[1], &args[2]);
    let mut expect: Option<usize> = None;
    let mut report = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--expect" => {
                i += 1;
                expect = Some(
                    args.get(i)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| die("--expect needs a number")),
                );
            }
            "--report" => report = true,
            other => die(format!("unknown option {other}")),
        }
        i += 1;
    }

    let e = Elf::open(input);
    if let Some(v) = e.glibc_version() {
        println!("{input}: glibc {v}");
    }
    let names: Vec<String> = std::fs::read_to_string(names_path)
        .unwrap_or_else(|err| die(format!("{names_path}: {err}")))
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let syms = dynsyms(&e);
    let vers = versyms(&e);

    // PT_NOTE is the only segment that can be repurposed without growing the
    // program header table, which is what patchelf-style tools do and what
    // `embed` already does for the loader.
    let notes: Vec<(usize, crate::Phdr)> = e
        .phdrs
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, p)| p.typ == PT_NOTE)
        .collect();
    if notes.len() != 1 {
        die(format!("expected exactly one PT_NOTE, found {}", notes.len()));
    }
    let note_idx = notes[0].0;

    // The stub page wants to be *near* .text: a handful of glibc functions
    // begin with a ±1 MiB (`b.cond`/`cbz`) or ±32 KiB (`tbz`) branch, and a copy
    // of such a window can only be moved within its reach. The gap between the
    // executable LOAD and the next one is unused padding in a normal libc, so
    // the stub page goes there when it fits and at the end of the file when it
    // does not (those few entries are then refused by name, not patched badly).
    let rx = e
        .phdrs
        .iter()
        .filter(|p| p.typ == PT_LOAD && p.flags & 1 != 0)
        .max_by_key(|p| p.filesz)
        .copied()
        .unwrap_or_else(|| die("libc has no executable PT_LOAD"));
    // The slot page grows the last writable LOAD, so no new PHDR is needed for
    // it either.
    let rw_idx = e
        .phdrs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.typ == PT_LOAD && p.flags & 2 != 0)
        .map(|(i, p)| (i, *p))
        .max_by_key(|(_, p)| p.vaddr + p.memsz)
        .unwrap_or_else(|| die("libc has no writable PT_LOAD"));
    let rw = rw_idx.1;

    let targets = branch_targets(&e);
    let mut picked: Vec<(String, u64, Vec<u32>)> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();

    for name in &names {
        // Default version wins over a compat alias.
        let mut cands: Vec<&Sym> = syms.iter().filter(|s| &s.name == name).collect();
        if cands.is_empty() {
            skipped.push((name.clone(), "not exported by this libc".into()));
            continue;
        }
        cands.sort_by_key(|s| {
            let idx = syms.iter().position(|x| x.value == s.value && x.name == s.name);
            match idx.and_then(|i| vers.get(i)) {
                Some(v) => v & 0x7fff,
                None => 0,
            }
        });
        let s = cands[cands.len() - 1];
        if s.value % 4 != 0 {
            skipped.push((name.clone(), "entry is not instruction aligned".into()));
            continue;
        }
        // A neighbouring symbol must not live inside the window.
        if syms
            .iter()
            .any(|o| o.value > s.value && o.value < s.value + (PATCH_LEN * 4) as u64)
        {
            skipped.push((name.clone(), "another symbol starts inside the window".into()));
            continue;
        }
        let off = match e.try_vaddr_to_off(s.value) {
            Some(o) => o,
            None => {
                skipped.push((name.clone(), "entry is not file-backed".into()));
                continue;
            }
        };
        let mut words = Vec::with_capacity(PATCH_LEN);
        for k in 0..PATCH_LEN {
            words.push(u32le(&e.data, off + k * 4));
        }
        if targets
            .iter()
            .any(|t| *t > s.value && *t < s.value + (PATCH_LEN * 4) as u64)
        {
            skipped.push((name.clone(), "something branches into the window".into()));
            continue;
        }
        picked.push((name.clone(), s.value, words));
    }

    if let Some(want) = expect {
        if picked.len() != want {
            die(format!(
                "expected {want} patched entries, got {} (refusing to write)",
                picked.len()
            ));
        }
    }

    // ---------------------------------------------------------------- layout
    //
    // Both pages live in the zero padding between the executable LOAD and the
    // next one. Nothing else will do:
    //
    //   * the stub page must be *near* .text -- a copied window can contain a
    //     ±32 KiB (`tbz`) or ±1 MiB (`cbz`, `b.cond`) branch, which cannot reach
    //     a page appended at the end of the file;
    //   * the slot page must be writable while the stub page is executable;
    //   * neither may land inside the data LOAD's .bss. Extending p_filesz over
    //     it would turn background zeros into file content (the section header
    //     table sits exactly there), and libc's uninitialised globals would come
    //     up holding garbage.
    let cap = names.len();
    let file_len = e.data.len() as u64;

    let gap_lo = align(rx.off + rx.filesz, 0x1000);
    let gap_hi = e
        .phdrs
        .iter()
        .filter(|p| p.typ == PT_LOAD && p.off >= rx.off + rx.filesz)
        .map(|p| p.off)
        .min()
        .unwrap_or(u64::MAX);
    let (stub_off, stub_vaddr) = (gap_lo, rx.vaddr + (gap_lo - rx.off));

    // The stub page is packed sequentially, not at a fixed stride: a window
    // whose `b.cond`/`cbz`/`tbz` falls out of reach grows by one instruction,
    // and the next stub starts after it. Offsets are recorded per entry, so the
    // runtime does not need a stride.
    let mut cursor = 0usize;
    let mut placed: Vec<(String, u64, Vec<u32>)> = Vec::with_capacity(picked.len());
    for (name, entry, raw) in picked.iter() {
        let stub = stub_vaddr + (cursor * 4) as u64;
        match relocate(raw, *entry, stub) {
            Ok(m) => {
                cursor += m.len() + 1;
                placed.push((name.clone(), *entry, m));
            }
            Err(why) => skipped.push((
                name.clone(),
                format!("window would not survive the move ({why})"),
            )),
        }
    }
    let stub_len = align((cursor * 4) as u64, 0x1000).max(0x1000);
    if gap_lo + stub_len > gap_hi {
        die(format!(
            "the padding after the executable LOAD is {:#x} bytes, the stub page needs {:#x}",
            gap_hi - gap_lo,
            stub_len
        ));
    }
    if !e.data[stub_off as usize..(stub_off + stub_len) as usize]
        .iter()
        .all(|b| *b == 0)
    {
        die(format!("padding at {stub_off:#x} is not empty; refusing to overwrite it"));
    }

    // The slot page goes *past the end of the last LOAD*, not next to the stub.
    //
    // glibc's dl-load rejects any PT_LOAD whose p_align is not a multiple of the
    // running page size ("ELF load command alignment not page-aligned"), and
    // Android arm64 ships 4 KiB, 16 KiB and 64 KiB pages. A segment squeezed
    // into the hole between two existing LOADs cannot be 64 KiB aligned there -
    // this libc's hole is 0x3000 bytes - so a page-sized slot page placed in the
    // gap loads on a 4 KiB device and fails to load at all on a 16 KiB one.
    // At the end of the file both the file offset and the vaddr can be chosen a
    // multiple of SEG_ALIGN, which makes the segment valid on every page size.
    //
    // The entry patched into .text reaches the slot with `adrp` (±4 GiB), so
    // distance from .text is irrelevant; only the stub page has to stay close.
    let slot_len = align((SLOT_HDR + cap * SLOT_STRIDE) as u64, 0x1000);
    let slot_off = align(file_len, SEG_ALIGN);
    let loaded_end = e
        .phdrs
        .iter()
        .filter(|p| p.typ == PT_LOAD)
        .map(|p| p.vaddr + p.memsz)
        .max()
        .unwrap_or(0);
    let slot_vaddr = align(loaded_end.max(slot_off), SEG_ALIGN);
    let slot_end = slot_off + slot_len;

    let mut d = e.data.clone();
    if d.len() < slot_end as usize {
        d.resize(slot_end as usize, 0);
    }
    d[slot_off as usize..slot_off as usize + 12].copy_from_slice(&SLOT_MAGIC);
    put_u32(&mut d, slot_off as usize + 12, cap as u32);
    put_u32(&mut d, slot_off as usize + 16, SLOT_STRIDE as u32);

    let mut ok = 0usize;
    let mut at = 0usize; // instructions written so far in the stub page
    for (i, (name, entry, moved)) in placed.iter().enumerate() {
        let stub = stub_vaddr + (at * 4) as u64;
        at += moved.len() + 1;
        let slot_i = names.iter().position(|n| n == name).unwrap_or(i);
        let e_off = slot_off as usize + SLOT_HDR + slot_i * SLOT_STRIDE;
        let nm = name.as_bytes();
        if nm.len() >= SLOT_NAME_MAX {
            die(format!("{name}: name longer than {SLOT_NAME_MAX} bytes"));
        }
        d[e_off..e_off + nm.len()].copy_from_slice(nm);
        put_u64(&mut d, e_off + 32, 0); // target: filled by the runtime
        // The stub sits below the slot page now, so the delta is negative: the
        // runtime adds it to the slot page base and the arithmetic wraps.
        put_u64(&mut d, e_off + 40, stub.wrapping_sub(slot_vaddr));

        let s_off = (stub - stub_vaddr + stub_off) as usize;
        for (k, w) in moved.iter().enumerate() {
            put_u32(&mut d, s_off + k * 4, *w);
        }
        // Back to the first instruction after the six overwritten at the entry.
        // The window may have grown while being relocated, so the landing point
        // is measured from the rewritten length, not from PATCH_LEN.
        put_u32(
            &mut d,
            s_off + moved.len() * 4,
            b_bytes(
                stub + (moved.len() * 4) as u64,
                entry + (PATCH_LEN * 4) as u64,
            ),
        );

        let a = e.try_vaddr_to_off(*entry).unwrap();
        put_u32(&mut d, a, adrp(16, *entry, slot_vaddr));
        put_u32(&mut d, a + 4, ldr_x(17, 16, (SLOT_HDR + slot_i * SLOT_STRIDE + 32) as u64));
        put_u32(&mut d, a + 8, cbnz_x(17, 12));
        put_u32(&mut d, a + 12, ldr_x(17, 16, (SLOT_HDR + slot_i * SLOT_STRIDE + 40) as u64));
        put_u32(&mut d, a + 16, ADD_X17_X16_X17);
        put_u32(&mut d, a + 20, BR_X17);
        ok += 1;
    }


    // ------------------------------------------------------------ program headers
    //
    // The executable LOAD grows over the stub page, and the note segment slot
    // becomes a writable LOAD for the slot page. It has to sit between the two
    // real LOADs: the loader walks LOADs in table order and expects their
    // addresses to increase.
    let po = e.e_phoff as usize;
    let rx_i = e
        .phdrs
        .iter()
        .position(|p| p.typ == PT_LOAD && p.off == rx.off && p.vaddr == rx.vaddr)
        .unwrap_or_else(|| die("executable segment vanished"));
    put_u64(&mut d, po + rx_i * 56 + 32, stub_off + stub_len - rx.off);
    put_u64(&mut d, po + rx_i * 56 + 40, stub_off + stub_len - rx.off);

    // p_type | p_flags<<32, then the six 8-byte fields; p_align is written last.
    let slot_phdr: [u64; 6] = [
        PT_LOAD as u64 | (6u64 << 32), // R|W
        slot_off,
        slot_vaddr,
        slot_vaddr,
        slot_len,
        slot_len,
    ];
    let mut order: Vec<([u64; 6], u64)> = Vec::with_capacity(e.phdrs.len());
    for (i, p) in e.phdrs.iter().enumerate() {
        if i == note_idx {
            // The note segment becomes the slot LOAD, in its own table slot: the
            // note header already sits after the last real LOAD, so the LOAD
            // list stays sorted by p_vaddr even though the slot page itself
            // lives past the end of the file.
            order.push((slot_phdr, SEG_ALIGN));
            continue;
        }
        let row = [
            p.typ as u64 | ((p.flags as u64) << 32),
            p.off,
            p.vaddr,
            p.vaddr,
            if i == rx_i { stub_off + stub_len - rx.off } else { p.filesz },
            if i == rx_i { stub_off + stub_len - rx.off } else { p.memsz },
        ];
        order.push((row, p.align));
    }
    if order.len() != e.phdrs.len() {
        die("program header bookkeeping mismatch");
    }
    // Every LOAD has to survive all three page sizes we claim to support:
    //   * p_align must be a power of two and a multiple of the page size, or
    //     glibc's dl-load refuses the object outright ("ELF load command
    //     alignment not page-aligned");
    //   * p_vaddr and p_offset must be congruent modulo p_align, and therefore
    //     modulo every page size the object is ever loaded on;
    //   * the LOADs must stay sorted by p_vaddr.
    let mut prev_vaddr = 0u64;
    let mut have_prev = false;
    for (row, al) in &order {
        if row[0] as u32 != PT_LOAD {
            continue;
        }
        let (off, vaddr) = (row[1], row[2]);
        if *al == 0 || al & (al - 1) != 0 {
            die(format!("p_align {al:#x} is not a power of two"));
        }
        if al % 0x4000 != 0 {
            die(format!(
                "PT_LOAD at {vaddr:#x} has p_align {al:#x}, which a 16 KiB page refuses"
            ));
        }
        if vaddr.wrapping_sub(off) & (al - 1) != 0 {
            die(format!(
                "PT_LOAD at {vaddr:#x} off {off:#x}: vaddr and offset differ modulo p_align {al:#x}"
            ));
        }
        if have_prev && vaddr < prev_vaddr {
            die(format!("PT_LOAD at {vaddr:#x} is out of order (previous {prev_vaddr:#x})"));
        }
        prev_vaddr = vaddr;
        have_prev = true;
    }
    for (i, (row, align_v)) in order.iter().enumerate() {
        let o = po + i * 56;
        for (k, v) in row.iter().enumerate() {
            put_u64(&mut d, o + k * 8, *v);
        }
        put_u64(&mut d, o + 48, *align_v);
    }

    std::fs::write(out_path, &d).unwrap_or_else(|err| die(format!("{out_path}: {err}")));

    println!(
        "{out_path}: {ok} of {cap} entries patched; stub page {stub_vaddr:#x} ({stub_len:#x} bytes, R|X), slot page {slot_vaddr:#x} ({slot_len:#x} bytes, R|W)"
    );
    if !skipped.is_empty() {
        println!("skipped {}:", skipped.len());
        for (n, why) in &skipped {
            println!("  {n}: {why}");
        }
    }
    if report {
        for (name, entry, _) in &picked {
            println!("  {entry:#014x}  {name}");
        }
    }
}
