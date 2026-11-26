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
//!   --library-path-file <f>
//!              write every absolute directory the guest's own binaries declare
//!              in DT_RUNPATH / DT_RPATH, one per line (see §"runpath dirs")

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
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
    let mut library_path_file: Option<String> = None;
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
            "--library-path-file" => {
                i += 1;
                match args.get(i) {
                    Some(v) => library_path_file = Some(v.clone()),
                    None => die("husk-ldconfig: --library-path-file needs a path"),
                }
            }
            "-f" | "-r" => {
                // -f <conf>: we read /etc/ld.so.conf.d ourselves; -r <root> is a
                // chroot we cannot perform without CAP_SYS_CHROOT, so both are
                // accepted and ignored with a note on stderr.
                i += 1;
                eprintln!(
                    "husk-ldconfig: note: '{}' is not applied inside the runtime",
                    args[i - 1]
                );
            }
            other if other.starts_with('-') => {
                eprintln!("husk-ldconfig: unknown option {other}, ignoring");
            }
            other => dirs.push(other.to_string()),
        }
        i += 1;
    }

    if let Some(out) = library_path_file {
        write_library_path(&out);
        return;
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
        let Ok(md) = fs::metadata(&path) else {
            continue;
        }; // follows symlinks
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
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.contains(".so") {
            continue;
        }
        let Some(soname) = elf_soname(&path).or_else(|| Some(name.to_string())) else {
            continue;
        };
        // First hit wins, exactly like ldconfig with an ordered directory list.
        out.entry(soname)
            .or_insert_with(|| path.to_string_lossy().into_owned());
    }
}

// -------------------------------------------------------------- runpath dirs

/// Where the guest's own binaries may declare private library directories.
/// Recursive: the interesting ones are nested (`gio/modules`, `dri`,
/// `aarch64-linux-gnu/libproxy`, `aarch64-linux-gnu/gio/modules`).
const RUNPATH_ROOTS: [&str; 6] = [
    "/usr/bin",
    "/usr/sbin",
    "/usr/libexec",
    "/usr/lib",
    "/usr/games",
    "/usr/local",
];

/// Every absolute directory the guest's binaries ask the loader to search.
///
/// glibc resolves `DT_RUNPATH` / `DT_RPATH` **inside the loader**, where there is
/// no GOT to interpose, so those absolute paths are looked up in the host
/// namespace and missed. Measured on the 2.0 tree: `sudo` cannot find
/// `libsudo_util.so.0` (RUNPATH `/usr/libexec/sudo`) and gio's proxy module
/// cannot find `libpxbackend-1.0.so` - the files are right there in the rootfs
/// (evidence 139 §3.1). Until the loader's own open is taught to translate,
/// the launcher supplies the answer: this list becomes `--library-path`.
///
/// `$ORIGIN`-relative entries are skipped on purpose: the object itself was
/// opened through a real path under the rootfs, so `$ORIGIN` already resolves
/// correctly and rewriting it would break it.
fn write_library_path(out: &str) {
    let mut dirs: Vec<String> = Vec::new();
    let mut seen_roots: Vec<PathBuf> = Vec::new();
    for root in RUNPATH_ROOTS {
        let real = fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
        if seen_roots.iter().any(|p| *p == real) {
            continue; // /bin and /lib are the same directories under merged /usr
        }
        seen_roots.push(real.clone());
        collect_runpath_dirs(&real, &mut dirs, 0);
    }
    // DT_RUNPATH may name optional directories that the installed package set
    // does not provide. Record only directories that actually exist in this
    // guest tree; otherwise a stale/non-shipped entry turns into a permanent
    // install failure and a malformed --library-path component.
    dirs.retain(|d| Path::new(d).is_dir());
    dirs.sort();
    dirs.dedup();
    let mut text = String::from("# generated by husk-ldconfig --library-path-file\n");
    for d in &dirs {
        text.push_str(d);
        text.push('\n');
    }
    match fs::write(out, text) {
        Ok(()) => println!(
            "husk-ldconfig: wrote {out} ({} runpath directories)",
            dirs.len()
        ),
        Err(e) => {
            eprintln!("husk-ldconfig: cannot write {out}: {e}");
            std::process::exit(1);
        }
    }
}

fn collect_runpath_dirs(dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let Ok(md) = fs::metadata(&path) else {
            continue;
        }; // follows symlinks
        if md.is_dir() {
            collect_runpath_dirs(&path, out, depth + 1);
            continue;
        }
        if !md.is_file() || md.len() < 64 {
            continue;
        }
        let Some(values) = elf_runpath(&path) else {
            continue;
        };
        for value in values {
            for part in value.split(':') {
                let part = part.trim();
                if part.is_empty()
                    || !part.starts_with('/')
                    || part.contains("$ORIGIN")
                    || part.contains("${ORIGIN}")
                {
                    continue;
                }
                let d = part.trim_end_matches('/');
                if !d.is_empty() {
                    out.push(d.to_string());
                }
            }
        }
    }
}

/// DT_RUNPATH (29) and DT_RPATH (15) of a 64-bit little-endian ELF.
///
/// Reads only the header, the program headers, the dynamic segment and the
/// dynamic string table: /usr/lib holds a few thousand objects and this runs on
/// a phone, so a whole-file read is not an option.
fn elf_runpath(path: &Path) -> Option<Vec<String>> {
    let mut f = fs::File::open(path).ok()?;
    let mut eh = [0u8; 64];
    f.read_exact(&mut eh).ok()?;
    if &eh[0..4] != b"\x7fELF" || eh[4] != 2 || eh[5] != 1 {
        return None;
    }
    let rd16 = |o: usize| -> u16 { u16::from_le_bytes([eh[o], eh[o + 1]]) };
    let rd64 = |o: usize| -> u64 {
        u64::from_le_bytes([
            eh[o],
            eh[o + 1],
            eh[o + 2],
            eh[o + 3],
            eh[o + 4],
            eh[o + 5],
            eh[o + 6],
            eh[o + 7],
        ])
    };
    let phoff = rd64(0x20);
    let phentsize = rd16(0x36) as usize;
    let phnum = rd16(0x38) as usize;
    if phentsize < 56 || phnum == 0 || phnum > 128 {
        return None;
    }
    let mut phbuf = vec![0u8; phentsize * phnum];
    f.seek(SeekFrom::Start(phoff)).ok()?;
    f.read_exact(&mut phbuf).ok()?;
    let be32 =
        |b: &[u8], o: usize| -> u32 { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) };
    let be64 = |b: &[u8], o: usize| -> u64 {
        u64::from_le_bytes([
            b[o],
            b[o + 1],
            b[o + 2],
            b[o + 3],
            b[o + 4],
            b[o + 5],
            b[o + 6],
            b[o + 7],
        ])
    };
    let mut loads: Vec<(u64, u64, u64)> = Vec::new();
    let mut dynamic: Option<(u64, u64)> = None;
    for i in 0..phnum {
        let p = i * phentsize;
        match be32(&phbuf, p) {
            1 => loads.push((
                be64(&phbuf, p + 16),
                be64(&phbuf, p + 8),
                be64(&phbuf, p + 32),
            )),
            2 => dynamic = Some((be64(&phbuf, p + 8), be64(&phbuf, p + 32))),
            _ => {}
        }
    }
    let (doff, dsz) = dynamic?;
    if dsz == 0 || dsz > 64 * 1024 {
        return None;
    }
    let mut dbuf = vec![0u8; dsz as usize];
    f.seek(SeekFrom::Start(doff)).ok()?;
    f.read_exact(&mut dbuf).ok()?;
    let mut strtab_va: Option<u64> = None;
    let mut strsz: u64 = 0;
    let mut offsets: Vec<u64> = Vec::new();
    let mut o = 0usize;
    while o + 16 <= dbuf.len() {
        let tag = be64(&dbuf, o) as i64;
        let val = be64(&dbuf, o + 8);
        match tag {
            0 => break,
            5 => strtab_va = Some(val),
            10 => strsz = val,
            15 | 29 => offsets.push(val),
            _ => {}
        }
        o += 16;
    }
    if offsets.is_empty() {
        return None;
    }
    let sv = strtab_va?;
    let mut st_off = None;
    for (va, off, fsz) in &loads {
        if sv >= *va && sv < *va + *fsz {
            st_off = Some(off + (sv - va));
            break;
        }
    }
    let st_off = st_off?;
    let n = if strsz > 0 && strsz < (1 << 20) {
        strsz as usize
    } else {
        8192
    };
    let mut sbuf = vec![0u8; n];
    f.seek(SeekFrom::Start(st_off)).ok()?;
    let got = f.read(&mut sbuf).ok()?;
    sbuf.truncate(got);
    let mut out = Vec::new();
    for off in offsets {
        let off = off as usize;
        if off >= sbuf.len() {
            continue;
        }
        let Some(len) = sbuf[off..].iter().position(|b| *b == 0) else {
            continue;
        };
        if let Ok(s) = std::str::from_utf8(&sbuf[off..off + len]) {
            out.push(s.to_string());
        }
    }
    Some(out)
}

/// DT_SONAME from a 64-bit little-endian ELF, resolved through PT_LOAD.
fn elf_soname(path: &Path) -> Option<String> {
    let data = fs::read(path).ok()?;
    if data.len() < 64 || &data[0..4] != b"\x7fELF" || data[4] != 2 || data[5] != 1 {
        return None;
    }
    let rd16 = |o: usize| -> u16 { u16::from_le_bytes([data[o], data[o + 1]]) };
    let rd32 =
        |o: usize| -> u32 { u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) };
    let rd64 = |o: usize| -> u64 {
        u64::from_le_bytes([
            data[o],
            data[o + 1],
            data[o + 2],
            data[o + 3],
            data[o + 4],
            data[o + 5],
            data[o + 6],
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
    let rd32 =
        |o: usize| -> u32 { u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) };
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
        out.push_str(&format!(
            "\t{} (libc6,AArch64) => {} (0x0)\n",
            s(key),
            s(value)
        ));
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
