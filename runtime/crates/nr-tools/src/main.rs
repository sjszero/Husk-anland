//! nr-tools - the host-side build tools for the LinuxHub Husk loader.
//!
//! Rust port of the original tools/loader Python scripts (removed from the tree
//! after byte-for-byte verification; see tag v0.1.0). The rule
//! for the port is byte identity: for the same inputs these must produce the
//! same output bytes as the Python tools (the loader's sha256 is the test).
//!
//!   nr-tools scan-syscalls report <elf> [--version V] [--expect NAME=COUNT ...]
//!   nr-tools scan-syscalls patch  <elf> <out> [--version V] [--expect ...]
//!   nr-tools embed <shim.so> <loader.orig> <hook.bin> <out.so>
//!   nr-tools provision [--no-push] [--stage DIR] [--cache DIR] [--serial S]
//!       (Rust port of tools/device-tests/provision-toolchain.sh)
//!
//! Why a scan and not an offset table: see the plan (docs/design/runtime.md).
//! Every expectation below is an assertion; a different glibc must fail loudly.

mod libc_slots;
mod provision;
mod rootfs;
mod status;

use std::fs;
use std::process::ExitCode;

// ------------------------------------------------------------------ ELF basics

fn u16le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn u64le(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([
        d[o],
        d[o + 1],
        d[o + 2],
        d[o + 3],
        d[o + 4],
        d[o + 5],
        d[o + 6],
        d[o + 7],
    ])
}
fn i64le(d: &[u8], o: usize) -> i64 {
    u64le(d, o) as i64
}
fn put_u32(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_le_bytes())
}
fn put_u64(d: &mut [u8], o: usize, v: u64) {
    d[o..o + 8].copy_from_slice(&v.to_le_bytes())
}

fn cstr(d: &[u8], off: usize) -> &[u8] {
    let end = d[off..]
        .iter()
        .position(|b| *b == 0)
        .map(|n| off + n)
        .unwrap_or(d.len());
    &d[off..end]
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct Phdr {
    typ: u32,
    flags: u32,
    off: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

struct Elf {
    data: Vec<u8>,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_shentsize: u16,
    e_shnum: u16,
    phdrs: Vec<Phdr>,
}

const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const PAGE: u64 = 0x1000;

fn die(msg: impl AsRef<str>) -> ! {
    // Same shape as the Python tools' assertion failures: one line, exit 1.
    eprintln!("{}", msg.as_ref());
    std::process::exit(1)
}

impl Elf {
    fn open(path: &str) -> Elf {
        let data = fs::read(path).unwrap_or_else(|e| die(format!("{path}: {e}")));
        let d = &data;
        if d.len() < 64 || &d[..4] != b"\x7fELF" || d[4] != 2 || &d[18..20] != b"\xb7\x00" {
            die(format!("{path}: not an aarch64 ELF64"));
        }
        let e_entry = u64le(d, 0x18);
        let e_phoff = u64le(d, 0x20);
        let e_shoff = u64le(d, 0x28);
        let e_phentsize = u16le(d, 0x36);
        let e_phnum = u16le(d, 0x38);
        let e_shentsize = u16le(d, 0x3A);
        let e_shnum = u16le(d, 0x3C);
        if e_phentsize != 56 {
            die(format!("{path}: unexpected e_phentsize {e_phentsize}"));
        }
        let mut phdrs = Vec::with_capacity(e_phnum as usize);
        for i in 0..e_phnum as usize {
            let o = e_phoff as usize + i * 56;
            phdrs.push(Phdr {
                typ: u32le(d, o),
                flags: u32le(d, o + 4),
                off: u64le(d, o + 8),
                vaddr: u64le(d, o + 16),
                filesz: u64le(d, o + 32),
                memsz: u64le(d, o + 40),
                align: u64le(d, o + 48),
            });
        }
        Elf {
            data,
            e_entry,
            e_phoff,
            e_shoff,
            e_shentsize,
            e_shnum,
            phdrs,
        }
    }

    fn vaddr_to_off(&self, vaddr: u64) -> usize {
        for p in &self.phdrs {
            if p.typ == PT_LOAD && p.vaddr <= vaddr && vaddr < p.vaddr + p.filesz {
                return (p.off + (vaddr - p.vaddr)) as usize;
            }
        }
        die(format!("{vaddr:#x} is not file-backed"))
    }

    fn try_vaddr_to_off(&self, vaddr: u64) -> Option<usize> {
        for p in &self.phdrs {
            if p.typ == PT_LOAD && p.vaddr <= vaddr && vaddr < p.vaddr + p.filesz {
                return Some((p.off + (vaddr - p.vaddr)) as usize);
            }
        }
        None
    }

    fn exec_ranges(&self) -> Vec<(u64, u64)> {
        self.phdrs
            .iter()
            .filter(|p| p.typ == PT_LOAD && p.flags & 1 != 0)
            .map(|p| (p.vaddr, p.vaddr + p.filesz))
            .collect()
    }

    fn section(&self, idx: usize) -> (u32, u64, u64, u64, u32, u64) {
        let o = self.e_shoff as usize + idx * self.e_shentsize as usize;
        (
            u32le(&self.data, o + 4),
            u64le(&self.data, o + 16),
            u64le(&self.data, o + 24),
            u64le(&self.data, o + 32),
            u32le(&self.data, o + 40),
            u64le(&self.data, o + 56),
        )
    }

    /// Highest GLIBC_x.y named in .gnu.version_d (base version excluded).
    fn glibc_version(&self) -> Option<String> {
        const SHT_GNU_VERDEF: u32 = 0x6FFF_FFFD;
        let mut verdef = None;
        for i in 0..self.e_shnum as usize {
            let s = self.section(i);
            if s.0 == SHT_GNU_VERDEF {
                verdef = Some(s);
                break;
            }
        }
        let (_, _, vd_off, vd_size, link, _) = verdef?;
        let strtab = self.section(link as usize);
        let base = strtab.2 as usize;
        let mut best: Option<String> = None;
        let mut p = 0u64;
        while p < vd_size {
            let o = vd_off as usize + p as usize;
            let vd_flags = u16le(&self.data, o + 2);
            let vd_aux = u32le(&self.data, o + 12);
            let vd_next = u32le(&self.data, o + 16);
            let name_off = u32le(&self.data, (o as u64 + vd_aux as u64) as usize) as usize;
            let name = String::from_utf8_lossy(cstr(&self.data, base + name_off)).to_string();
            if let Some(ver) = name.strip_prefix("GLIBC_") {
                let ok = ver.split('.').count() == 2
                    && ver
                        .split('.')
                        .all(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()));
                if ok && vd_flags & 1 == 0 {
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            let k = |s: &str| -> (u64, u64) {
                                let mut it = s.split('.');
                                (
                                    it.next().unwrap().parse().unwrap(),
                                    it.next().unwrap().parse().unwrap(),
                                )
                            };
                            k(ver) > k(b)
                        }
                    };
                    if better {
                        best = Some(ver.to_string());
                    }
                }
            }
            if vd_next == 0 {
                break;
            }
            p += vd_next as u64;
        }
        best
    }
}

// ------------------------------------------------------------ AArch64 decoding

const SVC: u32 = 0xD400_0001;
const NOP: u32 = 0xD503_201F;
const MOVN_X0_37: u32 = 0x9280_04A0; // -38 == -ENOSYS
const CMP_X0_0: u32 = 0xF100_001F;

/// Accelerators we neutralise: the caller provably ignores the result.
fn patchable(nr: u32) -> bool {
    nr == 99
}

/// Syscalls that must return ENOSYS without entering Android's seccomp trap.
/// The caller already has a legacy fallback for ENOSYS; unlike `patchable`,
/// its result is deliberately inspected.
fn enosys_patchable(nr: u32, next: u32) -> bool {
    nr == 435 && next == CMP_X0_0
}

/// Syscalls we locate in reports. `clone3` is patched to ENOSYS; `rseq` is
/// disabled by GLIBC_TUNABLES because its callers inspect the result.
fn accelerator_name(nr: u32) -> Option<&'static str> {
    match nr {
        99 => Some("set_robust_list"),
        293 => Some("rseq"),
        435 => Some("clone3"),
        _ => None,
    }
}

/// movz {x,w}<rd>, #imm -> (imm, true)
fn movz_imm(insn: u32, rd: u32) -> Option<u32> {
    if insn & 0xFFE0_001F == 0xD280_0000 | rd {
        return Some((insn >> 5) & 0xFFFF);
    }
    if insn & 0xFFE0_001F == 0x5280_0000 | rd {
        return Some((insn >> 5) & 0xFFFF);
    }
    None
}

/// True if this instruction writes x0/w0 and cannot depend on its old value.
fn writes_x0_only(insn: u32) -> bool {
    if insn & 0x1F != 0 {
        return false;
    }
    let top = insn & 0x7F80_0000;
    if matches!(
        top,
        0x5280_0000 | 0x1280_0000 | 0x7280_0000 | 0xD280_0000 | 0x9280_0000 | 0xF280_0000
    ) {
        return true;
    }
    let m = insn & 0x7F00_0000;
    if m == 0x1100_0000 || m == 0x5100_0000 {
        return (insn >> 5) & 0x1F != 0;
    }
    let m2 = insn & 0x7FE0_0000;
    if m2 == 0x0B00_0000 || m2 == 0x4B00_0000 {
        return ((insn >> 5) & 0x1F) != 0 && ((insn >> 16) & 0x1F) != 0;
    }
    if insn & 0x3B00_0000 == 0x3900_0000 {
        return (insn >> 5) & 0x1F != 0;
    }
    if insn & 0x3B20_0C00 == 0x3800_0000 {
        return (insn >> 5) & 0x1F != 0;
    }
    let m3 = insn & 0x7FE0_FFE0;
    if m3 == 0x2A00_03E0 || m3 == 0xAA00_03E0 {
        return (insn >> 16) & 0x1F != 0;
    }
    false
}

fn is_branch(insn: u32) -> bool {
    insn & 0x7C00_0000 == 0x1400_0000
        || insn & 0xFE00_0000 == 0x5400_0000
        || insn & 0x7E00_0000 == 0x3400_0000
        || insn & 0x7E00_0000 == 0x3600_0000
        || matches!(
            insn & 0xFFFF_FC1F,
            0xD61F_0000 | 0xD63F_0000 | 0xD65F_0000 | 0xD67F_0000
        )
        || insn == 0xD65F_03C0
}

struct Site {
    vaddr: u64,
    nr: u32,
    name: &'static str,
    back: Vec<u32>,
    next: u32,
}

fn scan(elf: &Elf) -> Vec<Site> {
    let mut sites = Vec::new();
    for (start, end) in elf.exec_ranges() {
        let base = elf.vaddr_to_off(start);
        let mut vaddr = start;
        while vaddr + 4 <= end {
            let o = base + (vaddr - start) as usize;
            if u32le(&elf.data, o) == SVC {
                let mut nr = None;
                let mut back = Vec::new();
                for k in 1..9usize {
                    let po = o as i64 - 4 * k as i64;
                    if po < 0 {
                        break;
                    }
                    let prev = u32le(&elf.data, po as usize);
                    if prev == SVC {
                        break;
                    }
                    if let Some(imm) = movz_imm(prev, 8) {
                        nr = Some(imm);
                        break;
                    }
                    if is_branch(prev) {
                        break;
                    }
                    back.push(prev);
                }
                if let Some(nr) = nr {
                    if let Some(name) = accelerator_name(nr) {
                        let nxt_off = o + 4;
                        let nxt = if nxt_off + 4 <= elf.data.len() {
                            u32le(&elf.data, nxt_off)
                        } else {
                            0
                        };
                        sites.push(Site {
                            vaddr,
                            nr,
                            name,
                            back,
                            next: nxt,
                        });
                    }
                }
            }
            vaddr += 4;
        }
    }
    sites
}

fn hex8(v: u32) -> String {
    format!("{v:#010x}")
}

pub(crate) fn scan_main(args: &[String]) -> ExitCode {
    if args.len() < 2 {
        die("usage: nr-tools scan-syscalls {report|patch} <elf> [out] [--version V] [--expect NAME=COUNT ...]");
    }
    let mode = args[0].as_str();
    let path = args[1].clone();
    let mut rest: Vec<&str> = args[2..].iter().map(|s| s.as_str()).collect();
    let out = if mode == "patch" {
        if rest.is_empty() {
            die("patch needs an output path");
        }
        let o = rest[0].to_string();
        rest.remove(0);
        Some(o)
    } else {
        None
    };
    let mut want_version: Option<String> = None;
    let mut expect: Vec<(String, i64)> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--version" => {
                i += 1;
                want_version = Some(rest.get(i).expect("--version needs a value").to_string());
            }
            "--expect" => {
                i += 1;
                let kv = rest.get(i).expect("--expect needs NAME=COUNT");
                let (n, c) = kv
                    .split_once('=')
                    .unwrap_or_else(|| die(format!("bad --expect {kv}")));
                expect.push((
                    n.to_string(),
                    c.parse()
                        .unwrap_or_else(|_| die(format!("bad count in {kv}"))),
                ));
            }
            other => die(format!("unknown argument {other}")),
        }
        i += 1;
    }

    let elf = Elf::open(&path);
    let ver = elf.glibc_version();
    println!(
        "{path}: glibc version from .gnu.version_d = {}",
        ver.clone().unwrap_or_else(|| "None".to_string())
    );
    let sites = scan(&elf);
    for s in &sites {
        let ignored = writes_x0_only(s.next);
        println!(
            "  {:#08x}  {}({})  gap={}  next={}  {}",
            s.vaddr,
            s.name,
            s.nr,
            s.back.len(),
            hex8(s.next),
            if ignored {
                "result ignored"
            } else if enosys_patchable(s.nr, s.next) {
                "result used; ENOSYS fallback"
            } else {
                "RESULT USED - do not nop"
            }
        );
        for b in s.back.iter().rev() {
            println!("          before {}", hex8(*b));
        }
    }

    if let Some(want) = &want_version {
        let got = ver.clone().unwrap_or_else(|| "None".to_string());
        if &got != want {
            println!(
                "REFUSING: this image says it is glibc {got}, this build expects {want} - \
                      the image shape (and therefore the sites) is not the one checked"
            );
            return ExitCode::from(1);
        }
    }
    let bad = sites
        .iter()
        .filter(|s| patchable(s.nr) && !writes_x0_only(s.next))
        .count();
    if bad > 0 {
        println!("REFUSING: {bad} patchable site(s) use the syscall's return value; a nop there would change behaviour");
        return ExitCode::from(1);
    }
    let bad_enosys = sites
        .iter()
        .filter(|s| s.nr == 435 && !enosys_patchable(s.nr, s.next))
        .count();
    if bad_enosys > 0 {
        println!("REFUSING: {bad_enosys} clone3 site(s) do not have the expected result check; refusing an unsafe ENOSYS rewrite");
        return ExitCode::from(1);
    }
    for (name, count) in &expect {
        let got = sites.iter().filter(|s| s.name == name.as_str()).count() as i64;
        if got != *count {
            println!(
                "REFUSING: {name}: found {got} site(s), expected {count} - \
                      this is not the glibc build this tool was checked against"
            );
            return ExitCode::from(1);
        }
    }
    if let Some(out) = out {
        let mut d = elf.data.clone();
        for s in &sites {
            let o = elf.vaddr_to_off(s.vaddr);
            if patchable(s.nr) {
                put_u32(&mut d, o, NOP);
            } else if enosys_patchable(s.nr, s.next) {
                // Replacing the syscall instruction with `movn x0, #37`
                // leaves x8=435 in place but makes the helper return -ENOSYS,
                // which is the documented signal that makes glibc fall back
                // to the legacy clone(2) path.
                put_u32(&mut d, o, MOVN_X0_37);
            }
        }
        fs::write(&out, &d).unwrap_or_else(|e| die(format!("{out}: {e}")));
        let n = sites
            .iter()
            .filter(|s| patchable(s.nr) || enosys_patchable(s.nr, s.next))
            .count();
        println!("patched {n} site(s) -> {out}");
    }
    ExitCode::SUCCESS
}

// ------------------------------------------------------------------ embed tool

const R_AARCH64_ABS64: u32 = 257;
const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;
const R_AARCH64_RELATIVE: u32 = 1027;

const STB_GLOBAL: u8 = 1;
const STB_WEAK: u8 = 2;
const STT_FUNC: u8 = 2;
const STT_GNU_IFUNC: u8 = 10;

const MAGIC: u64 = 0x4E52_484B_4F4F_4B31; // "NRHOOK1"
const DESC: usize = 48; // 6 x u64, see init-hook.S

// Checked against this loader build; a different glibc must be re-checked.
const EXPECT_LOADER_BUILD: &str = "glibc 2.39 aarch64";
const LD_START_BL: usize = 0x1AA74; // `bl _dl_init` inside _start
const LD_DL_INIT: u64 = 0x4BB0; // its target == _dl_init
const LD_TEXT_END: u64 = 0x41348; // end of the loader's last segment (vaddr)
const F_BASE: u64 = 0x40000; // where the shim lands inside the loader (64K-aligned)
/// Start of the shim's segment, and the reason it is not `F_BASE + SEG_ALIGN`:
///
/// the kernel maps every PT_LOAD from `ALIGN_DOWN(p_vaddr, page)` to
/// `ALIGN_UP(p_vaddr + p_memsz, page)`, using *the kernel's* page size. The
/// loader's last segment ends at 0x41348, so on a 4 KiB kernel it is mapped up
/// to 0x42000 and a shim at 0x42000 starts exactly where it stops. On a 16 KiB
/// kernel that same last segment is mapped up to 0x44000, and a shim at 0x42000
/// would be mapped from 0x40000 - i.e. MAP_FIXED over the last 16 KiB of the
/// loader's own .data/.got. The result is a SIGSEGV inside ld.so's startup
/// before a single syscall of ours runs (measured 2026-11-23: 4 KiB device fine,
/// 16 KiB kernel dead after execve with SEGV_ACCERR, no mprotect in strace).
///
/// So the shim must live at least one 64 KiB page above the loader's last
/// segment, while staying congruent with F_BASE modulo SEG_ALIGN.
const V_BASE: u64 = 0x90000;
/// Alignment declared for the shim's segment: the largest page size we support.
/// `(V_BASE - F_BASE) % SEG_ALIGN == 0` keeps p_offset == p_vaddr (mod SEG_ALIGN),
/// and `V_BASE >= ALIGN(LD_TEXT_END, SEG_ALIGN)` keeps the shim out of the
/// loader's own pages at every one of 4K/16K/64K.
const SEG_ALIGN: u64 = 0x10000;

/// Deliberately not interposed: our own entry points and compiler glue.
const NOT_INTERPOSED: [&str; 7] = [
    "nr_boot",
    "nr_exec_argv",
    "nr_resolve_name",
    "nr_get_main_map",
    "nr_set_main_map",
    "rust_eh_personality",
    "bcmp",
];

fn align(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

#[allow(dead_code)]
struct Sec {
    typ: u32,
    addr: u64,
    off: u64,
    size: u64,
}

fn sections(elf: &Elf) -> std::collections::HashMap<Vec<u8>, Sec> {
    let mut out = std::collections::HashMap::new();
    let shstrndx = u16le(&elf.data, 0x3E) as usize;
    let strtab_off = u64le(
        &elf.data,
        elf.e_shoff as usize + shstrndx * elf.e_shentsize as usize + 24,
    );
    for i in 0..elf.e_shnum as usize {
        let o = elf.e_shoff as usize + i * elf.e_shentsize as usize;
        let name_off = u32le(&elf.data, o) as usize;
        let (typ, addr, off_, size, _link, _) = elf.section(i);
        let name: Vec<u8> = if strtab_off != 0 {
            cstr(&elf.data, strtab_off as usize + name_off).to_vec()
        } else {
            Vec::new()
        };
        let _ = o;
        out.insert(
            name,
            Sec {
                typ,
                addr,
                off: off_,
                size,
            },
        );
    }
    out
}

struct EmbedSummary {
    shim_loads: Vec<u64>,
    shim_span: u64,
    hook_vaddr: u64,
    descriptor_vaddr: u64,
    rw: (u64, u64),
    externs: usize,
    interposed: usize,
    relative: usize,
    ext: usize,
    other: usize,
    self_reloc_slots: usize,
    total_load: u64,
    out_size: usize,
}

fn embed(shim_path: &str, loader_path: &str, hook_path: &str, out_path: &str) -> EmbedSummary {
    let shim = Elf::open(shim_path);
    let ld = Elf::open(loader_path);
    let mut hook = fs::read(hook_path).unwrap_or_else(|e| die(format!("{hook_path}: {e}")));
    let sec = sections(&shim);
    let dynsym = sec
        .get(b".dynsym" as &[u8])
        .unwrap_or_else(|| die("shim has no .dynsym"));
    let dynstr = sec
        .get(b".dynstr" as &[u8])
        .unwrap_or_else(|| die("shim has no .dynstr"));
    let nsyms = (dynsym.size / 24) as usize;

    // ------------------------------------------------------------- loader ---
    if ld.e_entry != 0x1AA40 {
        die(format!("unexpected loader entry {:#x}", ld.e_entry));
    }
    let l0 = ld
        .phdrs
        .iter()
        .find(|p| p.typ == PT_LOAD)
        .unwrap_or_else(|| die("loader has no PT_LOAD"));
    if !(l0.vaddr == 0 && l0.off == 0) {
        die("loader LOAD0 not at 0");
    }
    if u32le(&ld.data, LD_START_BL) & 0xFC00_0000 != 0x9400_0000 {
        die(format!("{LD_START_BL:#x} is not a `bl`"));
    }
    let bl = u32le(&ld.data, LD_START_BL);
    let mut imm26 = (bl & 0x03FF_FFFF) as i64;
    if imm26 & 0x0200_0000 != 0 {
        imm26 -= 1 << 26;
    }
    let bl_target = LD_START_BL as i64 + imm26 * 4;
    if bl_target != LD_DL_INIT as i64 {
        die(format!(
            "`bl` at {LD_START_BL:#x} targets {bl_target:#x}, expected _dl_init"
        ));
    }

    let notes: Vec<(usize, Phdr)> = ld
        .phdrs
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, p)| p.typ == PT_NOTE)
        .collect();
    if notes.len() != 1 {
        die("expected exactly one PT_NOTE to repurpose");
    }
    let (note_idx, note) = notes[0];
    if note.filesz > 0x40 {
        die("PT_NOTE is larger than expected");
    }

    // --------------------------------------------------------- shim block ---
    let mut span = 0u64;
    let mut loads = Vec::new();
    for p in &shim.phdrs {
        if p.typ != PT_LOAD {
            continue;
        }
        if p.off != p.vaddr {
            die("shim segments are not offset==vaddr");
        }
        if p.off % SEG_ALIGN != 0 || p.memsz < p.filesz {
            die("shim segment layout unexpected");
        }
        span = span.max(p.vaddr + p.memsz);
        loads.push(*p);
    }
    span = align(span, SEG_ALIGN);
    let mut image = vec![0u8; span as usize];
    for p in &loads {
        let s = p.off as usize;
        image[p.vaddr as usize..(p.vaddr + p.filesz) as usize]
            .copy_from_slice(&shim.data[s..s + p.filesz as usize]);
    }

    let rw = loads
        .iter()
        .find(|p| p.flags & 2 != 0)
        .unwrap_or_else(|| die("shim has no writable segment"));
    let rw_start = rw.vaddr;
    let rw_len = span - rw_start;

    // ----------------------------------------------------- relocation pass ---
    let mut extern_entries: Vec<(u64, u32, u64)> = Vec::new();
    let mut relative_slots: Vec<u64> = Vec::new();
    let (mut n_rel, mut n_ext, mut n_other) = (0usize, 0usize, 0usize);

    for relname in [b".rela.dyn" as &[u8], b".rela.plt" as &[u8]] {
        let Some(r) = sec.get(relname) else { continue };
        for i in 0..(r.size / 24) as usize {
            let o = r.off as usize + i * 24;
            let r_offset = u64le(&shim.data, o);
            let r_info = u64le(&shim.data, o + 8);
            let r_addend = i64le(&shim.data, o + 16);
            let r_sym = (r_info >> 32) as usize;
            let r_type = (r_info & 0xFFFF_FFFF) as u32;
            match r_type {
                0 => {}
                R_AARCH64_RELATIVE if r_sym == 0 => {
                    put_u64(
                        &mut image,
                        r_offset as usize,
                        V_BASE.wrapping_add(r_addend as u64),
                    );
                    relative_slots.push(r_offset);
                    n_rel += 1;
                }
                R_AARCH64_JUMP_SLOT | R_AARCH64_GLOB_DAT if r_sym != 0 => {
                    let s_name = u32le(&shim.data, dynsym.off as usize + r_sym * 24) as u64;
                    let name = cstr(&shim.data, dynstr.off as usize + s_name as usize);
                    extern_entries.push((
                        V_BASE + dynstr.addr + s_name,
                        name.len() as u32,
                        V_BASE + r_offset,
                    ));
                    n_ext += 1;
                }
                R_AARCH64_ABS64 => {
                    put_u64(
                        &mut image,
                        r_offset as usize,
                        V_BASE.wrapping_add(r_addend as u64),
                    );
                    relative_slots.push(r_offset);
                    n_other += 1;
                }
                other => die(format!(
                    "{}[{i}]: unsupported relocation {other}",
                    String::from_utf8_lossy(relname)
                )),
            }
        }
    }

    // ------------------------------------------------------ interpose table --
    let mut interpose_entries: Vec<(u64, u32, u64)> = Vec::new();
    for i in 1..nsyms {
        let o = dynsym.off as usize + i * 24;
        let st_name = u32le(&shim.data, o) as u64;
        let st_info = shim.data[o + 4];
        let st_shndx = u16le(&shim.data, o + 6);
        let st_value = u64le(&shim.data, o + 8);
        let (bind, typ) = (st_info >> 4, st_info & 0xF);
        if st_shndx == 0 || (bind != STB_GLOBAL && bind != STB_WEAK) {
            continue;
        }
        if typ != STT_FUNC && typ != STT_GNU_IFUNC {
            continue;
        }
        let name = cstr(&shim.data, dynstr.off as usize + st_name as usize);
        if NOT_INTERPOSED.iter().any(|n| n.as_bytes() == name) {
            continue;
        }
        interpose_entries.push((
            V_BASE + dynstr.addr + st_name,
            name.len() as u32,
            V_BASE + st_value,
        ));
    }
    if dynstr.addr + dynstr.size > span {
        die(".dynstr is outside the block");
    }

    // ---------------------------------------------------------- hook + data --
    let hook_off = span;
    // NB the hook is padded to a whole page, not to 8: the Python reference's align()
    // defaults to PAGE, and the table addresses are baked from data_off.
    let data_off = hook_off + align(hook.len() as u64, PAGE);
    let mut tables: Vec<u8> = Vec::new();
    let table = |entries: &[(u64, u32, u64)], tables: &mut Vec<u8>| -> u64 {
        let base = V_BASE + data_off + DESC as u64 + tables.len() as u64;
        let mut blob = Vec::with_capacity(8 + entries.len() * 24);
        blob.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (nv, nl, pl) in entries {
            blob.extend_from_slice(&nv.to_le_bytes());
            blob.extend_from_slice(&(*nl as u64).to_le_bytes());
            blob.extend_from_slice(&pl.to_le_bytes());
        }
        tables.extend_from_slice(&blob);
        base
    };
    let ext_addr = table(&extern_entries, &mut tables);
    let interp_addr = table(&interpose_entries, &mut tables);
    let reloc_entries: Vec<(u64, u32, u64)> = relative_slots
        .iter()
        .map(|off| (0u64, 0u32, V_BASE + off))
        .collect();
    let reloc_addr = table(&reloc_entries, &mut tables);

    let nr_boot = shim_symbol("nr_boot", &shim, dynsym, dynstr);
    let mut data = Vec::with_capacity(DESC + tables.len());
    for v in [
        ext_addr,
        interp_addr,
        V_BASE + nr_boot,
        V_BASE + rw_start,
        rw_len,
        reloc_addr,
    ] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.extend_from_slice(&tables);
    let total = align(data_off + data.len() as u64, PAGE);

    // patch the hook's magic header: {magic, hook vaddr, data vaddr, _dl_init}
    let magic = MAGIC.to_le_bytes();
    let m = hook
        .windows(8)
        .position(|w| w == magic)
        .unwrap_or_else(|| die("hook has no NRHOOK1 header"));
    put_u64(&mut hook, m + 8, V_BASE + hook_off);
    put_u64(&mut hook, m + 16, V_BASE + data_off);
    put_u64(&mut hook, m + 24, LD_DL_INIT);

    // --------------------------------------------------------- write it out --
    let mut out = ld.data.clone();
    if (out.len() as u64) < F_BASE {
        out.resize(F_BASE as usize, 0);
    }
    out.resize((F_BASE + total) as usize, 0);
    if !out[F_BASE as usize..].iter().all(|b| *b == 0) {
        die("the space we are claiming is not empty");
    }
    let fb = F_BASE as usize;
    out[fb..fb + span as usize].copy_from_slice(&image);
    out[fb + hook_off as usize..fb + hook_off as usize + hook.len()].copy_from_slice(&hook);
    out[fb + data_off as usize..fb + data_off as usize + data.len()].copy_from_slice(&data);

    // PT_NOTE -> PT_LOAD(R|E)
    let o = ld.e_phoff as usize + note_idx * 56;
    put_u32(&mut out, o, PT_LOAD);
    put_u32(&mut out, o + 4, 5);
    put_u64(&mut out, o + 8, F_BASE);
    put_u64(&mut out, o + 16, V_BASE);
    put_u64(&mut out, o + 24, V_BASE);
    put_u64(&mut out, o + 32, total);
    put_u64(&mut out, o + 40, total);
    // 64 KiB, not the page size: this segment is only ever exec'd by the kernel
    // (nothing dlopens the loader), where congruence is what counts, but glibc
    // would refuse a p_align that is not a multiple of the running page size -
    // and a static check that says "16K: FAIL" is a reading every future
    // auditor has to re-derive. F_BASE and V_BASE differ by exactly SEG_ALIGN,
    // so the pair stays congruent at 4K, 16K and 64K.
    put_u64(&mut out, o + 48, SEG_ALIGN);

    // `bl _dl_init` -> `bl nr_dl_init_hook`
    let target = V_BASE + hook_off;
    let delta = (target as i64 - LD_START_BL as i64) / 4;
    if delta < -(1 << 25) || delta >= (1 << 25) {
        die("hook out of branch range");
    }
    put_u32(
        &mut out,
        LD_START_BL,
        0x9400_0000 | (delta as u32 & 0x03FF_FFFF),
    );

    // ---------------------------------------------------------- bss padding ---
    // The kernel zeroes BSS exactly once, at the *largest* PT_LOAD end
    // (`elf_bss`). Our shim segment is now the largest one, so the loader's own
    // BSS would keep the file bytes that back its last page - and those bytes
    // are whatever the tooling left there (section headers, in practice).
    // That is not cosmetic: ld.so's statics live in that page, so a non-zero BSS
    // kills the loader before `_dl_init` is ever reached (measured on RMX3357:
    // SIGBUS inside dl_main while "generating link map"). Fix: zero the tail of
    // the RW segment's last file-backed page.
    // NB this is the *loader's* writable segment, not the shim's: the shim's BSS
    // tail is inside its own image, the loader's is inside the loader's file page.
    let ld_rw = ld
        .phdrs
        .iter()
        .find(|p| p.typ == PT_LOAD && p.flags & 2 != 0)
        .unwrap_or_else(|| die("loader has no writable PT_LOAD"));
    let rw_file_end = ld_rw.off + ld_rw.filesz;
    let rw_page_end = align(rw_file_end, PAGE).min(F_BASE);
    if rw_page_end > rw_file_end {
        let e_shoff = u64le(&ld.data, 0x28);
        let e_shentsize = u16le(&ld.data, 0x3a) as u64;
        let e_shnum = u16le(&ld.data, 0x3c) as u64;
        let sh_bytes = e_shnum * e_shentsize;
        if sh_bytes != 0 && e_shoff < rw_page_end && e_shoff + sh_bytes > rw_file_end {
            // Move the section header table past the payload first, so zeroing
            // the BSS tail does not destroy it.
            let start = e_shoff as usize;
            let table = out[start..start + sh_bytes as usize].to_vec();
            let new_off = (out.len() + 7) & !7;
            out.resize(new_off + table.len(), 0);
            out[new_off..].copy_from_slice(&table);
            put_u64(&mut out, 0x28, new_off as u64);
            eprintln!("bss pad: section headers moved {} -> {}", e_shoff, new_off);
        }
        for b in out[rw_file_end as usize..rw_page_end as usize].iter_mut() {
            *b = 0;
        }
        eprintln!(
            "bss pad: zeroed file [{}..{}) to keep ld.so's BSS zero",
            rw_file_end, rw_page_end
        );
    }

    fs::write(out_path, &out).unwrap_or_else(|e| die(format!("{out_path}: {e}")));

    EmbedSummary {
        shim_loads: loads.iter().map(|p| p.vaddr).collect(),
        shim_span: span,
        hook_vaddr: target,
        descriptor_vaddr: V_BASE + data_off,
        rw: (rw_start, rw_len),
        externs: n_ext,
        interposed: interpose_entries.len(),
        relative: n_rel,
        ext: n_ext,
        other: n_other,
        self_reloc_slots: relative_slots.len(),
        total_load: total,
        out_size: out.len(),
    }
}

fn shim_symbol(name: &str, shim: &Elf, dynsym: &Sec, dynstr: &Sec) -> u64 {
    let nsyms = (dynsym.size / 24) as usize;
    for i in 0..nsyms {
        let o = dynsym.off as usize + i * 24;
        let st_name = u32le(&shim.data, o) as usize;
        if cstr(&shim.data, dynstr.off as usize + st_name) == name.as_bytes() {
            return u64le(&shim.data, o + 8);
        }
    }
    die(format!("shim does not export {name}"))
}

fn print_summary(s: &EmbedSummary) {
    // Same shape as the Python tool's json.dumps(indent=2).
    let mut o = String::from("{\n");
    o += &format!("  \"loader\": \"{EXPECT_LOADER_BUILD}\",\n");
    o += "  \"shim_loads\": [\n";
    for (i, v) in s.shim_loads.iter().enumerate() {
        o += &format!(
            "    {v}{}\n",
            if i + 1 == s.shim_loads.len() { "" } else { "," }
        );
    }
    o += "  ],\n";
    o += &format!("  \"shim_span\": {},\n", s.shim_span);
    o += &format!("  \"hook_vaddr\": {},\n", s.hook_vaddr);
    o += &format!("  \"descriptor_vaddr\": {},\n", s.descriptor_vaddr);
    o += &format!("  \"rw\": [\n    {},\n    {}\n  ],\n", s.rw.0, s.rw.1);
    o += &format!("  \"externs\": {},\n", s.externs);
    o += &format!("  \"interposed\": {},\n", s.interposed);
    o += &format!("  \"relocations\": {{\n    \"relative\": {},\n    \"ext\": {},\n    \"other\": {}\n  }},\n",
                  s.relative, s.ext, s.other);
    o += &format!("  \"self_reloc_slots\": {},\n", s.self_reloc_slots);
    o += &format!("  \"file_offset\": {},\n", F_BASE);
    o += &format!("  \"vaddr\": {},\n", V_BASE);
    o += &format!("  \"total_load\": {},\n", s.total_load);
    o += &format!("  \"out_size\": {}\n}}", s.out_size);
    println!("{o}");
}

/// `nr-tools check-elf <elf|dir>... [--page N]...`
///
/// The only thing that decides whether an ELF runs on a 4K/16K/64K kernel is
/// the PT_LOAD congruence `p_offset == p_vaddr (mod PAGE_SIZE)` (plus p_align as
/// a lower bound). A device label is not evidence. This walks one file or a
/// whole rootfs and prints the verdict per page size, so "can this run on a 16K
/// device" is answered by looking, not by guessing.
fn check_elf_main(args: &[String]) {
    if args.is_empty() {
        die("usage: nr-tools check-elf <elf|dir>... [--page N]...");
    }
    let mut roots: Vec<String> = Vec::new();
    let mut pages: Vec<u64> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--page" {
            i += 1;
            let a = args.get(i).unwrap_or_else(|| die("--page needs a value"));
            pages.push(parse_page(a));
        } else {
            roots.push(args[i].clone());
        }
        i += 1;
    }
    if pages.is_empty() {
        pages = vec![4096, 16384, 65536];
    }

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for r in &roots {
        walk_elf_candidates(std::path::Path::new(r), &mut files);
    }
    files.sort();
    if files.is_empty() {
        die("no ELF files found");
    }

    let mut bad = vec![0usize; pages.len()];
    let mut checked = 0usize;
    for f in &files {
        let Ok(data) = fs::read(f) else { continue };
        if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 {
            continue;
        }
        let e_phoff = u64le(&data, 0x20) as usize;
        let e_phentsize = u16le(&data, 0x36) as usize;
        let e_phnum = u16le(&data, 0x38) as usize;
        if e_phentsize < 56 {
            continue;
        }
        let mut max_align = 0u64;
        let mut loads = 0usize;
        let mut verdict = vec![true; pages.len()];
        for k in 0..e_phnum {
            let o = e_phoff + k * e_phentsize;
            if o + 56 > data.len() {
                break;
            }
            if u32le(&data, o) != PT_LOAD {
                continue;
            }
            loads += 1;
            let off = u64le(&data, o + 8);
            let vaddr = u64le(&data, o + 16);
            let align = u64le(&data, o + 48);
            max_align = max_align.max(align);
            for (pi, page) in pages.iter().enumerate() {
                // Two separate rules, both enforced at load time:
                //   * p_vaddr and p_offset congruent modulo the page size, or
                //     the kernel cannot place the segment;
                //   * p_align a multiple of the page size, or glibc's dl-load
                //     refuses the object ("ELF load command alignment not
                //     page-aligned"). A 4 KiB-aligned segment in a libc is
                //     exactly the second case: congruent, still unloadable on a
                //     16 KiB kernel.
                if off % page != vaddr % page || (align != 0 && align % page != 0) {
                    verdict[pi] = false;
                }
            }
        }
        if loads == 0 {
            continue;
        }
        checked += 1;
        let name = f.display();
        let mut line = format!("{name}  p_align=0x{max_align:x}  loads={loads} ");
        for (pi, page) in pages.iter().enumerate() {
            if verdict[pi] {
                line.push_str(&format!(" {page}:ok"));
            } else {
                line.push_str(&format!(" {page}:FAIL"));
                bad[pi] += 1;
            }
        }
        println!("{line}");
    }

    println!();
    println!("checked {checked} ELF file(s)");
    for (pi, page) in pages.iter().enumerate() {
        if bad[pi] == 0 {
            println!("  {page:>6}: all congruent");
        } else {
            println!("  {page:>6}: {} file(s) would fail to exec", bad[pi]);
        }
    }
    if bad.iter().any(|n| *n > 0) {
        std::process::exit(1);
    }
}

/// Fail the build when the produced ELF could not be exec'd on a kernel with
/// `page`-sized pages. Used by `embed` for the 16 KiB case.
fn assert_page_congruent(path: &str, page: u64) {
    let data = fs::read(path).unwrap_or_else(|e| die(format!("{path}: {e}")));
    if data.len() < 64 || &data[..4] != b"\x7fELF" {
        die(format!("{path}: not an ELF"));
    }
    let e_phoff = u64le(&data, 0x20) as usize;
    let e_phentsize = u16le(&data, 0x36) as usize;
    let e_phnum = u16le(&data, 0x38) as usize;
    let mut i = 0;
    while i < e_phnum {
        let o = e_phoff + i * e_phentsize;
        i += 1;
        if o + 56 > data.len() {
            break;
        }
        if u32le(&data, o) != PT_LOAD {
            continue;
        }
        let off = u64le(&data, o + 8);
        let vaddr = u64le(&data, o + 16);
        let align = u64le(&data, o + 48);
        if off % page != vaddr % page {
            die(format!(
                "{path}: PT_LOAD off={off:#x} vaddr={vaddr:#x} is not congruent modulo {page:#x} \
                 -- this file cannot be exec'd on a {}-KiB page kernel",
                page / 1024
            ));
        }
        if align != 0 && align < page {
            eprintln!("{path}: warning: PT_LOAD p_align={align:#x} < {page:#x} (allowed, congruence is what matters)");
        }
    }
}

fn parse_page(a: &str) -> u64 {
    let v = if let Some(h) = a.strip_prefix("0x") {
        u64::from_str_radix(h, 16).unwrap_or_else(|_| die(format!("bad page size {a}")))
    } else {
        match a {
            "4k" | "4K" => 4096,
            "16k" | "16K" => 16384,
            "64k" | "64K" => 65536,
            _ => a
                .parse::<u64>()
                .unwrap_or_else(|_| die(format!("bad page size {a}"))),
        }
    };
    if v < 4096 || v & (v - 1) != 0 {
        die(format!("page size {a} is not a power of two >= 4096"));
    }
    v
}

fn walk_elf_candidates(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(md) = fs::symlink_metadata(p) else {
        return;
    };
    if md.file_type().is_symlink() {
        // Symlinks in a rootfs are absolute guest paths; the target resolves
        // inside the same tree, so skip and let the real file be visited.
        return;
    }
    if md.is_dir() {
        let Ok(rd) = fs::read_dir(p) else { return };
        for e in rd.flatten() {
            walk_elf_candidates(&e.path(), out);
        }
        return;
    }
    if !md.is_file() {
        return;
    }
    // Backups are not part of the image and must not decide the verdict:
    // libc.so.6.orig (pristine, for re-patching), .pre-<tag> (before a patch),
    // .bak-* and friends all live next to the real file.
    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        let backup = [".orig", ".pre-mq", ".bak", ".new", ".old"]
            .iter()
            .any(|sfx| name.ends_with(sfx))
            || name.contains(".pre-")
            || name.contains(".bak-");
        if backup {
            return;
        }
    }
    // Cheap pre-filter: ELF magic in the first four bytes.
    if let Ok(mut f) = fs::File::open(p) {
        use std::io::Read;
        let mut m = [0u8; 4];
        if f.read_exact(&mut m).is_ok() && &m == b"\x7fELF" {
            out.push(p.to_path_buf());
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprintln!("usage: nr-tools {{scan-syscalls|check-elf|embed|libc-interpose|provision|rootfs|rootfs-status}} ...");
        return ExitCode::from(2);
    };
    match cmd.as_str() {
        "scan-syscalls" => scan_main(&args[1..]),
        "check-elf" => {
            check_elf_main(&args[1..]);
            ExitCode::SUCCESS
        }
        "provision" => {
            provision::main(&args[1..]);
            ExitCode::SUCCESS
        }
        "rootfs" => {
            rootfs::main(&args[1..]);
            ExitCode::SUCCESS
        }
        "rootfs-status" => {
            status::main(&args[1..]);
            ExitCode::SUCCESS
        }
        "libc-interpose" => {
            libc_slots::main(&args[1..]);
            ExitCode::SUCCESS
        }
        "embed" => {
            if args.len() < 5 {
                die("usage: nr-tools embed <shim.so> <loader.orig> <hook.bin> <out.so>");
            }
            // Overlap is decided with the kernel's page size, not ours: on a
            // 16K/64K kernel the loader's last segment is mapped past its
            // p_memsz, so the comparison must use the largest page we support.
            if V_BASE < align(LD_TEXT_END, SEG_ALIGN) + (F_BASE % SEG_ALIGN) {
                die("shim would share a page with the loader's own segments");
            }
            if F_BASE % SEG_ALIGN != V_BASE % SEG_ALIGN {
                die("offset/vaddr must be congruent modulo the largest page size");
            }
            let s = embed(&args[1], &args[2], &args[3], &args[4]);
            // The output must be loadable on a 16 KiB-page kernel, not just on
            // the 4 KiB page of the build machine: every PT_LOAD has to satisfy
            // p_offset == p_vaddr (mod 0x4000). Checked on the file that was
            // actually written, not on the in-memory copy.
            assert_page_congruent(&args[4], 0x4000);
            assert_page_congruent(&args[4], 0x10000);
            print_summary(&s);
            ExitCode::SUCCESS
        }
        other => die(format!("unknown command {other}")),
    }
}
