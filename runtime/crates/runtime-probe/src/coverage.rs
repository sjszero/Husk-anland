use std::ffi::{c_char, c_int, CString};
use std::ptr;

extern "C" {
    static mut stderr: *mut libc::FILE;
    fn truncate64(path: *const c_char, length: i64) -> c_int;
    fn scandirat(
        dirfd: c_int,
        path: *const c_char,
        namelist: *mut *mut *mut libc::dirent,
        filter: Option<unsafe extern "C" fn(*const libc::dirent) -> c_int>,
        compare: Option<
            unsafe extern "C" fn(*mut *const libc::dirent, *mut *const libc::dirent) -> c_int,
        >,
    ) -> c_int;
    fn alphasort(left: *mut *const libc::dirent, right: *mut *const libc::dirent) -> c_int;
}

fn path(value: &str) -> CString {
    CString::new(value).expect("path has no NUL")
}

fn errno() -> c_int {
    unsafe { *libc::__errno_location() }
}

fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

fn exists(value: &str) -> bool {
    let value = path(value);
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::stat(value.as_ptr(), &mut st) == 0
    }
}

fn write_file(value: &str, contents: &str) {
    let value = path(value);
    let mode = CString::new("w").unwrap();
    unsafe {
        let file = libc::fopen(value.as_ptr(), mode.as_ptr());
        if !file.is_null() {
            libc::fputs(path(contents).as_ptr(), file);
            libc::fclose(file);
        }
    }
}

fn append_file(value: &str, contents: &str) {
    let value = path(value);
    let mode = CString::new("a").unwrap();
    unsafe {
        let file = libc::fopen(value.as_ptr(), mode.as_ptr());
        if !file.is_null() {
            libc::fputs(path(contents).as_ptr(), file);
            libc::fclose(file);
        }
    }
}

fn read_file(value: &str) -> String {
    let value = path(value);
    let mode = CString::new("r").unwrap();
    let mut out = [0u8; 32];
    unsafe {
        let file = libc::fopen(value.as_ptr(), mode.as_ptr());
        if file.is_null() {
            return String::new();
        }
        let result = libc::fgets(out.as_mut_ptr().cast(), out.len() as c_int, file);
        libc::fclose(file);
        if result.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(out.as_ptr().cast())
                .to_string_lossy()
                .into_owned()
        }
    }
}

fn open_fd_count() -> c_int {
    unsafe {
        let dir = libc::opendir(c"/proc/self/fd".as_ptr());
        if dir.is_null() {
            return -1;
        }
        let mut count = 0;
        loop {
            let entry = libc::readdir(dir);
            if entry.is_null() {
                break;
            }
            if (*entry).d_name[0] != b'.' as c_char {
                count += 1;
            }
        }
        libc::closedir(dir);
        count
    }
}

fn sock_family(fd: c_int) -> c_int {
    unsafe {
        let mut storage: libc::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        if libc::getsockname(
            fd,
            (&mut storage as *mut libc::sockaddr_storage).cast(),
            &mut len,
        ) != 0
        {
            return -1;
        }
        let addr: *const libc::sockaddr = (&storage as *const libc::sockaddr_storage).cast();
        (*addr).sa_family as c_int
    }
}

fn stat_mode(value: &str) -> i32 {
    let value = path(value);
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::stat(value.as_ptr(), &mut st) == 0 {
            (st.st_mode & 0o777) as i32
        } else {
            -1
        }
    }
}

fn stat_size(value: &str) -> i64 {
    let value = path(value);
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::stat(value.as_ptr(), &mut st) == 0 {
            st.st_size
        } else {
            -1
        }
    }
}

fn create_mode(path_value: &str, flags: c_int, mode: libc::mode_t) -> c_int {
    let value = path(path_value);
    unsafe { libc::open(value.as_ptr(), flags, mode) }
}

pub fn run() -> i32 {
    unsafe {
        write_file("/root/.p-unlink", "x");
        clear_errno();
        let rc = libc::unlink(path("/root/.p-unlink").as_ptr());
        let unlink_errno = if rc == 0 { 0 } else { errno() };
        println!(
            "CTL_UNLINK rc={} errno={} exists_after={}",
            rc,
            unlink_errno,
            exists("/root/.p-unlink") as i32
        );

        write_file("/root/.p-remove", "x");
        clear_errno();
        let rc = libc::remove(path("/root/.p-remove").as_ptr());
        let remove_errno = if rc == 0 { 0 } else { errno() };
        println!(
            "REMOVE rc={} errno={} exists_after={}",
            rc,
            remove_errno,
            exists("/root/.p-remove") as i32
        );

        let ifd = libc::inotify_init1(libc::IN_NONBLOCK);
        clear_errno();
        let wd = libc::inotify_add_watch(
            ifd,
            path("/root").as_ptr(),
            libc::IN_CREATE | libc::IN_MODIFY,
        );
        let watch_errno = if wd >= 0 { 0 } else { errno() };
        let message = if wd < 0 {
            std::ffi::CStr::from_ptr(libc::strerror(errno()))
                .to_string_lossy()
                .into_owned()
        } else {
            "Success".to_string()
        };
        println!(
            "INOTIFY fd={} wd={} errno={} {}",
            ifd, wd, watch_errno, message
        );
        if wd >= 0 {
            write_file("/root/.p-notify", "");
            libc::usleep(200_000);
            let mut buf = [0u8; 4096];
            clear_errno();
            let n = libc::read(ifd, buf.as_mut_ptr().cast(), buf.len());
            println!("INOTIFY_EVENT n={} errno={}", n, errno());
        }
        if ifd >= 0 {
            libc::close(ifd);
        }

        clear_errno();
        let reopen_path = path("/root/.p-freopen");
        let reopen_mode = b"w\0";
        let reopened = libc::freopen(reopen_path.as_ptr(), reopen_mode.as_ptr().cast(), stderr);
        let reopen_ok = !reopened.is_null();
        let reopen_errno = if reopen_ok { 0 } else { errno() };
        let marker = b"FREOPEN_MARK\n\0";
        libc::fprintf(stderr, marker.as_ptr().cast());
        libc::fflush(stderr);
        println!(
            "FREOPEN ok={} errno={} exists={}",
            reopen_ok as i32,
            reopen_errno,
            exists("/root/.p-freopen") as i32
        );

        clear_errno();
        let name_max = libc::pathconf(path("/root").as_ptr(), libc::_PC_NAME_MAX);
        let pathconf_errno = errno();
        println!("PATHCONF v={} errno={}", name_max, pathconf_errno);

        clear_errno();
        let mut vfs: libc::statvfs = std::mem::zeroed();
        let statvfs_rc = libc::statvfs(path("/root").as_ptr(), &mut vfs);
        let statvfs_errno = if statvfs_rc == 0 { 0 } else { errno() };
        println!(
            "STATVFS rc={} errno={} bsize={} namemax={}",
            statvfs_rc, statvfs_errno, vfs.f_bsize, vfs.f_namemax
        );

        clear_errno();
        let mut entries: *mut *mut libc::dirent = ptr::null_mut();
        let count = scandirat(
            libc::AT_FDCWD,
            path("/root").as_ptr(),
            &mut entries,
            None,
            Some(alphasort),
        );
        println!("SCANDIRAT n={} errno={}", count, errno());
        if !entries.is_null() {
            libc::free(entries.cast());
        }

        write_file("/root/.p-trunc", "0123456789");
        clear_errno();
        let rc = truncate64(path("/root/.p-trunc").as_ptr(), 3);
        let truncate_errno = if rc == 0 { 0 } else { errno() };
        println!(
            "TRUNCATE64 rc={} errno={} size={}",
            rc,
            truncate_errno,
            stat_size("/root/.p-trunc")
        );
        let stamp = libc::utimbuf {
            actime: 1_000_000_000,
            modtime: 1_000_000_000,
        };
        clear_errno();
        let rc = libc::utime(path("/root/.p-trunc").as_ptr(), &stamp);
        let utime_errno = if rc == 0 { 0 } else { errno() };
        println!("UTIME rc={} errno={}", rc, utime_errno);

        let dir = "/root/.p-hardlink";
        let a = "/root/.p-hardlink/a";
        let b = "/root/.p-hardlink/b";
        libc::mkdir(path(dir).as_ptr(), 0o700);
        libc::unlink(path(a).as_ptr());
        libc::unlink(path(b).as_ptr());
        write_file(a, "AAA");
        clear_errno();
        let link_rc = libc::link(path(a).as_ptr(), path(b).as_ptr());
        let link_errno = if link_rc == 0 { 0 } else { errno() };
        clear_errno();
        append_file(a, "BBB");
        let contents = read_file(b);
        println!(
            "HARDLINK_STDIO link_rc={} link_errno={} content={}",
            link_rc, link_errno, contents
        );
        libc::unlink(path(a).as_ptr());
        libc::unlink(path(b).as_ptr());
        libc::rmdir(path(dir).as_ptr());

        let create = "/root/.p-create-mode";
        libc::unlink(path(create).as_ptr());
        let old_umask = libc::umask(0);
        clear_errno();
        let fd = create_mode(create, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, 0o440);
        let create_errno = if fd >= 0 { 0 } else { errno() };
        let mode = stat_mode(create);
        let wr = if fd >= 0 {
            libc::write(fd, b"C".as_ptr().cast(), 1)
        } else {
            -1
        };
        if fd >= 0 {
            libc::close(fd);
        }
        libc::umask(old_umask);
        println!(
            "CREATE_MODE open_fd={} errno={} mode={:o} write={} expected=440",
            fd, create_errno, mode, wr
        );
        libc::unlink(path(create).as_ptr());

        let owner = "/root/.p-owner-restore";
        libc::unlink(path(owner).as_ptr());
        let fd = create_mode(owner, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o600);
        if fd >= 0 {
            libc::write(fd, b"A".as_ptr().cast(), 1);
            libc::close(fd);
        }
        libc::chmod(path(owner).as_ptr(), 0o440);
        clear_errno();
        let fd = create_mode(owner, libc::O_WRONLY, 0);
        let owner_errno = if fd >= 0 { 0 } else { errno() };
        let mode_open = stat_mode(owner);
        let wr = if fd >= 0 {
            libc::write(fd, b"B".as_ptr().cast(), 1)
        } else {
            -1
        };
        if fd >= 0 {
            libc::close(fd);
        }
        let mode_final = stat_mode(owner);
        println!(
            "OWNER_RESTORE open_fd={} errno={} mode_open={:o} write={} mode_final={:o}",
            fd, owner_errno, mode_open, wr, mode_final
        );
        libc::unlink(path(owner).as_ptr());

        let access_root = "/root/.p-access-root";
        libc::unlink(path(access_root).as_ptr());
        let fd = create_mode(
            access_root,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o600,
        );
        if fd >= 0 {
            libc::close(fd);
        }
        libc::chmod(path(access_root).as_ptr(), 0o000);
        clear_errno();
        let rd = libc::access(path(access_root).as_ptr(), libc::R_OK);
        let re = if rd == 0 { 0 } else { errno() };
        clear_errno();
        let wr = libc::access(path(access_root).as_ptr(), libc::W_OK);
        let we = if wr == 0 { 0 } else { errno() };
        clear_errno();
        let x0 = libc::access(path(access_root).as_ptr(), libc::X_OK);
        let xe = if x0 == 0 { 0 } else { errno() };
        libc::chmod(path(access_root).as_ptr(), 0o555);
        clear_errno();
        let x1 = libc::access(path(access_root).as_ptr(), libc::X_OK);
        let x1e = if x1 == 0 { 0 } else { errno() };
        println!(
            "ACCESS_ROOT read={}/{} write={}/{} exec0={}/{} exec5={}/{}",
            rd, re, wr, we, x0, xe, x1, x1e
        );
        libc::unlink(path(access_root).as_ptr());

        let deny_dir = "/tmp/.p-access-deny";
        let deny_file = "/tmp/.p-access-deny/file";
        let own_dir = "/tmp/.p-access-own";
        let own_file = "/tmp/.p-access-own/owned";
        libc::unlink(path(deny_file).as_ptr());
        libc::unlink(path(own_file).as_ptr());
        libc::rmdir(path(deny_dir).as_ptr());
        libc::rmdir(path(own_dir).as_ptr());
        libc::mkdir(path(deny_dir).as_ptr(), 0o700);
        libc::chmod(path(deny_dir).as_ptr(), 0o700);
        libc::mkdir(path(own_dir).as_ptr(), 0o755);
        libc::chmod(path(own_dir).as_ptr(), 0o755);
        let fd = create_mode(
            deny_file,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o600,
        );
        if fd >= 0 {
            libc::close(fd);
        }
        libc::chmod(path(deny_file).as_ptr(), 0o644);
        let fd = create_mode(
            own_file,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o600,
        );
        if fd >= 0 {
            libc::fchown(fd, 1000, 1000);
            libc::close(fd);
        }
        libc::chmod(path(own_file).as_ptr(), 0o600);
        libc::setgid(1000);
        libc::setuid(1000);
        clear_errno();
        let parent = libc::access(path(deny_file).as_ptr(), libc::R_OK);
        let parent_errno = if parent == 0 { 0 } else { errno() };
        clear_errno();
        let own = libc::access(path(own_file).as_ptr(), libc::R_OK);
        let own_errno = if own == 0 { 0 } else { errno() };
        libc::setuid(0);
        libc::setgid(0);
        println!(
            "ACCESS_USER parent={}/{} owned={}/{}",
            parent, parent_errno, own, own_errno
        );
        libc::unlink(path(deny_file).as_ptr());
        libc::unlink(path(own_file).as_ptr());
        libc::rmdir(path(deny_dir).as_ptr());
        libc::rmdir(path(own_dir).as_ptr());

        clear_errno();
        let fd = libc::syscall(
            libc::SYS_openat,
            libc::AT_FDCWD,
            path("/root/.p-rawopen").as_ptr(),
            libc::O_WRONLY | libc::O_CREAT,
            0o644,
        ) as c_int;
        println!("RAW_OPENAT fd={} errno={}", fd, errno());
        if fd >= 0 {
            libc::close(fd);
        }

        let before = open_fd_count();
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
        let dupfd = if fd >= 0 { libc::dup(fd) } else { -1 };
        let dup2fd = if fd >= 0 { libc::dup2(fd, 63) } else { -1 };
        let fcntlfd = if fd >= 0 {
            libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 64)
        } else {
            -1
        };
        let f0 = sock_family(fd);
        let f1 = sock_family(dupfd);
        let f2 = sock_family(dup2fd);
        let f3 = sock_family(fcntlfd);
        if fd >= 0 {
            libc::close(fd);
        }
        let f4 = sock_family(dupfd);
        let f5 = sock_family(dup2fd);
        let f6 = sock_family(fcntlfd);
        if dupfd >= 0 {
            libc::close(dupfd);
        }
        let f7 = sock_family(dup2fd);
        let f8 = sock_family(fcntlfd);
        if dup2fd >= 0 {
            libc::close(dup2fd);
        }
        let f9 = sock_family(fcntlfd);
        if fcntlfd >= 0 {
            libc::close(fcntlfd);
        }
        let after = open_fd_count();
        println!(
            "NETLINK_DUP before={} opened={} families={},{},{},{} dup={},{},{} final={},{},{} after={}",
            before,
            if fcntlfd >= 0 { before + 5 } else { -1 },
            f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, after
        );

        let before = open_fd_count();
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
        let opened = open_fd_count();
        clear_errno();
        let rc = libc::close_range(fd as u32, fd as u32, 0);
        println!(
            "CLOSE_RANGE_NETLINK before={} opened={} rc={} errno={} after={}",
            before,
            opened,
            rc,
            errno(),
            open_fd_count()
        );

        let mut pipe_fds = [0; 2];
        let pipe_rc = libc::pipe(pipe_fds.as_mut_ptr());
        clear_errno();
        let rc = if pipe_rc == 0 {
            libc::close_range(pipe_fds[0] as u32, pipe_fds[1] as u32, 0)
        } else {
            -1
        };
        let close_errno = errno();
        let fd0 = if pipe_rc == 0 {
            libc::fcntl(pipe_fds[0], libc::F_GETFD)
        } else {
            -1
        };
        let fd1 = if pipe_rc == 0 {
            libc::fcntl(pipe_fds[1], libc::F_GETFD)
        } else {
            -1
        };
        println!(
            "CLOSE_RANGE_PIPE pipe={} rc={} errno={} fd0={} fd1={}",
            pipe_rc, rc, close_errno, fd0, fd1
        );

        let mut pipe_fds = [0; 2];
        let pipe_rc = libc::pipe(pipe_fds.as_mut_ptr());
        clear_errno();
        let rc = if pipe_rc == 0 {
            libc::close_range(pipe_fds[0] as u32, pipe_fds[1] as u32, 4)
        } else {
            -1
        };
        let close_errno = errno();
        let fd0 = if pipe_rc == 0 {
            libc::fcntl(pipe_fds[0], libc::F_GETFD)
        } else {
            -1
        };
        let fd1 = if pipe_rc == 0 {
            libc::fcntl(pipe_fds[1], libc::F_GETFD)
        } else {
            -1
        };
        println!(
            "CLOSE_RANGE_CLOEXEC pipe={} rc={} errno={} fd0={} fd1={}",
            pipe_rc, rc, close_errno, fd0, fd1
        );
        if pipe_rc == 0 {
            libc::close(pipe_fds[0]);
            libc::close(pipe_fds[1]);
        }

        clear_errno();
        let rc = libc::close_range(3, 3, 2);
        println!("CLOSE_RANGE_UNSHARE rc={} errno={}", rc, errno());

        let p = "/root/.p-close-range-root";
        libc::unlink(path(p).as_ptr());
        let pid = libc::fork();
        if pid == 0 {
            let rc = libc::close_range(3, u32::MAX, 0);
            let fd = create_mode(p, libc::O_WRONLY | libc::O_CREAT, 0o600);
            libc::_exit(if rc == 0 && fd >= 0 { 0 } else { 3 });
        }
        let mut status = -1;
        libc::waitpid(pid, &mut status, 0);
        println!(
            "CLOSE_RANGE_ALL child={} exit={} exists={}",
            pid,
            if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                -1
            },
            exists(p) as i32
        );
        libc::unlink(path(p).as_ptr());
    }
    0
}
