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
pub const ENAMETOOLONG: CInt = 36;
pub const AT_FDCWD: CInt = -100;
const O_PATH: CInt = 0o10000000;
// AArch64 kernel UAPI values. These differ from the asm-generic defaults;
// using the generic numbers here silently turns off no-follow/directory checks.
const O_NOFOLLOW: CInt = 0o100000;
const O_DIRECTORY: CInt = 0o40000;
const O_CLOEXEC: CInt = 0o2000000;

const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_DUP: usize = 23;
const SYS_READLINKAT: usize = 78;
const SYS_NEWFSTATAT: usize = 79;
const AT_EMPTY_PATH: CInt = 0x1000;
const AT_SYMLINK_NOFOLLOW: CInt = 0x100;
const SYS_FACCESSAT: usize = 48;
const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;

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
    if path.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
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

unsafe fn fd_is_symlink(fd: CInt) -> Result<bool, CInt> {
    let mut st = [0u8; 128];
    let r = sc4(
        SYS_NEWFSTATAT,
        fd as usize,
        b"\0".as_ptr() as usize,
        st.as_mut_ptr() as usize,
        AT_EMPTY_PATH as usize,
    );
    if let Some(e) = errno_of(r) { return Err(e); }
    let mode = core::ptr::read_unaligned(st.as_ptr().add(16) as *const u32);
    Ok(mode & S_IFMT == S_IFLNK)
}

unsafe fn stat_is_symlink(st: *const u8) -> bool {
    (core::ptr::read_unaligned(st.add(16) as *const u32) & S_IFMT) == S_IFLNK
}

fn compose_symlink_target(dir: &[u8], target: &[u8], out: &mut [u8; PATH_MAX])
    -> Result<usize, CInt>
{
    if target.first() == Some(&b'/') {
        if target.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
        out[..target.len()].copy_from_slice(target);
        return Ok(target.len());
    }
    let n = dir.len() + 1 + target.len();
    if n >= PATH_MAX { return Err(ENAMETOOLONG); }
    out[..dir.len()].copy_from_slice(dir);
    out[dir.len()] = b'/';
    out[dir.len() + 1..n].copy_from_slice(target);
    Ok(n)
}

fn is_dot(c: &[u8]) -> bool { c == b"." }
fn is_dotdot(c: &[u8]) -> bool { c == b".." }

/// Resolve an absolute guest path under `rootfd`.
/// Returns an owned directory fd and the final component copied into `name`.
pub unsafe fn resolve_parent(rootfd: CInt, path: &[u8], name: &mut [u8]) -> Result<CInt, CInt> {
    resolve_parent_impl(rootfd, path, name, false, core::ptr::null_mut(), core::ptr::null_mut())
}

/// Like `resolve_parent`, but also follows a symlink in the final component
/// (chroot semantics), which is what open/stat/access/exec need by default.
pub unsafe fn resolve_parent_follow(rootfd: CInt, path: &[u8], name: &mut [u8]) -> Result<CInt, CInt> {
    resolve_parent_impl(rootfd, path, name, true, core::ptr::null_mut(), core::ptr::null_mut())
}

/// Resolve the parent of `path` and return its canonical guest directory.
/// The directory is needed when a final symlink target is relative and the
/// symlink itself must be resolved manually to avoid a follow race.
unsafe fn resolve_parent_with_dir(rootfd: CInt, path: &[u8], name: &mut [u8], follow: bool)
    -> Result<(CInt, [u8; PATH_MAX], usize), CInt>
{
    let mut dir = [0u8; PATH_MAX];
    let mut dir_len = 0usize;
    let fd = resolve_parent_impl(rootfd, path, name, follow, dir.as_mut_ptr(), &mut dir_len)?;
    Ok((fd, dir, dir_len))
}

unsafe fn resolve_parent_impl(rootfd: CInt, path: &[u8], name: &mut [u8], follow: bool,
                              dir_buf: *mut u8, dir_len: *mut usize) -> Result<CInt, CInt> {
    if path.first() != Some(&b'/') { return Err(EACCES); }
    if path.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
    resolve_at(rootfd, rootfd, path, name, 0, follow, dir_buf, dir_len)
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

struct Walk {
    cur: CInt,
    held: [CInt; 32],
    held_n: usize,
}

impl Walk {
    unsafe fn new(root: CInt) -> Result<Self, CInt> {
        Ok(Self { cur: dup(root)?, held: [0; 32], held_n: 0 })
    }

    unsafe fn pop(&mut self) {
        if self.held_n > 0 {
            close(self.cur);
            self.held_n -= 1;
            self.cur = self.held[self.held_n];
        }
    }

    unsafe fn finish(mut self) -> CInt {
        let fd = self.cur;
        self.cur = -1;
        fd
    }
}

impl Drop for Walk {
    fn drop(&mut self) {
        unsafe {
            if self.cur >= 0 { close(self.cur); }
            let mut i = 0;
            while i < self.held_n { close(self.held[i]); i += 1; }
        }
    }
}

unsafe fn resolve_at(root: CInt, _base: CInt, path: &[u8], name: &mut [u8], depth: u32,
                     follow: bool, dir_buf: *mut u8, dir_len: *mut usize) -> Result<CInt, CInt> {
    if depth > 16 { return Err(ELOOP); }
    let mut walk = Walk::new(root)?;
    let mut gpath = [0u8; PATH_MAX];
    let mut glen = 1usize;
    gpath[0] = b'/';
    let mut it = path.split(|c| *c == b'/').filter(|c| !c.is_empty()).peekable();
    while let Some(comp) = it.next() {
        let last = it.peek().is_none();
        if is_dot(comp) { continue; }
        if is_dotdot(comp) {
            walk.pop();
            // pop one guest component (path stack mirrors the fd stack)
            glen = pop_component(&gpath, glen);
            continue;
        }
        let mut link = [0u8; PATH_MAX];
        let n = readlinkat(walk.cur, comp, &mut link).unwrap_or(0);
        if n == link.len() { return Err(ENAMETOOLONG); }
        if n > 0 && (!last || follow) {
            // Compose the new absolute guest path, then re-walk from the root.
            let target = &link[..n];
            let mut raw = [0u8; PATH_MAX];
            let mut rn = if target[0] == b'/' {
                if target.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
                raw[..target.len()].copy_from_slice(target);
                target.len()
            } else {
                if glen + 1 + target.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
                raw[..glen].copy_from_slice(&gpath[..glen]);
                let mut rn = glen;
                raw[rn] = b'/'; rn += 1;
                raw[rn..rn + target.len()].copy_from_slice(target); rn += target.len();
                rn
            };
            for c in it.by_ref() {
                if rn + 1 + c.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
                raw[rn] = b'/'; rn += 1;
                raw[rn..rn + c.len()].copy_from_slice(c); rn += c.len();
            }
            let mut np = [0u8; PATH_MAX];
            let nn = norm(&raw[..rn], &mut np);
            drop(walk);
            return resolve_at(root, root, &np[..nn], name, depth + 1, follow, dir_buf, dir_len);
        }
        if last {
            if comp.len() >= name.len() { return Err(ENAMETOOLONG); }
            name[..comp.len()].copy_from_slice(comp);
            if !dir_buf.is_null() && !dir_len.is_null() {
                core::ptr::copy_nonoverlapping(gpath.as_ptr(), dir_buf, glen);
                *dir_len = glen;
            }
            return Ok(walk.finish());
        }
        let fd = openat(walk.cur, comp, O_PATH | O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC, 0)?;
        if walk.held_n >= walk.held.len() { close(fd); return Err(ELOOP); }
        walk.held[walk.held_n] = walk.cur; walk.held_n += 1;
        walk.cur = fd;
        if glen + 1 + comp.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
        gpath[glen] = b'/'; glen += 1;
        gpath[glen..glen + comp.len()].copy_from_slice(comp); glen += comp.len();
    }
    if name.len() < 2 { return Err(ENAMETOOLONG); }
    name[0] = b'.'; name[1] = 0;
    if !dir_buf.is_null() && !dir_len.is_null() {
        core::ptr::copy_nonoverlapping(gpath.as_ptr(), dir_buf, glen);
        *dir_len = glen;
    }
    Ok(walk.finish())
}

pub unsafe fn resolve_open(rootfd: CInt, path: &[u8], flags: CInt, mode: CInt) -> Result<CInt, CInt> {
    if path.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
    let follow = flags & O_NOFOLLOW == 0;
    let mut current = [0u8; PATH_MAX];
    current[..path.len()].copy_from_slice(path);
    let mut current_len = path.len();
    let mut depth = 0u32;
    loop {
        if depth > 16 { return Err(ELOOP); }
        let mut name = [0u8; 256];
        let (parent, dir, dir_len) = resolve_parent_with_dir(rootfd, &current[..current_len], &mut name, false)?;
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);
        let open_flags = if follow { flags | O_NOFOLLOW } else { flags };
        let mut result = openat(parent, &name[..n], open_flags, mode);
        if follow && flags & O_PATH != 0 {
            if let Ok(fd) = result {
                match fd_is_symlink(fd) {
                    Ok(true) => { close(fd); result = Err(ELOOP); }
                    Ok(false) => {}
                    Err(e) => { close(fd); result = Err(e); }
                }
            }
        }
        match result {
            Ok(fd) => { close(parent); return Ok(fd); }
            Err(e) if follow && (e == ELOOP || e == ENOTDIR) => {
                let mut link = [0u8; PATH_MAX];
                let r = readlinkat(parent, &name[..n], &mut link);
                close(parent);
                let link_len = match r {
                    Ok(v) if v < link.len() => v,
                    Ok(_) => return Err(ENAMETOOLONG),
                    Err(22) => { depth += 1; continue; }
                    Err(_) => return Err(e),
                };
                current_len = compose_symlink_target(&dir[..dir_len], &link[..link_len], &mut current)?;
                depth += 1;
            }
            Err(e) => { close(parent); return Err(e); }
        }
    }
}

pub unsafe fn resolve_fstat(rootfd: CInt, path: &[u8], st: *mut u8, flags: CInt) -> Result<(), CInt> {
    if path.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
    let follow = flags & 0x100 == 0;
    let mut current = [0u8; PATH_MAX];
    current[..path.len()].copy_from_slice(path);
    let mut current_len = path.len();
    let mut depth = 0u32;
    loop {
        if depth > 16 { return Err(ELOOP); }
        let mut name = [0u8; 256];
        let (parent, dir, dir_len) = resolve_parent_with_dir(rootfd, &current[..current_len], &mut name, false)?;
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);
        let mut nb = [0u8; 256];
        nb[..n].copy_from_slice(&name[..n]);
        let stat_flags = if follow { flags | 0x100 } else { flags };
        let r = sc4(SYS_NEWFSTATAT, parent as usize, nb.as_ptr() as usize, st as usize, stat_flags as usize);
        if r == 0 && follow && stat_is_symlink(st) {
            let mut link = [0u8; PATH_MAX];
            let lr = readlinkat(parent, &name[..n], &mut link);
            close(parent);
            let link_len = match lr {
                Ok(v) if v < link.len() => v,
                Ok(_) => return Err(ENAMETOOLONG),
                Err(22) => { depth += 1; continue; }
                Err(e) => return Err(e),
            };
            current_len = compose_symlink_target(&dir[..dir_len], &link[..link_len], &mut current)?;
            depth += 1;
            continue;
        }
        close(parent);
        return match errno_of(r) { Some(e) => Err(e), None => Ok(()) };
    }
}

/// Check access through an already-open descriptor. `access()` itself takes a
/// path, so the descriptor is addressed through `/proc/self/fd/N`; the fd pins
/// the inode and prevents a final-component replacement race.
pub unsafe fn access_fd(fd: CInt, mode: CInt, nofollow: bool) -> Result<(), CInt> {
    if fd < 0 { return Err(9); } // EBADF
    if mode & !7 != 0 { return Err(22); } // EINVAL
    if nofollow && fd_is_symlink(fd)? {
        // A symlink inode is always 0777 on Linux. Its target is deliberately
        // not consulted under AT_SYMLINK_NOFOLLOW.
        return Ok(());
    }
    let mut path = [0u8; 32];
    let prefix = b"/proc/self/fd/";
    path[..prefix.len()].copy_from_slice(prefix);
    let mut at = prefix.len();
    let mut digits = [0u8; 10];
    let mut n = 0usize;
    let mut v = fd as u32;
    if v == 0 {
        digits[0] = b'0';
        n = 1;
    } else {
        while v != 0 {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
    }
    while n > 0 {
        n -= 1;
        path[at] = digits[n];
        at += 1;
    }
    path[at] = 0;
    let r = sc4(SYS_FACCESSAT, AT_FDCWD as usize, path.as_ptr() as usize, mode as usize, 0);
    match errno_of(r) { Some(e) => Err(e), None => Ok(()) }
}

pub unsafe fn resolve_access(rootfd: CInt, path: &[u8], mode: CInt, flags: CInt) -> Result<(), CInt> {
    if path.len() >= PATH_MAX { return Err(ENAMETOOLONG); }
    let nofollow = flags & AT_SYMLINK_NOFOLLOW != 0;
    let mut current = [0u8; PATH_MAX];
    current[..path.len()].copy_from_slice(path);
    let mut current_len = path.len();
    let mut depth = 0u32;
    loop {
        if depth > 16 { return Err(ELOOP); }
        let mut name = [0u8; 256];
        let (parent, dir, dir_len) = resolve_parent_with_dir(rootfd, &current[..current_len], &mut name, false)?;
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);
        let fd = match openat(parent, &name[..n], O_PATH | O_NOFOLLOW | O_CLOEXEC, 0) {
            Ok(fd) => fd,
            Err(e) => { close(parent); return Err(e); }
        };
        let is_link = match fd_is_symlink(fd) {
            Ok(v) => v,
            Err(e) => { close(fd); close(parent); return Err(e); }
        };
        if nofollow || !is_link {
            let r = access_fd(fd, mode, nofollow);
            close(fd);
            close(parent);
            return r;
        }
        let mut link = [0u8; PATH_MAX];
        let r = readlinkat(parent, &name[..n], &mut link);
        close(fd);
        close(parent);
        let link_len = match r {
            Ok(v) if v < link.len() => v,
            Ok(_) => return Err(ENAMETOOLONG),
            Err(e) => return Err(e),
        };
        current_len = compose_symlink_target(&dir[..dir_len], &link[..link_len], &mut current)?;
        depth += 1;
    }
}
