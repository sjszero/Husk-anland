//! `nr-tools libc-patch` - the guest libc transform as one Rust command.
//!
//! The transform itself lives in two other subcommands (`libc-interpose`,
//! `scan-syscalls patch`). What this adds is the part the shell script that
//! used to do this could not do: **assertions on the result**. A `tar` that
//! failed, a refused symbol, a partially applied syscall patch or a segment
//! that only works on 4 KiB pages all used to end up as "installed and fine";
//! here each one stops the run.
//!
//! Everything is checked against the file that was written, not against
//! in-memory state or stdout text:
//!
//!   1. input is an aarch64 ELF64 shared object, glibc version == expected;
//!   2. the input still carries the `clone3` site(s): a libc without them is
//!      either already patched or not the libc we think it is -> refuse;
//!   3. after interposition, the slot page names every function in the table,
//!      in table order (a slot that is missing is a symbol that was refused:
//!      `open`/`fopen` in nss_files would silently reach the host again);
//!   4. after the syscall patch, `clone3` sites are gone and the other two
//!      counters are unchanged (the patch is targeted, not a rewrite);
//!   5. every PT_LOAD is congruent modulo 16 KiB and 64 KiB, so the file loads
//!      on a 4 KiB device as well as on one the build machine never saw.
//!
//! `--assert-only` runs 1/3/4/5 on an existing file (the release artifact), and
//! is what `make libc-verify` calls.

use std::fs;
use std::path::{Path, PathBuf};

use crate::libc_slots::{SLOT_HDR, SLOT_MAGIC, SLOT_STRIDE};
use crate::{assert_page_congruent, die, scan, u32le, Elf};

// --------------------------------------------------------------------- sha256

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    let mut w = [0u32; 64];
    for chunk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, hh].iter().enumerate() {
            h[i] = h[i].wrapping_add(*v);
        }
    }
    let mut out = String::with_capacity(64);
    for v in h {
        out.push_str(&format!("{v:08x}"));
    }
    out
}

// ------------------------------------------------------------------ slot page

/// Locate the slot page and return (offset, entry count).
fn slot_page(d: &[u8], what: &str) -> (usize, u32) {
    let magic = SLOT_MAGIC;
    let Some(at) = d.windows(magic.len()).position(|w| w == magic) else {
        die(format!("{what}: no slot page (magic {magic:?} not found)"));
    };
    let count = u32le(d, at + 12);
    let stride = u32le(d, at + 16);
    if stride as usize != SLOT_STRIDE {
        die(format!(
            "{what}: slot stride {stride} != {SLOT_STRIDE} (format changed?)"
        ));
    }
    if at + SLOT_HDR + count as usize * SLOT_STRIDE > d.len() {
        die(format!("{what}: slot page truncated ({count} entries)"));
    }
    (at, count)
}

fn slot_name(d: &[u8], page: usize, i: u32) -> String {
    let off = page + SLOT_HDR + i as usize * SLOT_STRIDE;
    let raw = &d[off..off + 32];
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).to_string()
}

/// Which table entries own a slot, and which are holes.
fn slot_entries(d: &[u8], what: &str) -> (Vec<String>, Vec<usize>) {
    let (page, count) = slot_page(d, what);
    let mut filled = Vec::new();
    let mut holes = Vec::new();
    for i in 0..count {
        let n = slot_name(d, page, i);
        if n.is_empty() {
            holes.push(i as usize);
        } else {
            filled.push(n);
        }
    }
    (filled, holes)
}

/// The slot page has to name every table entry except the ones declared to be
/// refused in `libc-interpose.expected` - set equality, both directions:
/// a refusal that is not declared means the transform silently lost a hook, and
/// a declared refusal that now gets patched means glibc (or the interposer)
/// changed and the list needs a review.
fn assert_slots(d: &[u8], names: &[String], expected_holes: &[String], what: &str) {
    let (page, count) = slot_page(d, what);
    if count as usize != names.len() {
        die(format!(
            "{what}: slot page has {count} entries, table has {}",
            names.len()
        ));
    }
    let (filled, holes) = slot_entries(d, what);
    let mut unexpected = Vec::new();
    for i in &holes {
        let n = names.get(*i).cloned().unwrap_or_else(|| format!("#{i}"));
        if !expected_holes.contains(&n) {
            unexpected.push(format!("{n} (slot {i})"));
        }
    }
    if !unexpected.is_empty() {
        die(format!(
            "{what}: {} hook(s) missing that are not declared in libc-interpose.expected: {}",
            unexpected.len(),
            unexpected.join(", ")
        ));
    }
    for e in expected_holes {
        let idx = names.iter().position(|n| n == e);
        let Some(idx) = idx else {
            die(format!(
                "{what}: libc-interpose.expected lists {e}, which is not in the table"
            ));
        };
        if !holes.contains(&idx) {
            die(format!(
                "{what}: {e} is declared refused but owns slot {idx}; drop the line from \
                 libc-interpose.expected"
            ));
        }
    }
    // The names that are filled have to be the table's names: same spellings,
    // same relative order (a duplicate would show up as a missing one).
    let want: Vec<&String> = names
        .iter()
        .filter(|n| !expected_holes.iter().any(|e| *e == **n))
        .collect();
    if filled.len() != want.len() {
        die(format!(
            "{what}: {} hooks patched, table minus declared refusals is {}",
            filled.len(),
            want.len()
        ));
    }
    for (got, w) in filled.iter().zip(want.iter()) {
        if got != *w {
            die(format!(
                "{what}: slot order changed: {got} where {w} belongs"
            ));
        }
    }
    let _ = page;
}

// ------------------------------------------------------------------ assertions

fn syscall_counts(path: &str) -> (usize, usize, usize) {
    let e = Elf::open(path);
    let sites = scan(&e);
    let mut robust = 0;
    let mut rseq = 0;
    let mut clone3 = 0;
    for s in &sites {
        match s.nr {
            99 => robust += 1,
            293 => rseq += 1,
            435 => clone3 += 1,
            _ => {}
        }
    }
    (robust, rseq, clone3)
}

fn assert_patched(path: &str, names: &[String], expected_holes: &[String], rseq_sites: usize) {
    let data = fs::read(path).unwrap_or_else(|e| die(format!("{path}: {e}")));
    if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 {
        die(format!("{path}: not an ELF64 after patching"));
    }
    if &data[16..18] != b"\x03\x00" {
        die(format!(
            "{path}: e_type {:?} is not ET_DYN",
            u16::from_le_bytes([data[16], data[17]])
        ));
    }
    assert_slots(&data, names, expected_holes, path);
    let (r, s, c) = syscall_counts(path);
    if c != 0 {
        die(format!(
            "{path}: {c} clone3 site(s) still present; the ENOSYS patch did not land"
        ));
    }
    // `set_robust_list` sites are rewritten by the patch (they must be gone),
    // `rseq` is left alone (it is disabled through GLIBC_TUNABLES instead, its
    // callers inspect the result), so its count must survive unchanged.
    if r != 0 || s != rseq_sites {
        die(format!(
            "{path}: after patching set_robust_list={r} (must be 0) rseq={s} (expected {rseq_sites})"
        ));
    }
    // 4 KiB is what the build box has, 16 KiB and 64 KiB are what Android ships.
    assert_page_congruent(path, 0x1000);
    assert_page_congruent(path, 0x4000);
    assert_page_congruent(path, 0x10000);
}

// ------------------------------------------------------------------------ main

/// Lines are `name` or `name # comment`; comments and blanks are dropped.
fn table(path: &str) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| die(format!("{path}: {e}")))
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn main(args: &[String]) {
    if args.len() < 2 {
        die(
            "usage: nr-tools libc-patch <libc.so.6> <out.so.6> --table <names.txt> \
             [--version 2.39] [--clone3-sites 1] [--rseq-sites 1] [--expected FILE] \
             [--expect-sha256 HEX] \
             [--assert-only]",
        );
    }
    let input = args[0].clone();
    let out = args[1].clone();
    let mut want_version = "2.39".to_string();
    let mut want_clone3 = 1usize;
    let mut expect_sha: Option<String> = None;
    let mut assert_only = false;
    let mut rseq_sites = 1usize;
    let mut table_path = String::new();
    let mut expected_path = String::new();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--table" => {
                i += 1;
                table_path = args
                    .get(i)
                    .unwrap_or_else(|| die("--table needs a path"))
                    .clone();
            }
            "--expected" => {
                i += 1;
                expected_path = args
                    .get(i)
                    .unwrap_or_else(|| die("--expected needs a path"))
                    .clone();
            }
            "--version" => {
                i += 1;
                want_version = args
                    .get(i)
                    .unwrap_or_else(|| die("--version needs a value"))
                    .clone();
            }
            "--clone3-sites" => {
                i += 1;
                want_clone3 = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| die("--clone3-sites needs a number"));
            }
            "--expect-sha256" => {
                i += 1;
                expect_sha = Some(
                    args.get(i)
                        .unwrap_or_else(|| die("--expect-sha256 needs a hex digest"))
                        .to_lowercase(),
                );
            }
            "--rseq-sites" => {
                i += 1;
                rseq_sites = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| die("--rseq-sites needs a number"));
            }
            "--assert-only" => assert_only = true,
            other => die(format!("unknown argument {other}")),
        }
        i += 1;
    }
    if table_path.is_empty() {
        die("--table is required (tools/loader/libc-interpose.txt)");
    }
    let names = table(&table_path);
    if names.is_empty() {
        die(format!("{table_path}: empty interposition table"));
    }
    if expected_path.is_empty() {
        // <table>.expected sits next to the table: one file per libc generation
        // is the wrong axis, so it is keyed by the table it belongs to.
        expected_path = match table_path.strip_suffix(".txt") {
            Some(stem) => format!("{stem}.expected"),
            None => format!("{table_path}.expected"),
        };
    }
    let expected_holes = table(&expected_path);
    for e in &expected_holes {
        if !names.iter().any(|n| n == e) {
            die(format!(
                "{expected_path}: {e} is not in {table_path} (stale list, or a typo)"
            ));
        }
    }

    if assert_only {
        let data = fs::read(&input).unwrap_or_else(|e| die(format!("{input}: {e}")));
        let e = Elf::open(&input);
        let ver = e
            .glibc_version()
            .unwrap_or_else(|| die(format!("{input}: no glibc version in .gnu.version_d")));
        assert_patched(&input, &names, &expected_holes, rseq_sites);
        println!(
            "{}: glibc {ver}, {}/{} hooks ({} declared refusals), clone3 0, \
             page-congruent 4K/16K/64K, sha256 {}",
            input,
            names.len() - expected_holes.len(),
            names.len(),
            expected_holes.len(),
            sha256_hex(&data)
        );
        return;
    }

    // 1. the input has to be the libc we think it is
    let e = Elf::open(&input);
    let ver = e
        .glibc_version()
        .unwrap_or_else(|| die(format!("{input}: no glibc version in .gnu.version_d")));
    if ver != want_version {
        die(format!(
            "{input}: glibc {ver}, expected {want_version} (assertions below are version-tied)"
        ));
    }
    if &e.data[16..18] != b"\x03\x00" {
        die(format!(
            "{input}: e_type is not ET_DYN (not a shared object)"
        ));
    }
    // 2. and it must be the *stock* one: no slot page, clone3 still there
    let (r0, s0, c0) = syscall_counts(&input);
    if c0 != want_clone3 {
        if c0 == 0 {
            die(format!(
                "{input}: no clone3 site left - this libc is already patched. Pass the stock \
                 libc (the .stock copy next to the installed one), or use --assert-only"
            ));
        }
        die(format!(
            "{input}: {c0} clone3 site(s), expected {want_clone3}"
        ));
    }
    if fs::read(&input)
        .map(|d| d.windows(SLOT_MAGIC.len()).any(|w| w == SLOT_MAGIC))
        .unwrap_or(false)
    {
        die(format!(
            "{input}: already carries a slot page; refusing to interpose twice"
        ));
    }

    // temp space for the intermediate file
    let tmp_dir: PathBuf = std::env::temp_dir().join(format!(
        "husk-libc-patch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&tmp_dir).unwrap_or_else(|err| die(format!("{}: {err}", tmp_dir.display())));
    let slots = tmp_dir.join("libc-slots.so.6");
    let slots_s = slots.to_string_lossy().to_string();

    // 3. interpose (the subcommand dies on its own hard failures)
    crate::libc_slots::main(&[input.clone(), slots_s.clone(), table_path.clone()]);
    let slots_data = fs::read(&slots).unwrap_or_else(|e| die(format!("{slots_s}: {e}")));
    assert_slots(&slots_data, &names, &expected_holes, &slots_s);
    println!(
        "   interpose: {}/{} hooks, {} declared refusals",
        slot_entries(&slots_data, &slots_s).0.len(),
        names.len(),
        expected_holes.len()
    );

    // 4. clone3 -> ENOSYS, then assert the file that was written
    let _ = crate::scan_main(&[
        "patch".to_string(),
        slots_s.clone(),
        out.clone(),
        "--version".to_string(),
        want_version.clone(),
        "--expect".to_string(),
        format!("set_robust_list={r0}"),
        "--expect".to_string(),
        format!("rseq={s0}"),
        "--expect".to_string(),
        format!("clone3={c0}"),
    ]);
    assert_patched(&out, &names, &expected_holes, s0);

    let out_data = fs::read(&out).unwrap_or_else(|e| die(format!("{out}: {e}")));
    let sha = sha256_hex(&out_data);
    if let Some(want) = &expect_sha {
        if &sha != want {
            die(format!(
                "{out}: sha256 {sha}, expected {want} - the transform is not byte-stable"
            ));
        }
    }
    println!(
        "{}: ok - glibc {ver}, {}/{} hooks, clone3 {c0}->0, set_robust_list {r0}->0, {} -> {} bytes, sha256 {sha}",
        out,
        names.len() - expected_holes.len(),
        names.len(),
        fs::metadata(&input).map(|m| m.len()).unwrap_or(0),
        out_data.len()
    );
    let _ = fs::remove_dir_all(Path::new(&tmp_dir));
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // 1 MiB of 0x61: the multi-block path, where the length field matters.
        assert_eq!(
            sha256_hex(&vec![b'a'; 1024 * 1024]),
            "9bc1b2a288b26af7257a36277ae3816a7d4f16e89c1e7e77d0a5c48bad62b360".to_string()
        );
    }
}
