//! husk-ldconfig — write a glibc `ld.so.cache` that describes *this* guest root.
//!
//! Ubuntu's own `ldconfig` is a static-pie binary: it cannot be interposed, so
//! inside Husk it resolves `/lib`, `/etc` and `/usr` against the *host*
//! namespace. Its only effects are noise (`Can't stat /lib`) and an exit code of
//! 1, and `dpkg --configure libc-bin` treats that as a real failure - which
//! stops apt from touching anything at all.
//!
//! So the guest gets its own cache writer. It is a plain guest binary: it runs
//! under the runtime loader like everything else, so the paths it reads and
//! writes are namespace paths. It writes the documented `glibc-ld.so.cache`
//! "1.1" format (`elf/dl-cache.h` in glibc 2.39), and the EGLIBC-era reader in
//! the original static `ldconfig.real` is kept next to it as `ldconfig.real.glbc`
//! so the cache can be dumped by an independent parser (`ldconfig.real.glbc -p`).
//!
//! usage: husk-ldconfig [-p] [-C cache] [dirs...]
//!   (no args)  rescan the standard directories and rewrite the cache
//!   -p         print the existing cache
//!   -N         ignored (accepted so a `-N` from a postinst is not an error)

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

/// FLAG_ELF_LIBC6 | FLAG_AARCH64_LIB64 (sysdeps/generic/ldconfig.h).
const FLAGS: u32 = 0x0003 | 0x0a00;
const CACHE_NEW_MAGIC: &[u8; 17] = b"glibc-ld.so.cache";
const CACHE_NEW_VERSION: &[u8; 3] = b"1.1";
const ENDIAN_LITTLE: u8 = 2;

const DEFAULT_DIRS: [&str; 4] = [
    "/lib/aarch64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/usr/local/lib",
    "/usr/lib",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut print = false;
    let mut cache_path = String::from("/etc/ld.so.cache");
    let mut dirs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--print-cache" => print = true,
            "-N" => {}
            "-v" | "--verbose" => {}
            "-C" => {
                i += 1;
                match args.get(i) {
                    Some(v) => cache_path = v.clone(),
                    None => die("husk-ldconfig: -C needs an argument"),
                }
            }
            "-f" | "-r" => {
                // -f <conf>: we read /etc/ld.so.conf.d ourselves; -r <root> is a
                // chroot we cannot perform without CAP_SYS_CHROOT, so both are
                // accepted and ignored with a note on stderr.
                i += 1;
                eprintln!("husk-ldconfig: note: '{}' is not applied inside the runtime", args[i - 1]);
            }
            other if other.starts_with('-') => {
                eprintln!("husk-ldconfig: unknown option {other}, ignoring");
            }
            other => dirs.push(other.to_string()),
        }
        i += 1;
    }

    if print {
        print_cache(&cache_path);
        return;
    }

    if dirs.is_empty() {
        dirs = default_dirs();
    }

    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for dir in &dirs {
        scan_dir(Path::new(dir), &mut found);
    }

    if found.is_empty() {
        eprintln!("husk-ldconfig: no libraries found in {dirs:?}");
    }

    match write_cache(&cache_path, &found) {
        Ok(()) => println!(
            "husk-ldconfig: wrote {cache_path} ({} entries from {} directories)",
            found.len(),
            dirs.len()
        ),
        Err(e) => {
            eprintln!("husk-ldconfig: cannot write {cache_path}: {e}");
            std::process::exit(1);
        }
    }
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

/// `/etc/ld.so.conf` plus `/etc/ld.so.conf.d/*.conf`, or the built-in list.
fn default_dirs() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: String| {
        let p = p.trim().to_string();
        if !p.is_empty() && !p.starts_with('#') && !out.contains(&p) {
            out.push(p);
        }
    };
    if let Ok(text) = fs::read_to_string("/etc/ld.so.conf") {
        for line in text.lines() {
            if line.trim().starts_with("include") {
                continue; // the .conf.d sweep below covers it
            }
            push(line.to_string());
        }
    }
    let mut confs: Vec<PathBuf> = fs::read_dir("/etc/ld.so.conf.d")
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension() == Some(OsStr::new("conf")))
                .collect()
        })
        .unwrap_or_default();
    confs.sort();
    for c in confs {
        if let Ok(text) = fs::read_to_string(&c) {
            for line in text.lines() {
                if !line.trim().starts_with("include") {
                    push(line.to_string());
                }
            }
        }
    }
    for d in DEFAULT_DIRS {
        push(d.to_string());
    }
    out
}

fn scan_dir(dir: &Path, out: &mut BTreeMap<String, String>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let Ok(md) = fs::metadata(&path) else { continue }; // follows symlinks
        let ft = md.file_type();
        if ft.is_dir() {
            // ldconfig scans exactly the directories its configuration names, no
            // recursion: subdirectories such as gconv/, dri/ and
            // glibc-hwcaps/ are loaded by explicit path, and letting them into
            // the soname table would let them shadow a real library.
            continue;
        }
        if !ft.is_file() && !ft.is_symlink() {
            continue;
        }
        if !md.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.contains(".so") {
            continue;
        }
        let Some(soname) = elf_soname(&path).or_else(|| Some(name.to_string())) else {
            continue;
        };
        // First hit wins, exactly like ldconfig with an ordered directory list.
        out.entry(soname).or_insert_with(|| path.to_string_lossy().into_owned());
    }
}

/// DT_SONAME from a 64-bit little-endian ELF, resolved through PT_LOAD.
fn elf_soname(path: &Path) -> Option<String> {
    let data = fs::read(path).ok()?;
    if data.len() < 64 || &data[0..4] != b"\x7fELF" || data[4] != 2 || data[5] != 1 {
        return None;
    }
    let rd16 = |o: usize| -> u16 { u16::from_le_bytes([data[o], data[o + 1]]) };
    let rd32 = |o: usize| -> u32 {
        u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
    };
    let rd64 = |o: usize| -> u64 {
        u64::from_le_bytes([
            data[o], data[o + 1], data[o + 2], data[o + 3], data[o + 4], data[o + 5], data[o + 6],
            data[o + 7],
        ])
    };
    let phoff = rd64(0x20) as usize;
    let phentsize = rd16(0x36) as usize;
    let phnum = rd16(0x38) as usize;
    if phentsize < 56 || phnum == 0 || phnum > 128 {
        return None;
    }
    let mut loads: Vec<(u64, u64, u64)> = Vec::new(); // vaddr, offset, filesz
    let mut dynamic: Option<(u64, u64)> = None;
    for i in 0..phnum {
        let p = phoff + i * phentsize;
        if p + 56 > data.len() {
            return None;
        }
        let ty = rd32(p);
        let offset = rd64(p + 8);
        let vaddr = rd64(p + 16);
        let filesz = rd64(p + 32);
        match ty {
            1 => loads.push((vaddr, offset, filesz)),
            2 => dynamic = Some((offset, filesz)),
            _ => {}
        }
    }
    let (dyn_off, dyn_sz) = dynamic?;
    let v2o = |v: u64| -> Option<usize> {
        for (lv, lo, ls) in &loads {
            if v >= *lv && v < *lv + *ls {
                return Some((lo + (v - lv)) as usize);
            }
        }
        None
    };
    let mut strtab: Option<u64> = None;
    let mut soname_off: Option<u64> = None;
    let mut o = dyn_off as usize;
    let end = (dyn_off + dyn_sz) as usize;
    while o + 16 <= end && o + 16 <= data.len() {
        let tag = rd64(o) as i64;
        let val = rd64(o + 8);
        match tag {
            0 => break,
            5 => strtab = Some(val),
            14 => soname_off = Some(val),
            _ => {}
        }
        o += 16;
    }
    let st = v2o(strtab?)?;
    let so = st + soname_off? as usize;
    if so >= data.len() {
        return None;
    }
    let rest = &data[so..];
    let len = rest.iter().position(|b| *b == 0)?;
    let s = std::str::from_utf8(&rest[..len]).ok()?.to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Serialise the `glibc-ld.so.cache` 1.1 format: 48-byte header, 24-byte
/// entries sorted by soname, then the string table.
fn write_cache(cache_path: &str, found: &BTreeMap<String, String>) -> std::io::Result<()> {
    let mut strings: Vec<u8> = Vec::new();
    let mut offsets: Vec<(u32, u32)> = Vec::with_capacity(found.len());
    for (soname, path) in found {
        let key = strings.len() as u32;
        strings.extend_from_slice(soname.as_bytes());
        strings.push(0);
        let value = strings.len() as u32;
        strings.extend_from_slice(path.as_bytes());
        strings.push(0);
        offsets.push((key, value));
    }
    let header_size = 48usize;
    let entries_size = 24usize * found.len();
    let str_base = (header_size + entries_size) as u32;

    let mut buf: Vec<u8> = Vec::with_capacity(header_size + entries_size + strings.len());
    buf.extend_from_slice(CACHE_NEW_MAGIC);
    buf.extend_from_slice(CACHE_NEW_VERSION);
    buf.extend_from_slice(&(found.len() as u32).to_le_bytes()); // nlibs
    buf.extend_from_slice(&(strings.len() as u32).to_le_bytes()); // len_strings
    buf.push(ENDIAN_LITTLE); // flags: endianness
    buf.extend_from_slice(&[0u8; 3]); // padding
    buf.extend_from_slice(&0u32.to_le_bytes()); // extension_offset: none
    buf.extend_from_slice(&[0u8; 12]); // unused, pads header to 48
    debug_assert_eq!(buf.len(), header_size);

    for (key, value) in &offsets {
        buf.extend_from_slice(&FLAGS.to_le_bytes());
        buf.extend_from_slice(&(str_base + key).to_le_bytes());
        buf.extend_from_slice(&(str_base + value).to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // osversion_unused
        buf.extend_from_slice(&0u64.to_le_bytes()); // hwcap
    }
    debug_assert_eq!(buf.len(), header_size + entries_size);
    buf.extend_from_slice(&strings);

    // Write next to the target and rename, like ldconfig, so a reader never sees
    // a half-written cache.
    let tmp = format!("{cache_path}~");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&buf)?;
        f.sync_all()?;
    }
    fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
    fs::rename(&tmp, cache_path)?;
    Ok(())
}

fn print_cache(cache_path: &str) {
    let Ok(data) = fs::read(cache_path) else {
        eprintln!("husk-ldconfig: {cache_path} is unreadable");
        std::process::exit(1);
    };
    if data.len() < 48 || &data[0..17] != CACHE_NEW_MAGIC {
        eprintln!("husk-ldconfig: {cache_path} is not a 1.1-format cache");
        std::process::exit(1);
    }
    let rd32 = |o: usize| -> u32 {
        u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
    };
    let nlibs = rd32(20) as usize;
    let mut out = String::from("\tlibc6 (libc6,AArch64) => name (path)\n");
    for i in 0..nlibs {
        let e = 48 + i * 24;
        if e + 24 > data.len() {
            break;
        }
        let key = rd32(e + 4) as usize;
        let value = rd32(e + 8) as usize;
        let s = |at: usize| -> String {
            let rest = &data[at.min(data.len())..];
            let n = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..n]).into_owned()
        };
        out.push_str(&format!("\t{} (libc6,AArch64) => {} (0x0)\n", s(key), s(value)));
    }
    // `ldconfig -p | head` closes the pipe early; a panic there would look like
    // a real failure to a caller that only reads the first lines.
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(out.as_bytes());
    let _ = so.flush();
}

#[allow(dead_code)]
fn _unused(md: &fs::Metadata) -> bool {
    md.file_type().is_char_device()
}
