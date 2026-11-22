// LinuxHub Native Runtime — canonical path resolver (no_std, no alloc).
// Resolves guest absolute paths against a rootfd, following symlinks the way
// chroot does: absolute link targets restart at the root, ".." never climbs
// above it. Only openat/readlinkat/close/dup are used, so it works inside an
// untrusted_app without openat2 (which the app seccomp policy kills).

#![no_std]

pub type CInt = i32;
pub type Ssize = isize;

pub const PATH_MAX: usize = 4096;
pub const EACCES: CInt = 13;
pub const ELOOP: CInt = 40;
pub const ENOTDIR: CInt = 20;
pub const AT_FDCWD: CInt = -100;
const O_PATH: CInt = 0o10000000;
const O_NOFOLLOW: CInt = 0o400000;
const O_DIRECTORY: CInt = 0o200000;
const O_CLOEXEC: CInt = 0o2000000;

const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_DUP: usize = 23;
const SYS_READLINKAT: usize = 78;

#[inline]
unsafe fn sc3(n: usize, a: usize, b: usize, c: usize) -> Ssize {
    let ret: Ssize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c,
        lateout("x0") ret, options(nostack));
    ret
}

#[inline]
unsafe fn sc4(n: usize, a: usize, b: usize, c: usize, d: usize) -> Ssize {
    let ret: Ssize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        lateout("x0") ret, options(nostack));
    ret
}

#[inline]
pub fn errno_of(r: Ssize) -> Option<CInt> {
    if r < 0 && r > -4096 { Some((-r) as CInt) } else { None }
}

pub unsafe fn openat(dirfd: CInt, path: &[u8], flags: CInt, mode: CInt) -> Result<CInt, CInt> {
    if path.is_empty() { return Err(2); }
    let mut buf = [0u8; PATH_MAX];
    buf[..path.len()].copy_from_slice(path);
    let r = sc4(SYS_OPENAT, dirfd as usize, buf.as_ptr() as usize, flags as usize, mode as usize);
    match errno_of(r) { Some(e) => Err(e), None => Ok(r as CInt) }
}

pub unsafe fn close(fd: CInt) { sc3(SYS_CLOSE, fd as usize, 0, 0); }

pub unsafe fn dup(fd: CInt) -> Result<CInt, CInt> {
    let r = sc3(SYS_DUP, fd as usize, 0, 0);
    match errno_of(r) { Some(e) => Err(e), None => Ok(r as CInt) }
}

unsafe fn readlinkat(dirfd: CInt, name: &[u8], out: &mut [u8]) -> Result<usize, CInt> {
    let mut nb = [0u8; 256];
    if name.len() >= nb.len() { return Err(36); }
    nb[..name.len()].copy_from_slice(name);
    let r = sc4(SYS_READLINKAT, dirfd as usize, nb.as_ptr() as usize, out.as_mut_ptr() as usize, out.len());
    match errno_of(r) { Some(e) => Err(e), None => Ok(r as usize) }
}

fn is_dot(c: &[u8]) -> bool { c == b"." }
fn is_dotdot(c: &[u8]) -> bool { c == b".." }

fn compose(a: &[u8], b: &[u8], out: &mut [u8]) -> Option<usize> {
    let n = a.len() + 1 + b.len();
    if n >= out.len() { return None; }
    out[..a.len()].copy_from_slice(a);
    out[a.len()] = b'/';
    out[a.len() + 1..n].copy_from_slice(b);
    Some(n)
}

/// Resolve an absolute guest path under `rootfd`.
/// Returns an owned directory fd and the final component copied into `name`.
pub unsafe fn resolve_parent(rootfd: CInt, path: &[u8], name: &mut [u8]) -> Result<CInt, CInt> {
    if path.first() != Some(&b'/') { return Err(EACCES); }
    resolve_at(rootfd, rootfd, path, name, 0, false)
}

/// Like `resolve_parent`, but also follows a symlink in the final component
/// (chroot semantics), which is what open/stat/access/exec need by default.
pub unsafe fn resolve_parent_follow(rootfd: CInt, path: &[u8], name: &mut [u8]) -> Result<CInt, CInt> {
    if path.first() != Some(&b'/') { return Err(EACCES); }
    resolve_at(rootfd, rootfd, path, name, 0, true)
}

fn norm(p: &[u8], out: &mut [u8; PATH_MAX]) -> usize {
    let mut n = 0usize;
    out[0] = b'/';
    for c in p.split(|x| *x == b'/').filter(|c| !c.is_empty() && *c != b".") {
        if c == b".." {
            n = pop_component(out, n);
            continue;
        }
        if n + 1 + c.len() >= PATH_MAX { break; }
        out[n] = b'/'; n += 1;
        out[n..n + c.len()].copy_from_slice(c); n += c.len();
    }
    if n == 0 { n = 1; out[0] = b'/'; }
    n
}

/// Drop the last component of an absolute path held in `out`, clamping at "/".
pub fn pop_component(out: &[u8; PATH_MAX], n: usize) -> usize {
    if n == 0 { return 1; }
    let mut k = n;
    while k > 0 && out[k - 1] != b'/' { k -= 1; }
    if k > 0 { k - 1 } else { 1 }
}

unsafe fn resolve_at(root: CInt, _base: CInt, path: &[u8], name: &mut [u8], depth: u32, follow: bool) -> Result<CInt, CInt> {
    if depth > 16 { return Err(ELOOP); }
    let mut cur = dup(root)?;
    let mut held: [CInt; 32] = [0; 32];
    let mut held_n = 0usize;
    let mut gpath = [0u8; PATH_MAX];
    let mut glen = 1usize;
    gpath[0] = b'/';
    let mut it = path.split(|c| *c == b'/').filter(|c| !c.is_empty()).peekable();
    while let Some(comp) = it.next() {
        let last = it.peek().is_none();
        if is_dot(comp) { continue; }
        if is_dotdot(comp) {
            if held_n > 0 { close(cur); held_n -= 1; cur = held[held_n]; }
            // pop one guest component (path stack mirrors the fd stack)
            glen = pop_component(&gpath, glen);
            continue;
        }
        let mut link = [0u8; 512];
        let n = readlinkat(cur, comp, &mut link).unwrap_or(0);
        if n > 0 && (!last || follow) {
            // Compose the new absolute guest path, then re-walk from the root.
            let target = &link[..n];
            let mut raw = [0u8; PATH_MAX];
            let mut rn = 0usize;
            if target[0] == b'/' {
                raw[..target.len()].copy_from_slice(target); rn = target.len();
            } else {
                if glen + 1 + target.len() >= PATH_MAX { return Err(36); }
                raw[..glen].copy_from_slice(&gpath[..glen]); rn = glen;
                raw[rn] = b'/'; rn += 1;
                raw[rn..rn + target.len()].copy_from_slice(target); rn += target.len();
            }
            for c in it.by_ref() {
                if rn + 1 + c.len() >= PATH_MAX { return Err(36); }
                raw[rn] = b'/'; rn += 1;
                raw[rn..rn + c.len()].copy_from_slice(c); rn += c.len();
            }
            let mut np = [0u8; PATH_MAX];
            let nn = norm(&raw[..rn], &mut np);
            for i in 0..held_n { close(held[i]); }
            close(cur);
            return resolve_at(root, root, &np[..nn], name, depth + 1, follow);
        }
        if last {
            if comp.len() >= name.len() { return Err(36); }
            name[..comp.len()].copy_from_slice(comp);
            for i in 0..held_n { close(held[i]); }
            return Ok(cur);
        }
        let fd = openat(cur, comp, O_PATH | O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC, 0)?;
        if held_n >= held.len() { close(fd); return Err(ELOOP); }
        held[held_n] = cur; held_n += 1;
        cur = fd;
        if glen + 1 + comp.len() >= PATH_MAX { return Err(36); }
        gpath[glen] = b'/'; glen += 1;
        gpath[glen..glen + comp.len()].copy_from_slice(comp); glen += comp.len();
    }
    if name.len() < 2 { return Err(36); }
    name[0] = b'.'; name[1] = 0;
    Ok(cur)
}

pub unsafe fn resolve_open(rootfd: CInt, path: &[u8], flags: CInt, mode: CInt) -> Result<CInt, CInt> {
    let mut name = [0u8; 256];
    let follow = flags & 0o400000 == 0;
    let parent = if follow { resolve_parent_follow(rootfd, path, &mut name)? } else { resolve_parent(rootfd, path, &mut name)? };
    let n = name.iter().position(|c| *c == 0).unwrap_or(0);
    let r = openat(parent, &name[..n], flags, mode);
    close(parent);
    r
}

pub unsafe fn resolve_fstat(rootfd: CInt, path: &[u8], st: *mut u8, flags: CInt) -> Result<(), CInt> {
    let mut name = [0u8; 256];
    let follow = flags & 0x100 == 0;
    let parent = if follow { resolve_parent_follow(rootfd, path, &mut name)? } else { resolve_parent(rootfd, path, &mut name)? };
    let n = name.iter().position(|c| *c == 0).unwrap_or(0);
    let mut nb = [0u8; 256];
    nb[..n].copy_from_slice(&name[..n]);
    const SYS_NEWFSTATAT: usize = 79;
    let r = sc4(SYS_NEWFSTATAT, parent as usize, nb.as_ptr() as usize, st as usize, flags as usize);
    close(parent);
    match errno_of(r) { Some(e) => Err(e), None => Ok(()) }
}

pub unsafe fn resolve_access(rootfd: CInt, path: &[u8], mode: CInt) -> Result<(), CInt> {
    let mut name = [0u8; 256];
    let parent = resolve_parent_follow(rootfd, path, &mut name)?;
    let n = name.iter().position(|c| *c == 0).unwrap_or(0);
    let mut nb = [0u8; 256];
    nb[..n].copy_from_slice(&name[..n]);
    const SYS_FACCESSAT: usize = 48;
    let r = sc4(SYS_FACCESSAT, parent as usize, nb.as_ptr() as usize, mode as usize, 0);
    close(parent);
    match errno_of(r) { Some(e) => Err(e), None => Ok(()) }
}
