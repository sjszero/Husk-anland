use std::ffi::{c_char, c_int, c_long, c_void, CString};
use std::ptr;

#[repr(C)]
struct Ftw {
    base: c_int,
    level: c_int,
}

extern "C" {
    fn __xstat(version: c_int, path: *const c_char, stat: *mut libc::stat) -> c_int;
    fn __lxstat(version: c_int, path: *const c_char, stat: *mut libc::stat) -> c_int;
    fn __fxstatat(
        version: c_int,
        dirfd: c_int,
        path: *const c_char,
        stat: *mut libc::stat,
        flags: c_int,
    ) -> c_int;
    fn canonicalize_file_name(path: *const c_char) -> *mut c_char;
    fn nftw(
        path: *const c_char,
        callback: unsafe extern "C" fn(*const c_char, *const libc::stat, c_int, *mut Ftw) -> c_int,
        descriptors: c_int,
        flags: c_int,
    ) -> c_int;
    fn sem_open(name: *const c_char, oflag: c_int, ...) -> *mut c_void;
    fn sem_close(sem: *mut c_void) -> c_int;
    fn sem_unlink(name: *const c_char) -> c_int;
    fn execveat(
        dirfd: c_int,
        pathname: *const c_char,
        argv: *const *const c_char,
        envp: *const *const c_char,
        flags: c_int,
    ) -> c_int;
    fn chroot(path: *const c_char) -> c_int;
    fn umount(path: *const c_char) -> c_int;
    fn swapon(path: *const c_char, flags: c_int) -> c_int;
    fn swapoff(path: *const c_char) -> c_int;
}

fn cpath(value: &str) -> CString {
    CString::new(value).unwrap()
}

fn errno() -> c_int {
    unsafe { *libc::__errno_location() }
}

fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

fn rep(what: &str, rc: c_long, err: c_int) {
    if rc < 0 {
        unsafe {
            let message = std::ffi::CStr::from_ptr(libc::strerror(err));
            println!(
                "{:<26} FAIL errno={} ({})",
                what,
                err,
                message.to_string_lossy()
            );
        }
    } else {
        println!("{:<26} ok   rc={}", what, rc);
    }
}

unsafe extern "C" fn nftw_callback(
    path: *const c_char,
    _stat: *const libc::stat,
    _kind: c_int,
    _ftw: *mut Ftw,
) -> c_int {
    if !path.is_null() {
        NFTW_SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    0
}

static NFTW_SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn prepare_fixture() {
    let root = cpath("/root/gap-tmp");
    let file = cpath("/root/gap-tmp/c");
    unsafe {
        libc::unlink(file.as_ptr());
        libc::rmdir(root.as_ptr());
        libc::mkdir(root.as_ptr(), 0o755);
        let fd = libc::open(
            file.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644,
        );
        if fd >= 0 {
            libc::write(fd, b"hi\n".as_ptr().cast(), 3);
            libc::close(fd);
        }
    }
}

pub fn run() -> i32 {
    prepare_fixture();
    let c_file = cpath("/root/gap-tmp/c");
    let mut st: libc::stat = unsafe { std::mem::zeroed() };

    unsafe {
        clear_errno();
        let rc = __xstat(0, c_file.as_ptr(), &mut st);
        let err = errno();
        rep("__xstat", rc as c_long, err);
        if rc == 0 {
            println!("   size={}", st.st_size);
        }
        clear_errno();
        let rc = __lxstat(0, c_file.as_ptr(), &mut st);
        rep("__lxstat", rc as c_long, errno());
        clear_errno();
        let rc = __fxstatat(0, libc::AT_FDCWD, c_file.as_ptr(), &mut st, 0);
        rep("__fxstatat", rc as c_long, errno());

        let a = cpath("/root/gap-tmp/a");
        let b = cpath("/root/gap-tmp/b");
        let b2 = cpath("/root/gap-tmp/b2");
        let fd = libc::open(a.as_ptr(), libc::O_CREAT | libc::O_WRONLY, 0o644);
        if fd >= 0 {
            libc::close(fd);
        }
        clear_errno();
        let rc = libc::remove(a.as_ptr());
        rep("remove", rc as c_long, errno());

        let mode = b"w\0";
        let file = libc::fopen(b.as_ptr(), mode.as_ptr().cast());
        clear_errno();
        let reopened = if file.is_null() {
            ptr::null_mut()
        } else {
            libc::freopen(b2.as_ptr(), b"a\0".as_ptr().cast(), file)
        };
        rep("freopen", if reopened.is_null() { -1 } else { 0 }, errno());
        if !reopened.is_null() {
            libc::fclose(reopened);
        }

        clear_errno();
        let name_max = libc::pathconf(cpath("/root/gap-tmp").as_ptr(), libc::_PC_NAME_MAX);
        rep("pathconf", name_max, errno());

        let fd = libc::open(c_file.as_ptr(), libc::O_CREAT | libc::O_WRONLY, 0o644);
        if fd >= 0 {
            let data = [b'x'; 100];
            libc::write(fd, data.as_ptr().cast(), data.len());
            libc::close(fd);
        }
        clear_errno();
        let rc = libc::truncate(c_file.as_ptr(), 0);
        rep("truncate64", rc as c_long, errno());

        let shm_name = cpath("/husk-gap");
        clear_errno();
        let fd = libc::shm_open(shm_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
        let shm_errno = errno();
        rep("shm_open", fd as c_long, shm_errno);
        if fd >= 0 {
            libc::close(fd);
        }
        libc::shm_unlink(shm_name.as_ptr());

        let sem_name = cpath("/husk-gap");
        clear_errno();
        let sem = sem_open(sem_name.as_ptr(), libc::O_CREAT, 0o600u32, 1u32);
        let sem_errno = errno();
        let sem_failed = sem as isize == -1;
        rep("sem_open", if sem_failed { -1 } else { 0 }, sem_errno);
        if !sem_failed {
            sem_close(sem);
        }
        sem_unlink(sem_name.as_ptr());

        let libm = cpath("/usr/lib/aarch64-linux-gnu/libm.so.6");
        clear_errno();
        let handle = libc::dlmopen(libc::LM_ID_BASE, libm.as_ptr(), libc::RTLD_NOW);
        rep("dlmopen", if handle.is_null() { -1 } else { 0 }, errno());
        if !handle.is_null() {
            libc::dlclose(handle);
        }

        let true_path = cpath("/usr/bin/true");
        let argv = [true_path.as_ptr(), ptr::null()];
        let envp = [ptr::null()];
        let pid = libc::fork();
        if pid == 0 {
            execveat(
                libc::AT_FDCWD,
                true_path.as_ptr(),
                argv.as_ptr(),
                envp.as_ptr(),
                0,
            );
            libc::_exit(97);
        }
        let mut status = 0;
        if pid > 0 {
            libc::waitpid(pid, &mut status, 0);
        }
        let exit = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            -1
        };
        println!(
            "{:<26} child exit={}{}",
            "execveat(/usr/bin/true)",
            exit,
            if exit == 0 {
                "  (exec 成功)"
            } else {
                "  (没跑起来)"
            }
        );

        let pid = libc::fork();
        if pid == 0 {
            libc::syscall(
                libc::SYS_execveat,
                libc::AT_FDCWD,
                true_path.as_ptr(),
                argv.as_ptr(),
                envp.as_ptr(),
                0,
            );
            libc::_exit(97);
        }
        let mut status = 0;
        if pid > 0 {
            libc::waitpid(pid, &mut status, 0);
        }
        let exit = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            -1
        };
        println!(
            "{:<26} child exit={}  (raw syscall，设计边界)",
            "syscall(SYS_execveat)", exit
        );

        let mut real = [0u8; 4096];
        clear_errno();
        let resolved = libc::realpath(c_file.as_ptr(), real.as_mut_ptr());
        rep("realpath", if resolved.is_null() { -1 } else { 0 }, errno());
        if !resolved.is_null() {
            println!(
                "   realpath={}",
                std::ffi::CStr::from_ptr(resolved).to_string_lossy()
            );
        }

        clear_errno();
        let canonical = canonicalize_file_name(c_file.as_ptr());
        rep(
            "canonicalize_file_name",
            if canonical.is_null() { -1 } else { 0 },
            errno(),
        );
        if !canonical.is_null() {
            println!(
                "   canonical={}",
                std::ffi::CStr::from_ptr(canonical).to_string_lossy()
            );
            libc::free(canonical.cast());
        }

        let mut glob: libc::glob64_t = std::mem::zeroed();
        let pattern = cpath("/root/gap-tmp/*");
        clear_errno();
        let glob_rc = libc::glob64(pattern.as_ptr(), 0, None, &mut glob);
        rep("glob", glob_rc as c_long, errno());
        println!("   glob_count={}", glob.gl_pathc);
        libc::globfree64(&mut glob);

        NFTW_SEEN.store(0, std::sync::atomic::Ordering::Relaxed);
        clear_errno();
        let nftw_rc = nftw(
            cpath("/root/gap-tmp").as_ptr(),
            nftw_callback,
            8,
            1, // FTW_PHYS
        );
        rep("nftw", nftw_rc as c_long, errno());
        println!(
            "   nftw_seen={}",
            NFTW_SEEN.load(std::sync::atomic::Ordering::Relaxed)
        );

        let mut statvfs: libc::statvfs = std::mem::zeroed();
        clear_errno();
        let rc = libc::statvfs(cpath("/root").as_ptr(), &mut statvfs);
        rep("statvfs", rc as c_long, errno());
        println!(
            "   statvfs_bsize={} blocks={}",
            statvfs.f_bsize, statvfs.f_blocks
        );

        clear_errno();
        let rc = chroot(cpath("/").as_ptr());
        rep("chroot(/)", rc as c_long, errno());
        clear_errno();
        let rc = umount(cpath("/root/gap-tmp").as_ptr());
        rep("umount", rc as c_long, errno());
        clear_errno();
        let rc = swapon(c_file.as_ptr(), 0);
        rep("swapon", rc as c_long, errno());
        clear_errno();
        let rc = swapoff(c_file.as_ptr());
        rep("swapoff", rc as c_long, errno());
    }
    0
}
