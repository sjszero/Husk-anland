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
const SYS_FSTAT: usize = 80;
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
    if r < 0 && r > -4096 {
        Some((-r) as CInt)
    } else {
        None
    }
}

pub unsafe fn openat(dirfd: CInt, path: &[u8], flags: CInt, mode: CInt) -> Result<CInt, CInt> {
    if path.is_empty() {
        return Err(2);
    }
    if path.len() >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let mut buf = [0u8; PATH_MAX];
    buf[..path.len()].copy_from_slice(path);
    let r = sc4(
        SYS_OPENAT,
        dirfd as usize,
        buf.as_ptr() as usize,
        flags as usize,
        mode as usize,
    );
    match errno_of(r) {
        Some(e) => Err(e),
        None => Ok(r as CInt),
    }
}

pub unsafe fn close(fd: CInt) {
    sc3(SYS_CLOSE, fd as usize, 0, 0);
}

pub unsafe fn dup(fd: CInt) -> Result<CInt, CInt> {
    let r = sc3(SYS_DUP, fd as usize, 0, 0);
    match errno_of(r) {
        Some(e) => Err(e),
        None => Ok(r as CInt),
    }
}

unsafe fn readlinkat(dirfd: CInt, name: &[u8], out: &mut [u8]) -> Result<usize, CInt> {
    let mut nb = [0u8; 256];
    if name.len() >= nb.len() {
        return Err(36);
    }
    nb[..name.len()].copy_from_slice(name);
    let r = sc4(
        SYS_READLINKAT,
        dirfd as usize,
        nb.as_ptr() as usize,
        out.as_mut_ptr() as usize,
        out.len(),
    );
    match errno_of(r) {
        Some(e) => Err(e),
        None => Ok(r as usize),
    }
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
    if let Some(e) = errno_of(r) {
        return Err(e);
    }
    let mode = core::ptr::read_unaligned(st.as_ptr().add(16) as *const u32);
    Ok(mode & S_IFMT == S_IFLNK)
}

fn compose_symlink_target(
    dir: &[u8],
    target: &[u8],
    out: &mut [u8; PATH_MAX],
) -> Result<usize, CInt> {
    if target.first() == Some(&b'/') {
        if target.len() >= PATH_MAX {
            return Err(ENAMETOOLONG);
        }
        out[..target.len()].copy_from_slice(target);
        return Ok(target.len());
    }
    let n = dir.len() + 1 + target.len();
    if n >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    out[..dir.len()].copy_from_slice(dir);
    out[dir.len()] = b'/';
    out[dir.len() + 1..n].copy_from_slice(target);
    Ok(n)
}

fn is_dot(c: &[u8]) -> bool {
    c == b"."
}
fn is_dotdot(c: &[u8]) -> bool {
    c == b".."
}

/// Resolve an absolute guest path under `rootfd`.
/// Returns an owned directory fd and the final component copied into `name`.
pub unsafe fn resolve_parent(rootfd: CInt, path: &[u8], name: &mut [u8]) -> Result<CInt, CInt> {
    resolve_parent_impl(
        rootfd,
        path,
        name,
        false,
        core::ptr::null_mut(),
        core::ptr::null_mut(),
    )
}

/// Like `resolve_parent`, but also follows a symlink in the final component
/// (chroot semantics), which is what open/stat/access/exec need by default.
pub unsafe fn resolve_parent_follow(
    rootfd: CInt,
    path: &[u8],
    name: &mut [u8],
) -> Result<CInt, CInt> {
    resolve_parent_impl(
        rootfd,
        path,
        name,
        true,
        core::ptr::null_mut(),
        core::ptr::null_mut(),
    )
}

/// Resolve the parent of `path` and return its canonical guest directory.
/// The directory is needed when a final symlink target is relative and the
/// symlink itself must be resolved manually to avoid a follow race.
unsafe fn resolve_parent_with_dir(
    rootfd: CInt,
    path: &[u8],
    name: &mut [u8],
    follow: bool,
) -> Result<(CInt, [u8; PATH_MAX], usize), CInt> {
    let mut dir = [0u8; PATH_MAX];
    let mut dir_len = 0usize;
    let fd = resolve_parent_impl(rootfd, path, name, follow, dir.as_mut_ptr(), &mut dir_len)?;
    Ok((fd, dir, dir_len))
}

unsafe fn resolve_parent_impl(
    rootfd: CInt,
    path: &[u8],
    name: &mut [u8],
    follow: bool,
    dir_buf: *mut u8,
    dir_len: *mut usize,
) -> Result<CInt, CInt> {
    if path.first() != Some(&b'/') {
        return Err(EACCES);
    }
    if path.len() >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    resolve_at(rootfd, rootfd, path, name, 0, follow, dir_buf, dir_len)
}

fn norm(p: &[u8], out: &mut [u8; PATH_MAX]) -> usize {
    let mut n = 0usize;
    out[0] = b'/';
    for c in p
        .split(|x| *x == b'/')
        .filter(|c| !c.is_empty() && *c != b".")
    {
        if c == b".." {
            n = pop_component(out, n);
            continue;
        }
        if n + 1 + c.len() >= PATH_MAX {
            break;
        }
        out[n] = b'/';
        n += 1;
        out[n..n + c.len()].copy_from_slice(c);
        n += c.len();
    }
    if n == 0 {
        n = 1;
        out[0] = b'/';
    }
    n
}

/// Drop the last component of an absolute path held in `out`, clamping at "/".
pub fn pop_component(out: &[u8; PATH_MAX], n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut k = n;
    while k > 0 && out[k - 1] != b'/' {
        k -= 1;
    }
    if k > 0 {
        k - 1
    } else {
        1
    }
}

struct Walk {
    cur: CInt,
}

impl Walk {
    unsafe fn new(root: CInt) -> Result<Self, CInt> {
        Ok(Self { cur: dup(root)? })
    }

    unsafe fn replace(&mut self, fd: CInt) {
        if self.cur >= 0 {
            close(self.cur);
        }
        self.cur = fd;
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
            if self.cur >= 0 {
                close(self.cur);
            }
        }
    }
}

/// Maximum number of symlinks Linux will follow while resolving one pathname
/// (`MAXSYMLINKS` / 40). The 41st link fails with `ELOOP`.
pub const LINK_MAX: u32 = 40;

/// Walk an absolute guest path from `root`, following symlinks with chroot
/// semantics: an absolute target restarts at the root, and the host `/` is
/// never reachable.
///
/// The walk is iterative on purpose. It used to recurse once per symlink and
/// every recursion frame carried four `PATH_MAX` (4 KiB) buffers plus a
/// `Walk`, so a 40-link chain needed roughly 640 KiB of stack — more than a
/// guest thread with a 64 KiB pthread stack can pay, which turns "raise the
/// limit to 40" into "trade ELOOP for SIGSEGV". All buffers now live in this
/// single frame and only one directory fd is alive at a time, so the stack
/// cost is constant regardless of link depth.
unsafe fn resolve_at(
    root: CInt,
    _base: CInt,
    path: &[u8],
    name: &mut [u8],
    depth: u32,
    follow: bool,
    dir_buf: *mut u8,
    dir_len: *mut usize,
) -> Result<CInt, CInt> {
    if depth >= LINK_MAX {
        return Err(ELOOP);
    }
    if path.len() >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    // `cur` is the path being walked. A symlink replaces it with the composed
    // target plus the components that had not been consumed yet.
    let mut cur = [0u8; PATH_MAX];
    let mut next = [0u8; PATH_MAX];
    let mut normed = [0u8; PATH_MAX];
    let mut gpath = [0u8; PATH_MAX];
    let mut link = [0u8; PATH_MAX];
    let mut cur_len = path.len();
    cur[..cur_len].copy_from_slice(path);
    let mut depth = depth;
    'outer: loop {
        let mut walk = Walk::new(root)?;
        let mut glen = 1usize;
        gpath[0] = b'/';
        let mut pos = 0usize;
        loop {
            while pos < cur_len && cur[pos] == b'/' {
                pos += 1;
            }
            if pos >= cur_len {
                break;
            }
            let start = pos;
            while pos < cur_len && cur[pos] != b'/' {
                pos += 1;
            }
            let comp = &cur[start..pos];
            // Where the next component starts, which also decides whether
            // `comp` is the final one (trailing slashes do not count).
            let mut after = pos;
            while after < cur_len && cur[after] == b'/' {
                after += 1;
            }
            let last = after >= cur_len;
            if is_dot(comp) {
                pos = after;
                continue;
            }
            if is_dotdot(comp) {
                // `..` is opened from the current directory descriptor; the guest
                // path stack decides when we are at the root and must clamp. This
                // avoids keeping one fd per component (the old fixed 32-fd stack
                // rejected valid paths sent by build tools and package managers).
                if glen > 1 {
                    let fd = openat(walk.cur, b"..", O_PATH | O_DIRECTORY | O_CLOEXEC, 0)?;
                    walk.replace(fd);
                    glen = pop_component(&gpath, glen);
                }
                pos = after;
                continue;
            }
            let n = readlinkat(walk.cur, comp, &mut link).unwrap_or(0);
            if n == link.len() {
                return Err(ENAMETOOLONG);
            }
            if n > 0 && (!last || follow) {
                // Compose the new absolute guest path, then restart the walk
                // from the root with it. This is the step that used to be a
                // recursive call.
                if depth >= LINK_MAX {
                    return Err(ELOOP);
                }
                let target = &link[..n];
                let mut rn;
                if target[0] == b'/' {
                    if target.len() >= PATH_MAX {
                        return Err(ENAMETOOLONG);
                    }
                    next[..target.len()].copy_from_slice(target);
                    rn = target.len();
                } else {
                    if glen + 1 + target.len() >= PATH_MAX {
                        return Err(ENAMETOOLONG);
                    }
                    next[..glen].copy_from_slice(&gpath[..glen]);
                    rn = glen;
                    next[rn] = b'/';
                    rn += 1;
                    next[rn..rn + target.len()].copy_from_slice(target);
                    rn += target.len();
                }
                if !last {
                    let rest = &cur[after..cur_len];
                    if rn + 1 + rest.len() >= PATH_MAX {
                        return Err(ENAMETOOLONG);
                    }
                    next[rn] = b'/';
                    rn += 1;
                    next[rn..rn + rest.len()].copy_from_slice(rest);
                    rn += rest.len();
                }
                let nn = norm(&next[..rn], &mut normed);
                cur_len = nn;
                cur[..nn].copy_from_slice(&normed[..nn]);
                depth += 1;
                drop(walk);
                continue 'outer;
            }
            if last {
                if comp.len() >= name.len() {
                    return Err(ENAMETOOLONG);
                }
                name[..comp.len()].copy_from_slice(comp);
                if !dir_buf.is_null() && !dir_len.is_null() {
                    core::ptr::copy_nonoverlapping(gpath.as_ptr(), dir_buf, glen);
                    *dir_len = glen;
                }
                return Ok(walk.finish());
            }
            let fd = openat(
                walk.cur,
                comp,
                O_PATH | O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC,
                0,
            )?;
            walk.replace(fd);
            if glen + 1 + comp.len() >= PATH_MAX {
                return Err(ENAMETOOLONG);
            }
            gpath[glen] = b'/';
            glen += 1;
            gpath[glen..glen + comp.len()].copy_from_slice(comp);
            glen += comp.len();
            pos = after;
        }
        if name.len() < 2 {
            return Err(ENAMETOOLONG);
        }
        name[0] = b'.';
        name[1] = 0;
        if !dir_buf.is_null() && !dir_len.is_null() {
            core::ptr::copy_nonoverlapping(gpath.as_ptr(), dir_buf, glen);
            *dir_len = glen;
        }
        return Ok(walk.finish());
    }
}

pub unsafe fn resolve_open(
    rootfd: CInt,
    path: &[u8],
    flags: CInt,
    mode: CInt,
) -> Result<CInt, CInt> {
    if path.len() >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let follow = flags & O_NOFOLLOW == 0;
    let mut current = [0u8; PATH_MAX];
    current[..path.len()].copy_from_slice(path);
    let mut current_len = path.len();
    let mut depth = 0u32;
    loop {
        let mut name = [0u8; 256];
        let (parent, dir, dir_len) =
            resolve_parent_with_dir(rootfd, &current[..current_len], &mut name, false)?;
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);
        let open_flags = if follow { flags | O_NOFOLLOW } else { flags };
        let mut result = openat(parent, &name[..n], open_flags, mode);
        if follow && flags & O_PATH != 0 {
            if let Ok(fd) = result {
                match fd_is_symlink(fd) {
                    Ok(true) => {
                        close(fd);
                        result = Err(ELOOP);
                    }
                    Ok(false) => {}
                    Err(e) => {
                        close(fd);
                        result = Err(e);
                    }
                }
            }
        }
        match result {
            Ok(fd) => {
                close(parent);
                return Ok(fd);
            }
            Err(e) if follow && (e == ELOOP || e == ENOTDIR) => {
                let mut link = [0u8; PATH_MAX];
                let r = readlinkat(parent, &name[..n], &mut link);
                close(parent);
                let link_len = match r {
                    Ok(v) if v < link.len() => v,
                    Ok(_) => return Err(ENAMETOOLONG),
                    Err(22) => {
                        if depth >= LINK_MAX {
                            return Err(ELOOP);
                        }
                        depth += 1;
                        continue;
                    }
                    Err(_) => return Err(e),
                };
                if depth >= LINK_MAX {
                    return Err(ELOOP);
                }
                current_len =
                    compose_symlink_target(&dir[..dir_len], &link[..link_len], &mut current)?;
                depth += 1;
            }
            Err(e) => {
                close(parent);
                return Err(e);
            }
        }
    }
}

/// Open the final object named by an absolute guest path and pin it with an
/// `O_PATH` descriptor. Symlinks are resolved manually under `rootfd`, so a
/// concurrent replacement can change which legitimate guest object is picked,
/// but cannot redirect the operation outside the guest root. Callers that need
/// metadata semantics use this descriptor instead of a parentfd + final-name
/// pair, which closes the final-component replacement race on this kernel.
pub unsafe fn resolve_final(rootfd: CInt, path: &[u8], follow: bool) -> Result<CInt, CInt> {
    if path.first() != Some(&b'/') {
        return Err(EACCES);
    }
    if path.len() >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let force_dir = path.len() > 1 && path.last() == Some(&b'/');
    let mut current = [0u8; PATH_MAX];
    current[..path.len()].copy_from_slice(path);
    let mut current_len = path.len();
    let mut depth = 0u32;
    loop {
        let mut name = [0u8; 256];
        let (parent, dir, dir_len) =
            resolve_parent_with_dir(rootfd, &current[..current_len], &mut name, false)?;
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);

        let mut open_flags = O_PATH | O_NOFOLLOW | O_CLOEXEC;
        if force_dir {
            open_flags |= O_DIRECTORY;
        }
        let fd = match openat(parent, &name[..n], open_flags, 0) {
            Ok(fd) => fd,
            Err(e) => {
                close(parent);
                return Err(e);
            }
        };
        let is_link = match fd_is_symlink(fd) {
            Ok(v) => v,
            Err(e) => {
                close(fd);
                close(parent);
                return Err(e);
            }
        };
        if !follow || !is_link {
            close(parent);
            return Ok(fd);
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
        if depth >= LINK_MAX {
            return Err(ELOOP);
        }
        current_len = compose_symlink_target(&dir[..dir_len], &link[..link_len], &mut current)?;
        depth += 1;
    }
}

pub unsafe fn resolve_fstat(
    rootfd: CInt,
    path: &[u8],
    st: *mut u8,
    flags: CInt,
) -> Result<(), CInt> {
    let follow = flags & 0x100 == 0;
    let fd = resolve_final(rootfd, path, follow)?;
    let r = sc3(SYS_FSTAT, fd as usize, st as usize, 0);
    close(fd);
    match errno_of(r) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Check access through an already-open descriptor. `access()` itself takes a
/// path, so the descriptor is addressed through `/proc/self/fd/N`; the fd pins
/// the inode and prevents a final-component replacement race.
pub unsafe fn access_fd(fd: CInt, mode: CInt, nofollow: bool) -> Result<(), CInt> {
    if fd < 0 {
        return Err(9);
    } // EBADF
    if mode & !7 != 0 {
        return Err(22);
    } // EINVAL
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
    let r = sc4(
        SYS_FACCESSAT,
        AT_FDCWD as usize,
        path.as_ptr() as usize,
        mode as usize,
        0,
    );
    match errno_of(r) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

pub unsafe fn resolve_access(
    rootfd: CInt,
    path: &[u8],
    mode: CInt,
    flags: CInt,
) -> Result<(), CInt> {
    if path.len() >= PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let nofollow = flags & AT_SYMLINK_NOFOLLOW != 0;
    let mut current = [0u8; PATH_MAX];
    current[..path.len()].copy_from_slice(path);
    let mut current_len = path.len();
    let mut depth = 0u32;
    loop {
        let mut name = [0u8; 256];
        let (parent, dir, dir_len) =
            resolve_parent_with_dir(rootfd, &current[..current_len], &mut name, false)?;
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);
        let fd = match openat(parent, &name[..n], O_PATH | O_NOFOLLOW | O_CLOEXEC, 0) {
            Ok(fd) => fd,
            Err(e) => {
                close(parent);
                return Err(e);
            }
        };
        let is_link = match fd_is_symlink(fd) {
            Ok(v) => v,
            Err(e) => {
                close(fd);
                close(parent);
                return Err(e);
            }
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
        if depth >= LINK_MAX {
            return Err(ELOOP);
        }
        current_len = compose_symlink_target(&dir[..dir_len], &link[..link_len], &mut current)?;
        depth += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{compose_symlink_target, errno_of, norm, pop_component, ENAMETOOLONG, PATH_MAX};

    #[test]
    fn errno_classification_is_bounded_to_linux_error_range() {
        assert_eq!(errno_of(-1), Some(1));
        assert_eq!(errno_of(-4095), Some(4095));
        assert_eq!(errno_of(-4096), None);
        assert_eq!(errno_of(0), None);
        assert_eq!(errno_of(7), None);
    }

    #[test]
    fn absolute_symlink_target_restarts_at_root() {
        let mut out = [0u8; PATH_MAX];
        let n = compose_symlink_target(b"/a/b", b"/etc/passwd", &mut out).unwrap();
        assert_eq!(&out[..n], b"/etc/passwd");
    }

    #[test]
    fn relative_symlink_target_is_joined_to_parent() {
        let mut out = [0u8; PATH_MAX];
        let n = compose_symlink_target(b"/a/b", b"../c", &mut out).unwrap();
        assert_eq!(&out[..n], b"/a/b/../c");
    }

    #[test]
    fn overlong_paths_are_rejected_without_slicing() {
        let mut out = [0u8; PATH_MAX];
        let target = [b'x'; PATH_MAX];
        assert_eq!(
            compose_symlink_target(b"/", &target, &mut out),
            Err(ENAMETOOLONG)
        );
    }

    #[test]
    fn normalization_removes_dot_empty_and_clamps_parent() {
        let mut out = [0u8; PATH_MAX];
        let n = norm(b"/a/./b//../../../../c", &mut out);
        assert_eq!(&out[..n], b"/c");
    }

    #[test]
    fn pop_component_clamps_at_root() {
        let mut out = [0u8; PATH_MAX];
        out[0] = b'/';
        out[1] = b'a';
        out[2] = b'/';
        out[3] = b'b';
        assert_eq!(pop_component(&out, 4), 2);
        assert_eq!(pop_component(&out, 2), 0);
        assert_eq!(pop_component(&out, 0), 1);
    }
}
