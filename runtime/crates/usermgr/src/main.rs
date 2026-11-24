//! husk-usermgr — the guest's own account management.
//!
//! Ubuntu's shadow tools cannot work here. `lckpwdf()` decides whether the
//! password database is free by hard-linking `/etc/passwd` to a lock name and
//! requiring `st_nlink == 2`; this SELinux domain refuses `link(2)` inside the
//! app's private directory (measured: EACCES from a bare toybox `ln`, no shim
//! involved). So `useradd` and `groupadd` always answer
//!
//!     useradd: /etc/passwd.21848: lock file already used (nlink: 1)
//!
//! and every package whose postinst creates an account fails to install.
//! Shipping the accounts in the image (tools/guest-config/guest-accounts.txt)
//! only covers accounts we knew about in advance.
//!
//! So the guest gets its own implementations, sharing one database layer:
//!
//!     useradd  groupadd  usermod  userdel  groupdel  chpasswd  passwd
//!
//! They edit the four files directly (`/etc/passwd`, `/etc/shadow`,
//! `/etc/group`, `/etc/gshadow`) under one `flock(2)` on `/etc/.pwd.lock`,
//! which the kernel does allow. Everything else follows shadow's documented
//! behaviour: UID/GID ranges from `login.defs`, USERGROUPS_ENAB, HOME_MODE,
//! CREATE_HOME, /etc/skel, `!`-locked passwords, SHA512 via the system
//! `crypt(3)`.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

extern "C" {
    fn lchown(path: *const std::os::raw::c_char, uid: u32, gid: u32) -> i32;
}

const PASSWD: &str = "/etc/passwd";
const SHADOW: &str = "/etc/shadow";
const GROUP: &str = "/etc/group";
const GSHADOW: &str = "/etc/gshadow";
const LOCKFILE: &str = "/etc/.pwd.lock";
const LOGIN_DEFS: &str = "/etc/login.defs";
const SKEL: &str = "/etc/skel";
const NOLOGIN: &str = "/usr/sbin/nologin";

type R<T> = Result<T, String>;

// ------------------------------------------------------------------ records --

#[derive(Clone, Debug)]
struct Pw {
    name: String,
    passwd: String,
    uid: u32,
    gid: u32,
    gecos: String,
    home: String,
    shell: String,
}

#[derive(Clone, Debug)]
struct Sh {
    name: String,
    passwd: String,
    lastchg: String,
    min: String,
    max: String,
    warn: String,
    inactive: String,
    expire: String,
    flag: String,
}

#[derive(Clone, Debug)]
struct Gr {
    name: String,
    passwd: String,
    gid: u32,
    members: Vec<String>,
}

#[derive(Clone, Debug)]
struct GS {
    name: String,
    passwd: String,
    admins: Vec<String>,
    members: Vec<String>,
}

/// A line we could not parse is carried through untouched: these files are
/// shared with the rest of the system, and dropping a line we did not expect
/// would be worse than leaving it alone.
#[derive(Clone, Debug)]
enum Line<T> {
    Ent(T),
    Raw(String),
}

impl Pw {
    fn render(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.name, self.passwd, self.uid, self.gid, self.gecos, self.home, self.shell
        )
    }
}

impl Sh {
    fn render(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.name, self.passwd, self.lastchg, self.min, self.max, self.warn, self.inactive,
            self.expire, self.flag
        )
    }
}

impl Gr {
    fn render(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.name,
            self.passwd,
            self.gid,
            self.members.join(",")
        )
    }
}

impl GS {
    fn render(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.name,
            self.passwd,
            self.admins.join(","),
            self.members.join(",")
        )
    }
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect()
}

fn parse_pw(line: &str) -> Option<Pw> {
    let f: Vec<&str> = line.split(':').collect();
    if f.len() != 7 {
        return None;
    }
    Some(Pw {
        name: f[0].into(),
        passwd: f[1].into(),
        uid: f[2].parse().ok()?,
        gid: f[3].parse().ok()?,
        gecos: f[4].into(),
        home: f[5].into(),
        shell: f[6].into(),
    })
}

fn parse_sh(line: &str) -> Option<Sh> {
    let f: Vec<&str> = line.split(':').collect();
    if f.len() != 9 {
        return None;
    }
    Some(Sh {
        name: f[0].into(),
        passwd: f[1].into(),
        lastchg: f[2].into(),
        min: f[3].into(),
        max: f[4].into(),
        warn: f[5].into(),
        inactive: f[6].into(),
        expire: f[7].into(),
        flag: f[8].into(),
    })
}

fn parse_gr(line: &str) -> Option<Gr> {
    let f: Vec<&str> = line.split(':').collect();
    if f.len() != 4 {
        return None;
    }
    Some(Gr {
        name: f[0].into(),
        passwd: f[1].into(),
        gid: f[2].parse().ok()?,
        members: split_list(f[3]),
    })
}

fn parse_gs(line: &str) -> Option<GS> {
    let f: Vec<&str> = line.split(':').collect();
    if f.len() != 4 {
        return None;
    }
    Some(GS {
        name: f[0].into(),
        passwd: f[1].into(),
        admins: split_list(f[2]),
        members: split_list(f[3]),
    })
}

fn read_entries<T, F>(path: &str, parse: F) -> Vec<Line<T>>
where
    F: Fn(&str) -> Option<T>,
{
    let mut out = Vec::new();
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return out,
    };
    for line in data.lines() {
        if line.is_empty() {
            continue;
        }
        match parse(line) {
            Some(e) => out.push(Line::Ent(e)),
            None => out.push(Line::Raw(line.to_string())),
        }
    }
    out
}

fn render_lines<T, F>(lines: &[Line<T>], render: F) -> String
where
    F: Fn(&T) -> String,
{
    let mut s = String::new();
    for l in lines {
        match l {
            Line::Ent(e) => {
                s.push_str(&render(e));
                s.push('\n');
            }
            Line::Raw(r) => {
                s.push_str(r);
                s.push('\n');
            }
        }
    }
    s
}

struct Db {
    passwd: Vec<Line<Pw>>,
    shadow: Vec<Line<Sh>>,
    group: Vec<Line<Gr>>,
    gshadow: Vec<Line<GS>>,
}

impl Db {
    fn load() -> Db {
        Db {
            passwd: read_entries(PASSWD, parse_pw),
            shadow: read_entries(SHADOW, parse_sh),
            group: read_entries(GROUP, parse_gr),
            gshadow: read_entries(GSHADOW, parse_gs),
        }
    }

    fn save(&self) -> R<()> {
        write_atomic(&PASSWD, &render_lines(&self.passwd, |p| p.render()), 0o644)?;
        write_atomic(&SHADOW, &render_lines(&self.shadow, |s| s.render()), 0o640)?;
        write_atomic(&GROUP, &render_lines(&self.group, |g| g.render()), 0o644)?;
        write_atomic(&GSHADOW, &render_lines(&self.gshadow, |g| g.render()), 0o640)?;
        Ok(())
    }

    fn pw(&self, name: &str) -> Option<&Pw> {
        self.passwd.iter().find_map(|l| match l {
            Line::Ent(p) if p.name == name => Some(p),
            _ => None,
        })
    }

    fn pw_by_uid(&self, uid: u32) -> Option<&Pw> {
        self.passwd.iter().find_map(|l| match l {
            Line::Ent(p) if p.uid == uid => Some(p),
            _ => None,
        })
    }

    fn gr(&self, name: &str) -> Option<&Gr> {
        self.group.iter().find_map(|l| match l {
            Line::Ent(g) if g.name == name => Some(g),
            _ => None,
        })
    }

    fn gr_by_gid(&self, gid: u32) -> Option<&Gr> {
        self.group.iter().find_map(|l| match l {
            Line::Ent(g) if g.gid == gid => Some(g),
            _ => None,
        })
    }

    fn sh(&self, name: &str) -> Option<&Sh> {
        self.shadow.iter().find_map(|l| match l {
            Line::Ent(s) if s.name == name => Some(s),
            _ => None,
        })
    }

    fn users_pw(&self) -> Vec<Pw> {
        self.passwd
            .iter()
            .filter_map(|l| match l {
                Line::Ent(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    fn groups_gr(&self) -> Vec<Gr> {
        self.group
            .iter()
            .filter_map(|l| match l {
                Line::Ent(g) => Some(g.clone()),
                _ => None,
            })
            .collect()
    }
}

fn write_atomic(path: &str, content: &str, mode: u32) -> R<()> {
    let tmp = format!("{path}.husk.tmp");
    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(&tmp)
            .map_err(|e| format!("cannot open {tmp}: {e}"))?;
        f.write_all(content.as_bytes())
            .map_err(|e| format!("cannot write {tmp}: {e}"))?;
        f.sync_all().ok();
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)).ok();
    fs::rename(&tmp, path).map_err(|e| format!("cannot rename {tmp} -> {path}: {e}"))?;
    Ok(())
}

// ------------------------------------------------------------------ locking --

extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

const LOCK_EX: i32 = 2;

struct Lock {
    _f: File,
}

fn lock_pwdb() -> R<Lock> {
    let f = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(LOCKFILE)
        .map_err(|e| format!("cannot open {LOCKFILE}: {e}"))?;
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { flock(f.as_raw_fd(), LOCK_EX) };
    if rc != 0 {
        return Err(format!("cannot lock {LOCKFILE}: {}", io::Error::last_os_error()));
    }
    Ok(Lock { _f: f })
}

// -------------------------------------------------------------- login.defs --

fn login_defs() -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Ok(data) = fs::read_to_string(LOGIN_DEFS) {
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                m.entry(k.to_string()).or_insert_with(|| v.to_string());
            }
        }
    }
    m
}

fn def_num(m: &HashMap<String, String>, key: &str, dflt: u32) -> u32 {
    m.get(key).and_then(|v| v.parse().ok()).unwrap_or(dflt)
}

fn def_str(m: &HashMap<String, String>, key: &str) -> Option<String> {
    m.get(key).cloned()
}

fn def_yes(m: &HashMap<String, String>, key: &str) -> bool {
    m.get(key).map(|v| v == "yes").unwrap_or(false)
}

// ------------------------------------------------------------------- naming --

fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 32 {
        return false;
    }
    let b = name.as_bytes();
    if b[0].is_ascii_digit() || b[0] == b'-' || b[0] == b'.' {
        return false;
    }
    if name == "." || name == ".." {
        return false;
    }
    name.bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.')
}

// -------------------------------------------------------------- allocation --

fn first_free_uid(db: &Db, lo: u32, hi: u32) -> Option<u32> {
    (lo..=hi).find(|u| db.pw_by_uid(*u).is_none())
}

fn first_free_gid(db: &Db, lo: u32, hi: u32) -> Option<u32> {
    (lo..=hi).find(|g| db.gr_by_gid(*g).is_none())
}

// ------------------------------------------------------------------- crypt ---

extern "C" {
    fn dlopen(filename: *const core::ffi::c_char, flag: i32) -> *mut core::ffi::c_void;
    fn dlsym(
        handle: *mut core::ffi::c_void,
        symbol: *const core::ffi::c_char,
    ) -> *mut core::ffi::c_void;
}

const RTLD_NOW: i32 = 2;

fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if let Ok(mut f) = File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() {
            return buf;
        }
    }
    // Deterministic fallback so we never write an empty salt.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for (i, b) in buf.iter_mut().enumerate() {
        *b = ((t >> ((i % 16) * 8)) as u8).wrapping_mul(31).wrapping_add(i as u8);
    }
    buf
}

const ALNUM: &[u8] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn make_salt(method: &str) -> String {
    let prefix = match method {
        "MD5" => "$1$",
        "SHA256" => "$5$",
        _ => "$6$",
    };
    let mut salt = String::from(prefix);
    for b in random_bytes(16) {
        salt.push(ALNUM[(b as usize) % ALNUM.len()] as char);
    }
    salt.push('$');
    salt
}

/// `crypt(3)` from the guest's libcrypt. Reimplementing SHA512-crypt would only
/// add a way to get it subtly wrong; the library is right there and is what
/// `login` itself will use to verify.
fn crypt(pass: &str, salt: &str) -> R<String> {
    let lib = CString::new("libcrypt.so.1").unwrap();
    unsafe {
        let h = dlopen(lib.as_ptr() as *const core::ffi::c_char, RTLD_NOW);
        if h.is_null() {
            return Err("cannot load libcrypt.so.1".into());
        }
        let sym = CString::new("crypt").unwrap();
        let fp = dlsym(h, sym.as_ptr() as *const core::ffi::c_char);
        if fp.is_null() {
            return Err("libcrypt has no crypt".into());
        }
        let f: unsafe extern "C" fn(
            *const core::ffi::c_char,
            *const core::ffi::c_char,
        ) -> *const core::ffi::c_char = std::mem::transmute(fp);
        let key = CString::new(pass).map_err(|_| "password contains NUL".to_string())?;
        let s = CString::new(salt).map_err(|_| "salt contains NUL".to_string())?;
        let out = f(
            key.as_ptr() as *const core::ffi::c_char,
            s.as_ptr() as *const core::ffi::c_char,
        );
        if out.is_null() {
            return Err("crypt(3) failed".into());
        }
        let cstr = std::ffi::CStr::from_ptr(out as *const core::ffi::c_char);
        Ok(cstr.to_string_lossy().into_owned())
    }
}

fn hash_password(pass: &str, method: &str) -> R<String> {
    if pass.starts_with('$') || pass.starts_with('!') || pass == "*" {
        // Already hashed (or intentionally locked): store verbatim.
        return Ok(pass.to_string());
    }
    crypt(pass, &make_salt(method))
}

// ------------------------------------------------------------------- dates ---

fn today_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD` -> days since the epoch, as shadow stores it. Also accepts the
/// empty string (never expires) and a bare day count.
fn parse_date_days(s: &str) -> Option<i64> {
    if s.is_empty() {
        return Some(0);
    }
    if let Ok(v) = s.parse::<i64>() {
        return Some(v);
    }
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

// ------------------------------------------------------------------- parser --

#[derive(Default)]
struct Opts {
    flags: Vec<String>,
    vals: HashMap<String, String>,
    pos: Vec<String>,
}

/// Parses both `-m -s /bin/sh` and `--shell=/bin/sh`. `with_value` lists the
/// option letters that consume an argument.
fn parse_opts(args: &[String], with_value: &str, long_map: &[(&str, char)]) -> R<Opts> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            o.pos.extend(args[i + 1..].iter().cloned());
            break;
        }
        if let Some(rest) = a.strip_prefix("--") {
            let (k, v) = match rest.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (rest, None),
            };
            match k {
                "help" => {
                    o.flags.push("h".into());
                    i += 1;
                    continue;
                }
                "version" => {
                    o.flags.push("V".into());
                    i += 1;
                    continue;
                }
                "badnames" | "no-log-init" | "extrausers" => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let c = long_map
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, c)| *c)
                .ok_or_else(|| format!("unrecognized option '--{k}'"))?;
            if with_value.contains(c) {
                let val = match v {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i)
                            .cloned()
                            .ok_or_else(|| format!("option '--{k}' requires an argument"))?
                    }
                };
                o.vals.insert(c.to_string(), val);
            } else {
                o.flags.push(c.to_string());
            }
            i += 1;
            continue;
        }
        if a.starts_with('-') && a.len() > 1 {
            let chars: Vec<char> = a[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                let c = chars[j];
                if !is_known(c, with_value) {
                    return Err(format!("invalid option -- '{c}'"));
                }
                if with_value.contains(c) {
                    let rest: String = chars[j + 1..].iter().collect();
                    let val = if !rest.is_empty() {
                        rest
                    } else {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    };
                    o.vals.insert(c.to_string(), val);
                    break;
                } else {
                    o.flags.push(c.to_string());
                }
                j += 1;
            }
            i += 1;
            continue;
        }
        o.pos.push(a.clone());
        i += 1;
    }
    Ok(o)
}

fn is_known(c: char, with_value: &str) -> bool {
    if with_value.contains(c) {
        return true;
    }
    // A conservative allow-list: unknown short options must not be silently
    // swallowed, because half of them change what we are asked to write.
    matches!(
        c,
        'm' | 'M' | 'r' | 'o' | 'U' | 'N' | 'a' | 'L' | 'u' | 'l' | 'R' | 'P' | 'e' | 'f' | 'd'
            | 's' | 'c' | 'g' | 'G' | 'b' | 'k' | 'p' | 'K' | 'h' | 'V' | 'S' | 'F' | 'Z' | 'z'
            | 'q' | 'C' | 'i' | 'w' | 'E' | 'n' | 'x'
    )
}

// ----------------------------------------------------------------- helpers ---

fn validate_new_name(db: &Db, name: &str) -> R<()> {
    if !valid_name(name) {
        return Err(format!("invalid user name '{name}'"));
    }
    if db.pw(name).is_some() {
        return Err(format!("user '{name}' already exists"));
    }
    Ok(())
}

fn resolve_group(db: &Db, spec: &str) -> R<u32> {
    if let Ok(gid) = spec.parse::<u32>() {
        if db.gr_by_gid(gid).is_none() {
            return Err(format!("group '{spec}' does not exist"));
        }
        return Ok(gid);
    }
    db.gr(spec)
        .map(|g| g.gid)
        .ok_or_else(|| format!("group '{spec}' does not exist"))
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    for ent in fs::read_dir(src)? {
        let ent = ent?;
        let name = ent.file_name();
        let s = ent.path();
        let d = dst.join(&name);
        let md = ent.metadata()?;
        if md.is_dir() {
            fs::create_dir(&d)?;
            fs::set_permissions(&d, fs::Permissions::from_mode(md.permissions().mode()))?;
            copy_dir(&s, &d)?;
        } else if md.file_type().is_symlink() {
            let target = fs::read_link(&s)?;
            std::os::unix::fs::symlink(target, &d)?;
        } else {
            fs::copy(&s, &d)?;
            fs::set_permissions(&d, fs::Permissions::from_mode(md.permissions().mode()))?;
        }
    }
    Ok(())
}

/// `chown -R` as the reference `useradd` does it: everything under the new home
/// belongs to the new account. Guest chown(2) is answered by the runtime's
/// ownership table (it cannot really change a file's owner here), but the answer
/// is what every later process reads back - without this, the first `sudo` or
/// `ssh` run as the user refuses its own home as "owned by uid 0".
fn chown_tree(p: &Path, uid: u32, gid: u32) {
    let c = match CString::new(p.as_os_str().to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return,
    };
    unsafe {
        lchown(c.as_ptr(), uid, gid);
    }
    if let Ok(rd) = fs::read_dir(p) {
        for ent in rd.flatten() {
            let t = ent.path();
            if !ent.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
                chown_tree(&t, uid, gid);
            }
        }
    }
}

fn create_home(home: &str, skel: &str, mode: u32, uid: u32, gid: u32) -> R<()> {
    let p = Path::new(home);
    if p.exists() {
        return Ok(());
    }
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {parent:?}: {e}"))?;
        }
    }
    fs::create_dir(p).map_err(|e| format!("cannot create directory {home}: {e}"))?;
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).ok();
    let sk = Path::new(skel);
    if sk.is_dir() {
        copy_dir(sk, p).map_err(|e| format!("warning: cannot copy {skel}: {e}"))?;
    }
    chown_tree(p, uid, gid);
    Ok(())
}

fn append_mail_spool(user: &str) -> R<()> {
    let m = format!("/var/mail/{user}");
    let p = Path::new("/var/mail");
    if !p.is_dir() {
        return Ok(());
    }
    if !Path::new(&m).exists() {
        File::create(&m).map_err(|e| format!("cannot create {m}: {e}"))?;
        fs::set_permissions(&m, fs::Permissions::from_mode(0o660)).ok();
    }
    Ok(())
}

// ------------------------------------------------------------------ useradd --

fn cmd_useradd(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[
        ("system", 'r'),
        ("create-home", 'm'),
        ("no-create-home", 'M'),
        ("home-dir", 'd'),
        ("base-dir", 'b'),
        ("shell", 's'),
        ("uid", 'u'),
        ("gid", 'g'),
        ("groups", 'G'),
        ("comment", 'c'),
        ("expiredate", 'e'),
        ("inactive", 'f'),
        ("non-unique", 'o'),
        ("skel", 'k'),
        ("password", 'p'),
        ("user-group", 'U'),
        ("no-user-group", 'N'),
        ("key", 'K'),
        ("root", 'R'),
        ("prefix", 'P'),
    ];
    let o = match parse_opts(args, "dbsugGcefkpKRP", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("useradd: {e}");
            return 2;
        }
    };
    if o.flags.iter().any(|f| f == "h") || o.pos.is_empty() {
        eprintln!("usage: useradd [options] LOGIN");
        return 2;
    }
    let name = o.pos[0].clone();
    let system = o.flags.iter().any(|f| f == "r");
    let create_home_flag = o.flags.iter().any(|f| f == "m");
    let no_home = o.flags.iter().any(|f| f == "M");
    let non_unique = o.flags.iter().any(|f| f == "o");
    let user_group = o.flags.iter().any(|f| f == "U");
    let no_user_group = o.flags.iter().any(|f| f == "N");

    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("useradd: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    if let Err(e) = validate_new_name(&db, &name) {
        eprintln!("useradd: {e}");
        return 9;
    }
    let defs = login_defs();

    let (uid_lo, uid_hi) = if system {
        (
            def_num(&defs, "SYS_UID_MIN", 100),
            def_num(&defs, "SYS_UID_MAX", 999),
        )
    } else {
        (
            def_num(&defs, "UID_MIN", 1000),
            def_num(&defs, "UID_MAX", 60000),
        )
    };

    let uid = match o.vals.get("u") {
        Some(v) => match v.parse::<u32>() {
            Ok(u) => {
                if !non_unique && db.pw_by_uid(u).is_some() {
                    eprintln!("useradd: UID {u} is not unique");
                    return 4;
                }
                u
            }
            Err(_) => {
                eprintln!("useradd: invalid user ID '{v}'");
                return 3;
            }
        },
        None => match first_free_uid(&db, uid_lo, uid_hi) {
            Some(u) => u,
            None => {
                eprintln!("useradd: can't get unique UID (no more available UIDs)");
                return 4;
            }
        },
    };

    // Group: explicit -g wins; otherwise USERGROUPS_ENAB creates a private one.
    let (gid, make_group) = if let Some(g) = o.vals.get("g") {
        match resolve_group(&db, g) {
            Ok(gid) => (gid, None),
            Err(e) => {
                eprintln!("useradd: {e}");
                return 6;
            }
        }
    } else if user_group || (def_yes(&defs, "USERGROUPS_ENAB") && !no_user_group) {
        let want: u32 = if !system && db.gr_by_gid(uid).is_none() {
            uid
        } else {
            let (lo, hi) = if system {
                (
                    def_num(&defs, "SYS_GID_MIN", 100),
                    def_num(&defs, "SYS_GID_MAX", 999),
                )
            } else {
                (
                    def_num(&defs, "GID_MIN", 1000),
                    def_num(&defs, "GID_MAX", 60000),
                )
            };
            match first_free_gid(&db, lo, hi) {
                Some(g) => g,
                None => {
                    eprintln!("useradd: can't get unique GID");
                    return 10;
                }
            }
        };
        (want, Some((name.clone(), want)))
    } else {
        // shadow falls back to the "users" group when user groups are off.
        let gid = db.gr("users").map(|g| g.gid).unwrap_or(100);
        (gid, None)
    };

    let shell = if let Some(s) = o.vals.get("s") {
        s.clone()
    } else if system {
        NOLOGIN.to_string()
    } else {
        def_str(&defs, "SHELL").unwrap_or_else(|| "/bin/sh".into())
    };

    let base = o
        .vals
        .get("b")
        .cloned()
        .or_else(|| def_str(&defs, "HOME_BASE"))
        .unwrap_or_else(|| "/home".into());
    let home = if let Some(d) = o.vals.get("d") {
        d.clone()
    } else if system {
        if create_home_flag && !no_home {
            format!("{base}/{name}")
        } else {
            "/nonexistent".into()
        }
    } else {
        format!("{base}/{name}")
    };

    let gecos = o.vals.get("c").cloned().unwrap_or_default();
    let passwd_field = match o.vals.get("p") {
        Some(p) => p.clone(),
        None => "!".into(),
    };
    let shadow_pw = if passwd_field == "!" {
        "!".to_string()
    } else {
        passwd_field
    };

    let expire = match o.vals.get("e") {
        Some(v) => match parse_date_days(v) {
            Some(d) => {
                if v.is_empty() {
                    String::new()
                } else {
                    d.to_string()
                }
            }
            None => {
                eprintln!("useradd: invalid date '{v}'");
                return 3;
            }
        },
        None => String::new(),
    };
    let inactive = o.vals.get("f").cloned().unwrap_or_default();

    if let Some(groups) = o.vals.get("G") {
        for g in groups.split(',') {
            if !g.is_empty() && db.gr(g).is_none() {
                eprintln!("useradd: group '{g}' does not exist");
                return 6;
            }
        }
    }

    // Write everything; a failure halfway leaves at most a new group, which is
    // the same order shadow uses (group first, then the user).
    if let Some((gname, ggid)) = &make_group {
        db.group.push(Line::Ent(Gr {
            name: gname.clone(),
            passwd: "!".into(),
            gid: *ggid,
            members: vec![],
        }));
        db.gshadow.push(Line::Ent(GS {
            name: gname.clone(),
            passwd: "!".into(),
            admins: vec![],
            members: vec![],
        }));
    }

    let create_home_wanted =
        !no_home && (create_home_flag || def_yes(&defs, "CREATE_HOME"));
    let home_mode = defs
        .get("HOME_MODE")
        .and_then(|v| u32::from_str_radix(v.trim_start_matches('0'), 8).ok())
        .unwrap_or(0o755);
    if create_home_wanted && home != "/nonexistent" {
        if Path::new(&home).exists() {
            eprintln!("useradd: warning: the home directory {home} already exists.");
        } else if let Err(e) = create_home(
            &home,
            &o.vals.get("k").cloned().unwrap_or_else(|| SKEL.into()),
            home_mode,
            uid,
            gid,
        ) {
            eprintln!("useradd: {e}");
            return 12;
        }
    }

    let today = today_days().to_string();
    let (sh_min, sh_max) = if system {
        (String::new(), String::new())
    } else {
        (
            def_str(&defs, "PASS_MIN_DAYS").unwrap_or_else(|| "0".into()),
            def_str(&defs, "PASS_MAX_DAYS").unwrap_or_else(|| "99999".into()),
        )
    };
    let sh_warn = if system {
        String::new()
    } else {
        def_str(&defs, "PASS_WARN_AGE").unwrap_or_else(|| "7".into())
    };

    db.passwd.push(Line::Ent(Pw {
        name: name.clone(),
        passwd: "x".into(),
        uid,
        gid,
        gecos,
        home: home.clone(),
        shell,
    }));
    db.shadow.push(Line::Ent(Sh {
        name: name.clone(),
        passwd: shadow_pw,
        lastchg: today,
        min: sh_min,
        max: sh_max,
        warn: sh_warn,
        inactive,
        expire,
        flag: String::new(),
    }));

    if let Some(groups) = o.vals.get("G") {
        for g in groups.split(',') {
            if g.is_empty() {
                continue;
            }
            for l in db.group.iter_mut() {
                if let Line::Ent(gr) = l {
                    if gr.name == g && !gr.members.contains(&name) {
                        gr.members.push(name.clone());
                    }
                }
            }
            for l in db.gshadow.iter_mut() {
                if let Line::Ent(gs) = l {
                    if gs.name == g && !gs.members.contains(&name) {
                        gs.members.push(name.clone());
                    }
                }
            }
        }
    }

    if let Err(e) = db.save() {
        eprintln!("useradd: {e}");
        return 1;
    }
    if def_yes(&defs, "CREATE_MAIL_SPOOL") {
        if let Err(e) = append_mail_spool(&name) {
            eprintln!("useradd: {e}");
        }
    }
    0
}

// ----------------------------------------------------------------- groupadd --

fn cmd_groupadd(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[
        ("gid", 'g'),
        ("system", 'r'),
        ("force", 'f'),
        ("password", 'p'),
        ("key", 'K'),
        ("root", 'R'),
        ("prefix", 'P'),
        ("non-unique", 'o'),
    ];
    let o = match parse_opts(args, "gpKRP", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("groupadd: {e}");
            return 2;
        }
    };
    if o.flags.iter().any(|f| f == "h") || o.pos.is_empty() {
        eprintln!("usage: groupadd [options] GROUP");
        return 2;
    }
    let name = o.pos[0].clone();
    let system = o.flags.iter().any(|f| f == "r");
    let force = o.flags.iter().any(|f| f == "f");
    let non_unique = o.flags.iter().any(|f| f == "o");

    if !valid_name(&name) {
        eprintln!("groupadd: '{name}' is not a valid group name");
        return 3;
    }
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("groupadd: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    if db.gr(&name).is_some() {
        if force {
            return 0;
        }
        eprintln!("groupadd: group '{name}' already exists");
        return 9;
    }
    let defs = login_defs();
    let gid = match o.vals.get("g") {
        Some(v) => match v.parse::<u32>() {
            Ok(g) => {
                if !non_unique && db.gr_by_gid(g).is_some() {
                    if force {
                        return 0;
                    }
                    eprintln!("groupadd: GID '{g}' already exists");
                    return 4;
                }
                g
            }
            Err(_) => {
                eprintln!("groupadd: invalid group ID '{v}'");
                return 3;
            }
        },
        None => {
            let (lo, hi) = if system {
                (
                    def_num(&defs, "SYS_GID_MIN", 100),
                    def_num(&defs, "SYS_GID_MAX", 999),
                )
            } else {
                (
                    def_num(&defs, "GID_MIN", 1000),
                    def_num(&defs, "GID_MAX", 60000),
                )
            };
            match first_free_gid(&db, lo, hi) {
                Some(g) => g,
                None => {
                    eprintln!("groupadd: can't get unique GID");
                    return 4;
                }
            }
        }
    };
    let pw = o.vals.get("p").cloned().unwrap_or_else(|| "!".into());
    db.group.push(Line::Ent(Gr {
        name: name.clone(),
        passwd: "x".into(),
        gid,
        members: vec![],
    }));
    db.gshadow.push(Line::Ent(GS {
        name,
        passwd: pw,
        admins: vec![],
        members: vec![],
    }));
    if let Err(e) = db.save() {
        eprintln!("groupadd: {e}");
        return 10;
    }
    0
}

// ------------------------------------------------------------------ usermod --

fn cmd_usermod(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[
        ("login", 'l'),
        ("home", 'd'),
        ("move-home", 'm'),
        ("shell", 's'),
        ("comment", 'c'),
        ("gid", 'g'),
        ("groups", 'G'),
        ("append", 'a'),
        ("lock", 'L'),
        ("unlock", 'U'),
        ("uid", 'u'),
        ("expiredate", 'e'),
        ("inactive", 'f'),
        ("root", 'R'),
        ("prefix", 'P'),
    ];
    let o = match parse_opts(args, "ldscgGuefRP", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("usermod: {e}");
            return 2;
        }
    };
    if o.pos.is_empty() {
        eprintln!("usage: usermod [options] LOGIN");
        return 2;
    }
    let name = o.pos[0].clone();
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("usermod: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    let cur = match db.pw(&name) {
        Some(p) => p.clone(),
        None => {
            eprintln!("usermod: user '{name}' does not exist");
            return 6;
        }
    };
    let newname = o.vals.get("l").cloned().unwrap_or_else(|| name.clone());
    if newname != name {
        if !valid_name(&newname) || db.pw(&newname).is_some() {
            eprintln!("usermod: user '{newname}' already exists or is invalid");
            return 9;
        }
    }
    let mut newp = cur.clone();
    newp.name = newname.clone();
    if let Some(v) = o.vals.get("d") {
        newp.home = v.clone();
    }
    if let Some(v) = o.vals.get("s") {
        newp.shell = v.clone();
    }
    if let Some(v) = o.vals.get("c") {
        newp.gecos = v.clone();
    }
    if let Some(v) = o.vals.get("g") {
        match resolve_group(&db, v) {
            Ok(g) => newp.gid = g,
            Err(e) => {
                eprintln!("usermod: {e}");
                return 6;
            }
        }
    }
    if let Some(v) = o.vals.get("u") {
        match v.parse::<u32>() {
            Ok(u) => {
                if let Some(other) = db.pw_by_uid(u) {
                    if other.name != name {
                        eprintln!("usermod: UID '{u}' already exists");
                        return 4;
                    }
                }
                newp.uid = u;
            }
            Err(_) => {
                eprintln!("usermod: invalid user ID '{v}'");
                return 3;
            }
        }
    }

    let lock_user = o.flags.iter().any(|f| f == "L");
    let unlock_user = o.flags.iter().any(|f| f == "U");
    let expire = o.vals.get("e").and_then(|v| parse_date_days(v));
    let inactive = o.vals.get("f").cloned();

    for l in db.passwd.iter_mut() {
        if let Line::Ent(p) = l {
            if p.name == name {
                *p = newp.clone();
            }
        }
    }
    for l in db.shadow.iter_mut() {
        if let Line::Ent(s) = l {
            if s.name == name {
                s.name = newname.clone();
                if lock_user && !s.passwd.starts_with('!') {
                    s.passwd = format!("!{}", s.passwd);
                }
                if unlock_user && s.passwd.starts_with('!') {
                    s.passwd = s.passwd.trim_start_matches('!').to_string();
                }
                if let Some(e) = expire {
                    s.expire = if e == 0 { String::new() } else { e.to_string() };
                }
                if let Some(i) = &inactive {
                    s.inactive = i.clone();
                }
            }
        }
    }

    // Group membership: -G replaces, -a -G appends.
    if let Some(list) = o.vals.get("G") {
        let append = o.flags.iter().any(|f| f == "a");
        let want: Vec<String> = list.split(',').filter(|v| !v.is_empty()).map(|v| v.to_string()).collect();
        for g in &want {
            if db.gr(g).is_none() {
                eprintln!("usermod: group '{g}' does not exist");
                return 6;
            }
        }
        for l in db.group.iter_mut() {
            if let Line::Ent(gr) = l {
                let is_private = gr.name == name;
                if is_private {
                    continue;
                }
                let has = gr.members.contains(&name);
                let should = want.contains(&gr.name);
                if should && !has {
                    gr.members.push(newname.clone());
                } else if !should && has && !append {
                    gr.members.retain(|m| m != &name);
                }
            }
        }
        for l in db.gshadow.iter_mut() {
            if let Line::Ent(gs) = l {
                let has = gs.members.contains(&name);
                let should = want.contains(&gs.name);
                if should && !has {
                    gs.members.push(newname.clone());
                } else if !should && has && !append {
                    gs.members.retain(|m| m != &name);
                }
            }
        }
    }

    // -l renames and stays consistent: group memberships and the private group.
    if newname != name {
        for l in db.group.iter_mut() {
            if let Line::Ent(gr) = l {
                if gr.name == name && gr.gid == cur.uid {
                    gr.name = newname.clone();
                }
                for m in gr.members.iter_mut() {
                    if *m == name {
                        *m = newname.clone();
                    }
                }
            }
        }
        for l in db.gshadow.iter_mut() {
            if let Line::Ent(gs) = l {
                if gs.name == name {
                    gs.name = newname.clone();
                }
                for m in gs.members.iter_mut().chain(gs.admins.iter_mut()) {
                    if *m == name {
                        *m = newname.clone();
                    }
                }
            }
        }
    }

    // -m moves the home directory.
    if o.flags.iter().any(|f| f == "m") {
        let old = Path::new(&cur.home);
        let new = Path::new(&newp.home);
        if old.exists() && old != new {
            if new.exists() {
                eprintln!("usermod: directory {new:?} already exists");
                return 12;
            }
            if let Some(parent) = new.parent() {
                fs::create_dir_all(parent).ok();
            }
            if fs::rename(old, new).is_err() {
                eprintln!("usermod: cannot move {old:?} to {new:?}");
                return 12;
            }
        }
    }

    if let Err(e) = db.save() {
        eprintln!("usermod: {e}");
        return 1;
    }
    0
}

// ------------------------------------------------------------------ userdel --

fn cmd_userdel(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[("remove", 'r'), ("force", 'f'), ("root", 'R'), ("prefix", 'P')];
    let o = match parse_opts(args, "RP", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("userdel: {e}");
            return 2;
        }
    };
    if o.pos.is_empty() {
        eprintln!("usage: userdel [options] LOGIN");
        return 2;
    }
    let name = o.pos[0].clone();
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("userdel: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    let cur = match db.pw(&name) {
        Some(p) => p.clone(),
        None => {
            eprintln!("userdel: user '{name}' does not exist");
            return 6;
        }
    };
    db.passwd.retain(|l| !matches!(l, Line::Ent(p) if p.name == name));
    db.shadow.retain(|l| !matches!(l, Line::Ent(s) if s.name == name));
    let mut removed_group = None;
    db.group.retain(|l| match l {
        Line::Ent(g) if g.name == name && g.gid == cur.uid => {
            removed_group = Some(g.name.clone());
            false
        }
        Line::Ent(g) => {
            let _ = g;
            true
        }
        _ => true,
    });
    for l in db.group.iter_mut() {
        if let Line::Ent(g) = l {
            g.members.retain(|m| m != &name);
        }
    }
    if let Some(g) = &removed_group {
        db.gshadow.retain(|l| !matches!(l, Line::Ent(gs) if &gs.name == g));
    }
    for l in db.gshadow.iter_mut() {
        if let Line::Ent(gs) = l {
            gs.members.retain(|m| m != &name);
            gs.admins.retain(|m| m != &name);
        }
    }
    if let Err(e) = db.save() {
        eprintln!("userdel: {e}");
        return 1;
    }

    if o.flags.iter().any(|f| f == "r") {
        if cur.home != "/nonexistent" && cur.home != "/" {
            let p = Path::new(&cur.home);
            if p.exists() {
                if let Err(e) = fs::remove_dir_all(p) {
                    eprintln!("userdel: cannot remove home {}: {e}", cur.home);
                    return 12;
                }
            }
        }
        let mail = format!("/var/mail/{name}");
        if Path::new(&mail).exists() {
            fs::remove_file(&mail).ok();
        }
    }
    0
}

// ----------------------------------------------------------------- groupdel --

fn cmd_groupdel(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[("force", 'f'), ("root", 'R'), ("prefix", 'P')];
    let o = match parse_opts(args, "RP", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("groupdel: {e}");
            return 2;
        }
    };
    if o.pos.is_empty() {
        eprintln!("usage: groupdel [options] GROUP");
        return 2;
    }
    let name = o.pos[0].clone();
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("groupdel: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    let g = match db.gr(&name) {
        Some(g) => g.clone(),
        None => {
            eprintln!("groupdel: group '{name}' does not exist");
            return 6;
        }
    };
    if let Some(u) = db.users_pw().iter().find(|u| u.gid == g.gid) {
        eprintln!(
            "groupdel: cannot remove the primary group of user '{}'",
            u.name
        );
        return 8;
    }
    db.group.retain(|l| !matches!(l, Line::Ent(gr) if gr.name == name));
    db.gshadow.retain(|l| !matches!(l, Line::Ent(gs) if gs.name == name));
    if let Err(e) = db.save() {
        eprintln!("groupdel: {e}");
        return 10;
    }
    0
}

// ---------------------------------------------------------------- chpasswd --

fn cmd_chpasswd(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[("encrypted", 'e'), ("crypt-method", 'c'), ("md5", 'm'), ("root", 'R'), ("prefix", 'P')];
    let o = match parse_opts(args, "cRP", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("chpasswd: {e}");
            return 1;
        }
    };
    let encrypted = o.flags.iter().any(|f| f == "e");
    let method = if o.flags.iter().any(|f| f == "m") {
        "MD5".to_string()
    } else {
        o.vals.get("c").cloned().unwrap_or_else(|| "SHA512".into())
    };

    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("chpasswd: cannot read stdin");
        return 1;
    }
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("chpasswd: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    let mut rc = 0;
    for (idx, line) in input.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (user, pass) = match line.split_once(':') {
            Some(v) => v,
            None => {
                eprintln!("chpasswd: line {}: missing new password", idx + 1);
                rc = 1;
                continue;
            }
        };
        if db.pw(user).is_none() {
            eprintln!("chpasswd: line {}: user '{user}' does not exist", idx + 1);
            rc = 1;
            continue;
        }
        let hashed = if encrypted {
            pass.to_string()
        } else {
            match hash_password(pass, &method) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("chpasswd: {e}");
                    rc = 1;
                    continue;
                }
            }
        };
        let mut found = false;
        for l in db.shadow.iter_mut() {
            if let Line::Ent(s) = l {
                if s.name == user {
                    s.passwd = hashed.clone();
                    s.lastchg = today_days().to_string();
                    found = true;
                }
            }
        }
        if !found {
            db.shadow.push(Line::Ent(Sh {
                name: user.to_string(),
                passwd: hashed,
                lastchg: today_days().to_string(),
                min: "0".into(),
                max: "99999".into(),
                warn: "7".into(),
                inactive: String::new(),
                expire: String::new(),
                flag: String::new(),
            }));
        }
    }
    if let Err(e) = db.save() {
        eprintln!("chpasswd: {e}");
        return 1;
    }
    rc
}

// ------------------------------------------------------------------- passwd --

fn cmd_passwd(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[
        ("lock", 'l'),
        ("unlock", 'u'),
        ("delete", 'd'),
        ("status", 'S'),
        ("expire", 'e'),
        ("stdin", 's'),
        ("root", 'R'),
    ];
    let o = match parse_opts(args, "R", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("passwd: {e}");
            return 1;
        }
    };
    let name = o.pos.first().cloned().unwrap_or_else(|| "root".to_string());
    let lock = o.flags.iter().any(|f| f == "l");
    let unlock = o.flags.iter().any(|f| f == "u");
    let delete = o.flags.iter().any(|f| f == "d");
    let status = o.flags.iter().any(|f| f == "S");
    let expire = o.flags.iter().any(|f| f == "e");

    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("passwd: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    if db.pw(&name).is_none() {
        eprintln!("passwd: user '{name}' does not exist");
        return 1;
    }
    if status {
        if let Some(s) = db.sh(&name) {
            let state = if s.passwd.starts_with('!') {
                "L"
            } else if s.passwd.is_empty() {
                "NP"
            } else {
                "P"
            };
            println!("{name} {state} {}/{}", s.lastchg, s.max);
        }
        return 0;
    }
    let mut newhash = None;
    if !(lock || unlock || delete || expire) {
        let mut buf = String::new();
        if io::stdin().read_to_string(&mut buf).is_err() {
            eprintln!("passwd: cannot read stdin");
            return 1;
        }
        let pw = buf.lines().next().unwrap_or("").to_string();
        match hash_password(&pw, "SHA512") {
            Ok(h) => newhash = Some(h),
            Err(e) => {
                eprintln!("passwd: {e}");
                return 1;
            }
        }
    }
    let mut found = false;
    for l in db.shadow.iter_mut() {
        if let Line::Ent(s) = l {
            if s.name == name {
                found = true;
                if let Some(h) = &newhash {
                    s.passwd = h.clone();
                    s.lastchg = today_days().to_string();
                }
                if lock && !s.passwd.starts_with('!') {
                    s.passwd = format!("!{}", s.passwd);
                }
                if unlock && s.passwd.starts_with('!') {
                    s.passwd = s.passwd.trim_start_matches('!').to_string();
                }
                if delete {
                    s.passwd.clear();
                }
                if expire {
                    s.lastchg = "0".into();
                }
            }
        }
    }
    if !found {
        if let Some(h) = newhash {
            db.shadow.push(Line::Ent(Sh {
                name: name.clone(),
                passwd: h,
                lastchg: today_days().to_string(),
                min: "0".into(),
                max: "99999".into(),
                warn: "7".into(),
                inactive: String::new(),
                expire: String::new(),
                flag: String::new(),
            }));
        }
    }
    if let Err(e) = db.save() {
        eprintln!("passwd: {e}");
        return 1;
    }
    0
}

// --------------------------------------------------------------------- chfn --

fn gecos_parts(gecos: &str) -> Vec<String> {
    let mut v: Vec<String> = gecos.split(',').map(|s| s.to_string()).collect();
    while v.len() < 5 {
        v.push(String::new());
    }
    v.truncate(5);
    v
}

/// `chfn` exists here for one measured reason: Debian's `adduser` runs
/// `/bin/chfn -f "<gecos>" <user>` for `--gecos`, and the real chfn goes through
/// PAM, which answers "PAM: System error" in this domain, aborting adduser with
/// rc=82 *after* it has already created the account (half-done state, evidence
/// 111). Writing the field directly is the whole job.
fn cmd_chfn(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[
        ("full-name", 'f'),
        ("room-number", 'r'),
        ("work-phone", 'w'),
        ("home-phone", 'h'),
        ("other", 'o'),
        ("root", 'R'),
    ];
    let o = match parse_opts(args, "frwhoR", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("chfn: {e}");
            return 1;
        }
    };
    let name = o.pos.first().cloned().unwrap_or_else(|| "root".to_string());
    if o.vals.is_empty() {
        eprintln!("usage: chfn [-f full-name] [-r room] [-w work-phone] [-h home-phone] [-o other] [user]");
        return 1;
    }
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("chfn: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    if db.pw(&name).is_none() {
        eprintln!("chfn: user '{name}' does not exist");
        return 1;
    }
    for l in db.passwd.iter_mut() {
        if let Line::Ent(p) = l {
            if p.name == name {
                let mut g = gecos_parts(&p.gecos);
                if let Some(v) = o.vals.get("f") {
                    g[0] = v.clone();
                }
                if let Some(v) = o.vals.get("r") {
                    g[1] = v.clone();
                }
                if let Some(v) = o.vals.get("w") {
                    g[2] = v.clone();
                }
                if let Some(v) = o.vals.get("h") {
                    g[3] = v.clone();
                }
                if let Some(v) = o.vals.get("o") {
                    g[4] = v.clone();
                }
                let mut out = g.join(",");
                while out.ends_with(',') {
                    out.pop();
                }
                p.gecos = out;
            }
        }
    }
    if let Err(e) = db.save() {
        eprintln!("chfn: {e}");
        return 1;
    }
    0
}

// --------------------------------------------------------------------- chsh --

fn cmd_chsh(args: &[String]) -> i32 {
    let long: &[(&str, char)] = &[("shell", 's'), ("list-shells", 'l'), ("root", 'R')];
    let o = match parse_opts(args, "sR", long) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("chsh: {e}");
            return 1;
        }
    };
    if o.flags.iter().any(|f| f == "l") {
        if let Ok(defs) = fs::read_to_string("/etc/shells") {
            print!("{defs}");
        }
        return 0;
    }
    let shell = match o.vals.get("s") {
        Some(s) => s.clone(),
        None => {
            eprintln!("usage: chsh [-s shell] [user]");
            return 1;
        }
    };
    let name = o.pos.first().cloned().unwrap_or_else(|| "root".to_string());
    let ok = fs::read_to_string("/etc/shells")
        .map(|d| d.lines().any(|l| l.trim() == shell))
        .unwrap_or(false);
    if !ok {
        eprintln!("chsh: Warning: {shell} is not listed in /etc/shells.");
    }
    let _lock = match lock_pwdb() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("chsh: {e}");
            return 1;
        }
    };
    let mut db = Db::load();
    if db.pw(&name).is_none() {
        eprintln!("chsh: user '{name}' does not exist");
        return 1;
    }
    for l in db.passwd.iter_mut() {
        if let Line::Ent(p) = l {
            if p.name == name {
                p.shell = shell.clone();
            }
        }
    }
    if let Err(e) = db.save() {
        eprintln!("chsh: {e}");
        return 1;
    }
    0
}

// --------------------------------------------------------------------- main --

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if std::env::var_os("HUSK_USERMGR_DEBUG").is_some() {
        eprintln!("husk-usermgr: argv={argv:?}");
    }
    let argv0 = argv
        .first()
        .map(|p| {
            Path::new(p)
                .file_name()
                .map(|v| v.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone())
        })
        .unwrap_or_default();

    let (cmd, args): (String, Vec<String>) = match argv0.as_str() {
        "useradd" | "groupadd" | "usermod" | "userdel" | "groupdel" | "chpasswd" | "passwd"
        | "chfn" | "chsh" => (argv0.clone(), argv[1..].to_vec()),
        _ => {
            let cmd = argv.get(1).cloned().unwrap_or_default();
            (cmd, argv.get(2..).map(|s| s.to_vec()).unwrap_or_default())
        }
    };

    let code = match cmd.as_str() {
        "useradd" => cmd_useradd(&args),
        "groupadd" => cmd_groupadd(&args),
        "usermod" => cmd_usermod(&args),
        "userdel" => cmd_userdel(&args),
        "groupdel" => cmd_groupdel(&args),
        "chpasswd" => cmd_chpasswd(&args),
        "passwd" => cmd_passwd(&args),
        "chfn" => cmd_chfn(&args),
        "chsh" => cmd_chsh(&args),
        _ => {
            eprintln!("usage: husk-usermgr <useradd|groupadd|usermod|userdel|groupdel|chpasswd|passwd|chfn|chsh> ...");
            2
        }
    };
    std::process::exit(code);
}
