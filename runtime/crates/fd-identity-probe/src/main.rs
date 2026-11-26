//! Dynamic descriptor-identity regression probe for synthetic V4L2 and
//! netlink descriptors.
//!
//! Run this binary through the Husk loader. It deliberately talks to libc's
//! open/ioctl/dup2/dup3/socket entry points so the loader-installed path shim
//! sees the calls.
//!
//! The important case is `dup2(ordinary, fake_destination)` / `dup3(...)`:
//! the kernel closes the old destination before installing the source. Any
//! private table that still calls that descriptor number a fake device is stale.
//! The ioctl/getsockname result must prove the descriptor left the emulation.

use std::ffi::{c_void, CString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const QUERYCAP: libc::c_ulong = 0x8068_5600;
const AF_NETLINK: libc::c_int = 16;
const NETLINK_ROUTE: libc::c_int = 0;
const COPY_FD: RawFd = 73;

/// A memfd whose kernel-side name is exactly `name` (readlink shows
/// `/memfd:<name>`). Short name on purpose: the identity grammar is matched
/// against the kernel's target string, so the name is the thing under test.
fn memfd(name: &str) -> RawFd {
    const SYS_MEMFD_CREATE: libc::c_long = 279;
    let c = CString::new(name).expect("memfd name");
    unsafe { libc::syscall(SYS_MEMFD_CREATE, c.as_ptr(), 0u32) as RawFd }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn line(name: &str, ok: bool, detail: impl AsRef<str>) {
    println!(
        "fdidentity|{}|{}|{}",
        name,
        if ok { "PASS" } else { "FAIL" },
        detail.as_ref()
    );
}

fn test_pathshim_drm_open_cloexec() {
    const NAME: &str = "pathshim-drm-open-cloexec";
    match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/dev/dri/renderD128")
    {
        Ok(file) => {
            let fd = file.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            line(
                NAME,
                flags >= 0 && flags & libc::FD_CLOEXEC != 0,
                format!("fd={fd} fdflags={flags} expected_cloexec=1"),
            );
        }
        Err(e) => line(NAME, false, format!("open /dev/dri/renderD128: {e}")),
    }
}

fn test_pathshim_fchown_owner() {
    const NAME: &str = "pathshim-fchown-owner";
    const PATH: &str = "/root/.fdidentity-fchown-owner";
    let _ = std::fs::remove_file(PATH);
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(PATH)
    {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, format!("create: {e}"));
            return;
        }
    };
    let fd = file.as_raw_fd();
    let chown_rc = unsafe { libc::fchown(fd, 1500, 1600) };
    let chown_errno = if chown_rc == 0 { 0 } else { errno() };
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let stat_rc = unsafe { libc::fstat(fd, &mut st) };
    let same = format!("{}:{}", st.st_uid, st.st_gid);
    drop(file);
    let cross = match std::process::Command::new("/usr/bin/stat")
        .args(["-c", "%u:%g", PATH])
        .output()
    {
        Ok(v) if v.status.success() => String::from_utf8_lossy(&v.stdout).trim().to_string(),
        Ok(v) => format!(
            "stat rc={} stderr={}",
            v.status,
            String::from_utf8_lossy(&v.stderr).trim()
        ),
        Err(e) => format!("spawn stat: {e}"),
    };
    let _ = std::fs::remove_file(PATH);
    line(
        NAME,
        chown_rc == 0 && stat_rc == 0 && same == "1500:1600" && cross == "1500:1600",
        format!(
            "chown_rc={chown_rc} chown_errno={chown_errno} same={same} cross={cross} expected=1500:1600"
        ),
    );
}

fn test_v4l2_open_flags(rootfs: &str) {
    const NAME: &str = "pathshim-v4l2-open-flags";
    let sock_path = PathBuf::from(rootfs).join("run/husk/camera.sock");
    let (server, ready) = match camera_server(sock_path.clone()) {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, e);
            return;
        }
    };
    let video = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open("/dev/video0")
    {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, format!("open /dev/video0: {e}"));
            return;
        }
    };
    if ready.recv_timeout(Duration::from_secs(8)) != Ok(Ok(())) {
        line(NAME, false, "camera INFO handshake failed");
        return;
    }
    let fd = video.as_raw_fd();
    let fdflags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    line(
        NAME,
        fdflags >= 0 && fdflags & libc::FD_CLOEXEC != 0 && fl >= 0 && fl & libc::O_NONBLOCK != 0,
        format!("fd={fd} fdflags={fdflags} fl={fl} expected_cloexec=1 expected_nonblock=1"),
    );
    drop(video);
    if let Err(e) = finish_camera(server) {
        line(&format!("{NAME}-socket"), false, e);
    }
    let _ = std::fs::remove_file(sock_path);
}

fn open_video() -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/video0")
        .map_err(|e| format!("open /dev/video0: errno={}", e.raw_os_error().unwrap_or(0)))
}

fn querycap(fd: RawFd) -> Result<String, i32> {
    let mut cap = [0u8; 104];
    let rc = unsafe { libc::ioctl(fd, QUERYCAP, cap.as_mut_ptr() as *mut c_void) };
    if rc == 0 {
        let end = cap[..16].iter().position(|b| *b == 0).unwrap_or(16);
        Ok(String::from_utf8_lossy(&cap[..end]).into_owned())
    } else {
        Err(errno())
    }
}

fn camera_server(
    sock_path: PathBuf,
) -> Result<
    (
        std::thread::JoinHandle<Result<(), String>>,
        mpsc::Receiver<Result<(), String>>,
    ),
    String,
> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let _ = std::fs::remove_file(&sock_path);
    let listener =
        UnixListener::bind(&sock_path).map_err(|e| format!("bind {}: {e}", sock_path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("listener nonblocking: {e}"))?;

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let handle = std::thread::spawn(move || -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut stream: UnixStream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("camera server accept timeout".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(format!("accept: {e}")),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("stream timeout setup: {e}"))?;
        let mut info = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream
                .read_exact(&mut byte)
                .map_err(|e| format!("read INFO: {e}"))?;
            if byte[0] == b'\n' {
                break;
            }
            info.push(byte[0]);
            if info.len() > 64 {
                return Err("INFO line too long".into());
            }
        }
        if info != b"INFO" {
            return Err(format!(
                "unexpected first command: {:?}",
                String::from_utf8_lossy(&info)
            ));
        }
        stream
            .write_all(b"CAM k=0 id=test facing=back name=TEST sizes=640x480\nOK\n")
            .map_err(|e| format!("write INFO reply: {e}"))?;
        let _ = ready_tx.send(Ok(()));
        let mut sink = [0u8; 64];
        loop {
            match stream.read(&mut sink) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(e) => return Err(format!("wait camera EOF: {e}")),
            }
        }
    });
    Ok((handle, ready_rx))
}

fn finish_camera(handle: std::thread::JoinHandle<Result<(), String>>) -> Result<(), String> {
    match handle.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("camera server: {e}")),
        Err(_) => Err("camera server panicked".into()),
    }
}

fn test_v4l2_overwrite(rootfs: &str, use_dup3: bool, name: &str) {
    let sock_path = PathBuf::from(rootfs).join("run/husk/camera.sock");
    let (server, ready) = match camera_server(sock_path.clone()) {
        Ok(v) => v,
        Err(e) => {
            line(name, false, e);
            return;
        }
    };
    let victim = match open_video() {
        Ok(v) => v,
        Err(e) => {
            line(name, false, e);
            return;
        }
    };
    if ready.recv_timeout(Duration::from_secs(8)) != Ok(Ok(())) {
        line(name, false, "camera INFO handshake failed");
        return;
    }
    let victim_fd = victim.as_raw_fd();
    let ordinary = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(format!("/tmp/fdidentity-v4l2-{name}"))
    {
        Ok(v) => v,
        Err(e) => {
            line(name, false, format!("ordinary open: {e}"));
            return;
        }
    };
    let ordinary_fd = ordinary.as_raw_fd();
    let rc = unsafe {
        if use_dup3 {
            libc::dup3(ordinary_fd, victim_fd, 0)
        } else {
            libc::dup2(ordinary_fd, victim_fd)
        }
    };
    if rc < 0 {
        line(name, false, format!("dup failed errno={}", errno()));
        return;
    }

    let got = querycap(victim_fd);
    let left_v4l2 = got.is_err();
    line(
        name,
        left_v4l2,
        format!("QUERYCAP={got:?} expected=negative after ordinary overwrite"),
    );

    drop(ordinary);
    drop(victim);
    match finish_camera(server) {
        Ok(()) => line(
            &format!("{name}-socket"),
            true,
            "camera socket reached EOF after overwrite",
        ),
        Err(e) => line(&format!("{name}-socket"), false, e),
    }
    let _ = std::fs::remove_file(sock_path);
}

fn test_v4l2_dup_preserved(rootfs: &str) {
    const NAME: &str = "v4l2-dup2-fake-preserved";
    let sock_path = PathBuf::from(rootfs).join("run/husk/camera.sock");
    let (server, ready) = match camera_server(sock_path.clone()) {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, e);
            return;
        }
    };
    let source = match open_video() {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, e);
            return;
        }
    };
    if ready.recv_timeout(Duration::from_secs(8)) != Ok(Ok(())) {
        line(NAME, false, "camera INFO handshake failed");
        return;
    }
    let source_fd = source.as_raw_fd();
    unsafe {
        libc::close(COPY_FD);
    }
    let copied = unsafe { libc::dup2(source_fd, COPY_FD) };
    if copied != COPY_FD {
        line(NAME, false, format!("dup2 rc={copied} errno={}", errno()));
        return;
    }
    drop(source);
    let got = querycap(COPY_FD);
    line(
        NAME,
        matches!(&got, Ok(driver) if driver == "husk"),
        format!("QUERYCAP={got:?} after source close"),
    );
    unsafe {
        libc::close(COPY_FD);
    }
    match finish_camera(server) {
        Ok(()) => line(
            &format!("{NAME}-socket"),
            true,
            "camera socket reached EOF at last close",
        ),
        Err(e) => line(&format!("{NAME}-socket"), false, e),
    }
    let _ = std::fs::remove_file(sock_path);
}

fn socket_family(fd: RawFd) -> Result<i32, i32> {
    let mut addr = [0u8; 128];
    let mut len = addr.len() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockname(
            fd,
            addr.as_mut_ptr() as *mut libc::sockaddr,
            &mut len as *mut libc::socklen_t,
        )
    };
    if rc < 0 {
        Err(errno())
    } else {
        Ok(i32::from_ne_bytes([addr[0], addr[1], 0, 0]))
    }
}

fn open_netlink() -> Result<File, String> {
    let fd = unsafe { libc::socket(AF_NETLINK, libc::SOCK_RAW, NETLINK_ROUTE) };
    if fd < 0 {
        Err(format!("socket(AF_NETLINK) errno={}", errno()))
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn test_netlink_socket_flags() {
    const NAME: &str = "pathshim-netlink-socket-flags";
    let sock = unsafe {
        libc::socket(
            AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            NETLINK_ROUTE,
        )
    };
    if sock < 0 {
        line(NAME, false, format!("socket errno={}", errno()));
        return;
    }
    let fdflags = unsafe { libc::fcntl(sock, libc::F_GETFD) };
    let fl = unsafe { libc::fcntl(sock, libc::F_GETFL) };
    line(
        NAME,
        fdflags >= 0 && fdflags & libc::FD_CLOEXEC != 0 && fl >= 0 && fl & libc::O_NONBLOCK != 0,
        format!("fd={sock} fdflags={fdflags} fl={fl} expected_cloexec=1 expected_nonblock=1"),
    );
    unsafe {
        libc::close(sock);
    }
}

fn test_netlink_overwrite(use_dup3: bool, name: &str) {
    let fake = match open_netlink() {
        Ok(v) => v,
        Err(e) => {
            line(name, false, e);
            return;
        }
    };
    let fake_fd = fake.as_raw_fd();
    let family_before = socket_family(fake_fd);
    let ordinary_fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0) };
    if ordinary_fd < 0 {
        line(name, false, format!("ordinary socket errno={}", errno()));
        return;
    }
    let rc = unsafe {
        if use_dup3 {
            libc::dup3(ordinary_fd, fake_fd, 0)
        } else {
            libc::dup2(ordinary_fd, fake_fd)
        }
    };
    if rc < 0 {
        line(name, false, format!("dup failed errno={}", errno()));
        return;
    }
    let family_after = socket_family(fake_fd);
    let left_netlink = matches!(family_after, Ok(family) if family != AF_NETLINK);
    line(
        name,
        family_before == Ok(AF_NETLINK) && left_netlink,
        format!("family_before={family_before:?} family_after={family_after:?}"),
    );
    unsafe {
        libc::close(ordinary_fd);
    }
    drop(fake);
}

fn test_netlink_dup_preserved() {
    const NAME: &str = "netlink-dup2-fake-preserved";
    let source = match open_netlink() {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, e);
            return;
        }
    };
    let source_fd = source.as_raw_fd();
    unsafe {
        libc::close(COPY_FD);
    }
    let rc = unsafe { libc::dup2(source_fd, COPY_FD) };
    if rc != COPY_FD {
        line(NAME, false, format!("dup2 rc={rc} errno={}", errno()));
        return;
    }
    drop(source);
    let family = socket_family(COPY_FD);
    line(
        NAME,
        family == Ok(AF_NETLINK),
        format!("family_after_source_close={family:?}"),
    );
    unsafe {
        libc::close(COPY_FD);
    }
}

unsafe fn load_drm_preload() -> Result<*mut c_void, String> {
    const PATH: &[u8] = b"/root/libhusk_drm.so\0";
    let handle = libc::dlopen(PATH.as_ptr() as *const libc::c_char, libc::RTLD_NOW);
    if handle.is_null() {
        let detail = libc::dlerror();
        let text = if detail.is_null() {
            "unknown dlopen error".into()
        } else {
            std::ffi::CStr::from_ptr(detail)
                .to_string_lossy()
                .into_owned()
        };
        Err(format!("dlopen /root/libhusk_drm.so: {text}"))
    } else {
        Ok(handle)
    }
}

fn test_drm_preload() {
    const NAME: &str = "drm-preload-dup2-ordinary-over-fake";
    const PRESERVED: &str = "drm-preload-dup2-fake-preserved";

    let handle = match unsafe { load_drm_preload() } {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, e.clone());
            line(PRESERVED, false, e);
            return;
        }
    };
    let open_sym = unsafe {
        libc::dlsym(
            handle,
            b"husk_drm_fake_open\0".as_ptr() as *const libc::c_char,
        )
    };
    let fake_sym = unsafe {
        libc::dlsym(
            handle,
            b"drm_shim_is_fake\0".as_ptr() as *const libc::c_char,
        )
    };
    if open_sym.is_null() || fake_sym.is_null() {
        line(
            NAME,
            false,
            "missing husk_drm_fake_open or drm_shim_is_fake",
        );
        return;
    }
    let fake_open: unsafe extern "C" fn() -> RawFd = unsafe { std::mem::transmute(open_sym) };
    let is_fake: unsafe extern "C" fn(RawFd) -> i32 = unsafe { std::mem::transmute(fake_sym) };
    let open_sym = unsafe { libc::dlsym(handle, b"open\0".as_ptr() as *const libc::c_char) };
    let openat_sym = unsafe { libc::dlsym(handle, b"openat\0".as_ptr() as *const libc::c_char) };
    if open_sym.is_null() || openat_sym.is_null() {
        line(NAME, false, "missing open or openat");
        return;
    }
    let preload_open: unsafe extern "C" fn(*const libc::c_char, i32, i32) -> RawFd =
        unsafe { std::mem::transmute(open_sym) };
    let preload_openat: unsafe extern "C" fn(i32, *const libc::c_char, i32, i32) -> RawFd =
        unsafe { std::mem::transmute(openat_sym) };
    let mmap_sym = unsafe { libc::dlsym(handle, b"mmap\0".as_ptr() as *const libc::c_char) };
    if mmap_sym.is_null() {
        line(NAME, false, "missing mmap");
        return;
    }
    let preload_mmap: unsafe extern "C" fn(
        *mut c_void,
        usize,
        i32,
        i32,
        RawFd,
        i64,
    ) -> *mut c_void = unsafe { std::mem::transmute(mmap_sym) };
    let ioctl_sym = unsafe { libc::dlsym(handle, b"ioctl\0".as_ptr() as *const libc::c_char) };
    if ioctl_sym.is_null() {
        line(NAME, false, "missing ioctl");
        return;
    }
    let preload_ioctl: unsafe extern "C" fn(RawFd, libc::c_ulong, *mut c_void) -> i32 =
        unsafe { std::mem::transmute(ioctl_sym) };

    let victim = unsafe { fake_open() };
    if victim < 0 {
        line(NAME, false, format!("husk_drm_fake_open errno={}", errno()));
        return;
    }
    let victim_owner = unsafe { File::from_raw_fd(victim) };
    let ordinary = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("/tmp/fdidentity-drm-preload-ordinary")
    {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, format!("ordinary open: {e}"));
            return;
        }
    };
    let before = unsafe { is_fake(victim) };
    let rc = unsafe { libc::dup2(ordinary.as_raw_fd(), victim) };
    if rc != victim {
        line(NAME, false, format!("dup2 rc={rc} errno={}", errno()));
        return;
    }
    let after = unsafe { is_fake(victim) };
    line(
        NAME,
        before != 0 && after == 0,
        format!("before={before} after={after} expected=1->0"),
    );
    drop(ordinary);
    drop(victim_owner);

    let source = unsafe { fake_open() };
    if source < 0 {
        line(
            PRESERVED,
            false,
            format!("husk_drm_fake_open errno={}", errno()),
        );
        return;
    }
    let source_owner = unsafe { File::from_raw_fd(source) };
    unsafe {
        libc::close(COPY_FD);
    }
    let copied = unsafe { libc::dup2(source, COPY_FD) };
    if copied != COPY_FD {
        line(
            PRESERVED,
            false,
            format!("dup2 rc={copied} errno={}", errno()),
        );
        return;
    }
    drop(source_owner);
    let family = unsafe { is_fake(COPY_FD) };
    line(
        PRESERVED,
        family != 0,
        format!("is_fake_after_source_close={family} expected=1"),
    );
    unsafe {
        libc::close(COPY_FD);
    }

    const FAILURE: &str = "drm-preload-open-failure-abi";
    let missing = CString::new("/definitely-missing-husk-drm-open").unwrap();
    let rc = unsafe { preload_open(missing.as_ptr(), libc::O_RDONLY, 0) };
    let error = errno();
    line(
        FAILURE,
        rc == -1 && error == libc::ENOENT,
        format!("rc={rc} errno={error} expected=-1/{}", libc::ENOENT),
    );

    const OPEN_CLOEXEC: &str = "drm-preload-open-cloexec";
    let render = CString::new("/dev/dri/renderD128").unwrap();
    let render_fd = unsafe { preload_open(render.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC, 0) };
    if render_fd < 0 {
        line(
            OPEN_CLOEXEC,
            false,
            format!("open rc={render_fd} errno={}", errno()),
        );
    } else {
        let fdflags = unsafe { libc::fcntl(render_fd, libc::F_GETFD) };
        line(
            OPEN_CLOEXEC,
            fdflags >= 0 && fdflags & libc::FD_CLOEXEC != 0,
            format!("fd={render_fd} fdflags={fdflags} expected_cloexec=1"),
        );
        unsafe {
            libc::close(render_fd);
        }
    }

    const MMAP_FAILURE: &str = "drm-preload-mmap-failure-errno";
    let mapped = unsafe {
        preload_mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            -1,
            0,
        )
    };
    let mmap_error = errno();
    line(
        MMAP_FAILURE,
        mapped as isize == -1 && mmap_error == libc::EBADF,
        format!(
            "rc={} errno={} expected=-1/{}",
            mapped as isize,
            mmap_error,
            libc::EBADF
        ),
    );

    const IOCTL_FAILURE: &str = "drm-preload-ioctl-failure-errno";
    match OpenOptions::new().read(true).open("/dev/null") {
        Ok(dev_null) => {
            let rc = unsafe { preload_ioctl(dev_null.as_raw_fd(), 0, std::ptr::null_mut()) };
            let error = errno();
            line(
                IOCTL_FAILURE,
                rc == -1 && error == libc::ENOTTY,
                format!("rc={} errno={} expected=-1/{}", rc, error, libc::ENOTTY),
            );
        }
        Err(e) => line(IOCTL_FAILURE, false, format!("open /dev/null: {e}")),
    }

    const OPENAT_FAILURE: &str = "drm-preload-openat-failure-errno";
    let missing_at = CString::new("/definitely-missing-husk-drm-openat").unwrap();
    let rc_at = unsafe { preload_openat(libc::AT_FDCWD, missing_at.as_ptr(), libc::O_RDONLY, 0) };
    let error_at = errno();
    line(
        OPENAT_FAILURE,
        rc_at == -1 && error_at == libc::ENOENT,
        format!(
            "rc={} errno={} expected=-1/{}",
            rc_at,
            error_at,
            libc::ENOENT
        ),
    );

    const CREATE: &str = "drm-preload-openat-create-mode";
    let create_path = std::env::var("NR_ROOTFS")
        .ok()
        .map(|root| format!("{root}/root/fdidentity-drm-openat-mode"))
        .unwrap_or_else(|| "/tmp/fdidentity-drm-openat-mode".into());
    let create_c = CString::new(create_path.clone()).unwrap();
    unsafe {
        libc::unlink(create_c.as_ptr());
    }
    let fd = unsafe {
        preload_openat(
            libc::AT_FDCWD,
            create_c.as_ptr(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC,
            0o640,
        )
    };
    if fd < 0 {
        line(CREATE, false, format!("openat rc={fd} errno={}", errno()));
    } else {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let stat_rc = unsafe { libc::fstat(fd, &mut st) };
        let mode = if stat_rc == 0 { st.st_mode & 0o777 } else { 0 };
        unsafe {
            libc::close(fd);
            libc::unlink(create_c.as_ptr());
        }
        line(
            CREATE,
            stat_rc == 0 && mode == 0o640,
            format!("stat_rc={stat_rc} mode={mode:o} expected=640 path={create_path}"),
        );
    }
}

/// Identity must come from exact names, never substrings.
///
/// The old predicate was `windows(4).any(|w| w == b"mali") || windows(12)
/// .any(|w| w == b"/memfd:husk-")`, which claimed two classes of foreign fd:
/// any ordinary path containing "mali", and every `husk-*` memfd - including
/// the V4L2 shim's camera fds (`/memfd:husk-v4l2-N`). Both then entered the DRM
/// ioctl/mmap emulation. This probe pins the exactness from the outside, the
/// way a real client sees it.
fn test_drm_is_fake_exactness() {
    const NAME: &str = "drm-preload-is-fake-exact-names";
    const SYS_MEMFD_CREATE: libc::c_long = 279;

    let handle = match unsafe { load_drm_preload() } {
        Ok(v) => v,
        Err(e) => {
            line(NAME, false, e);
            return;
        }
    };
    let sym = unsafe {
        libc::dlsym(
            handle,
            b"drm_shim_is_fake\0".as_ptr() as *const libc::c_char,
        )
    };
    if sym.is_null() {
        line(NAME, false, "missing drm_shim_is_fake");
        return;
    }
    let is_fake: unsafe extern "C" fn(RawFd) -> i32 = unsafe { std::mem::transmute(sym) };

    let mut problems: Vec<String> = Vec::new();

    // 1. An ordinary file whose *path* contains "mali" must not be claimed.
    let decoy = b"/tmp/mali-cache-decoy\0";
    let decoy_fd = unsafe {
        libc::open(
            decoy.as_ptr() as *const libc::c_char,
            libc::O_CREAT | libc::O_RDWR,
            0o644,
        )
    };
    if decoy_fd < 0 {
        problems.push(format!("open /tmp/mali-cache-decoy: {}", errno()));
    } else {
        let got = unsafe { is_fake(decoy_fd) };
        if got != 0 {
            problems.push(format!(
                "/tmp/mali-cache-decoy claimed as DRM fake fd ({got})"
            ));
        }
        unsafe { libc::close(decoy_fd) };
        unsafe { libc::unlink(decoy.as_ptr() as *const libc::c_char) };
    }

    // 2. Another subsystem's memfd must not be claimed. `/memfd:husk-v4l2-N`
    //    is what the camera shim hands to guests.
    let name = b"husk-v4l2-0\0";
    let v4l2_fd =
        unsafe { libc::syscall(SYS_MEMFD_CREATE, name.as_ptr() as *const libc::c_char, 0u32) }
            as RawFd;
    if v4l2_fd < 0 {
        problems.push(format!("memfd_create husk-v4l2-0: {}", errno()));
    } else {
        let got = unsafe { is_fake(v4l2_fd) };
        if got != 0 {
            problems.push(format!("/memfd:husk-v4l2-0 claimed as DRM fake fd ({got})"));
        }
        unsafe { libc::close(v4l2_fd) };
    }

    // 3. The two names that *are* ours must still be claimed.
    let drm_name = b"husk-drm\0";
    let drm_fd = unsafe {
        libc::syscall(
            SYS_MEMFD_CREATE,
            drm_name.as_ptr() as *const libc::c_char,
            0u32,
        )
    } as RawFd;
    if drm_fd < 0 {
        problems.push(format!("memfd_create husk-drm: {}", errno()));
    } else {
        let got = unsafe { is_fake(drm_fd) };
        if got != 1 {
            problems.push(format!("/memfd:husk-drm not claimed ({got})"));
        }
        unsafe { libc::close(drm_fd) };
    }

    let mali = b"/dev/mali0\0";
    let mali_fd = unsafe { libc::open(mali.as_ptr() as *const libc::c_char, libc::O_RDWR) };
    if mali_fd < 0 {
        problems.push(format!(
            "open /dev/mali0: {} (device absent, cannot verify positive case)",
            errno()
        ));
    } else {
        let got = unsafe { is_fake(mali_fd) };
        if got != 1 {
            problems.push(format!("/dev/mali0 not claimed ({got})"));
        }
        unsafe { libc::close(mali_fd) };
    }

    line(
        NAME,
        problems.is_empty(),
        if problems.is_empty() {
            "decoy paths and foreign memfds rejected; husk-drm and /dev/mali0 claimed".to_string()
        } else {
            problems.join("; ")
        },
    );
}

/// Identity that is lost when a descriptor number is not in our table.
///
/// `FAKE_FDS` is keyed by fd number, and a number is process-local: after an
/// `SCM_RIGHTS` transfer the receiver holds the same memfd under a number we
/// never registered. A raw `/memfd:husk-drm` reproduces that shape exactly - the
/// kernel target is ours, the table has nothing - and so does `dup` of it.
///
/// The answer must be an explicit `EOPNOTSUPP`, never the memfd's own `ENOTTY`:
/// clients read `ENOTTY` as "this is not a GPU" and drop to software rendering
/// with nothing in any log. Until round 3 this answered `EBADF(9)`, which is a
/// lie about a descriptor that is perfectly valid.
///
/// Deliberately *not* built on `open("/dev/dri/renderD128")` plus
/// `/proc/self/fd/N` any more: the shim opens that node afresh and re-registers
/// identity, so the case under test never happened - the ioctl was answered by
/// the DRM emulation instead (ENOSYS), and the old probe could not tell the
/// difference on a device that has no `renderD128` at all.
fn test_fake_fd_identity_lost() {
    const NAME: &str = "pathshim-drm-fake-fd-identity-lost";
    const DRM_MAGIC_IOCTL: u64 = 0x64b1; // _IOC_TYPE == 'd'

    let plain = memfd("husk-drm");
    if plain < 0 {
        line(
            NAME,
            false,
            format!("memfd_create(husk-drm): errno {}", errno()),
        );
        return;
    }
    let copied = unsafe { libc::dup(plain) };

    let on_memfd = unsafe { libc::ioctl(plain, DRM_MAGIC_IOCTL as _, 0usize) };
    let memfd_errno = errno();
    let on_copy = unsafe { libc::ioctl(copied, DRM_MAGIC_IOCTL as _, 0usize) };
    let copy_errno = errno();

    line(
        NAME,
        on_memfd == -1 && memfd_errno == 95 && on_copy == -1 && copy_errno == 95,
        format!(
            "memfd_fd={plain} rc={on_memfd} errno={memfd_errno} | dup_fd={copied} rc={on_copy} errno={copy_errno} (want -1/EOPNOTSUPP=95 both)"
        ),
    );
    unsafe {
        libc::close(copied);
        libc::close(plain);
    }
}

/// Same condition, but with the LD_PRELOAD DRM layer already live in this
/// process: `/memfd:husk-drm` is then claimed by that layer. Both layers must
/// answer the same explicit `EOPNOTSUPP(95)` for an unmodelled request.
///
/// The invariant that must hold in both worlds is "explicit refusal": never 0,
/// and never `ENOTTY(25)` - that is what makes clients discard the GPU. Using
/// the same errno in both paths also avoids selecting different client fallback
/// semantics just because `LD_PRELOAD` happened to be active.
fn test_fake_fd_identity_lost_with_preload() {
    const NAME: &str = "pathshim-drm-fake-fd-identity-lost-preload";
    const DRM_MAGIC_IOCTL: u64 = 0x64b1;

    let fd = memfd("husk-drm");
    if fd < 0 {
        line(
            NAME,
            false,
            format!("memfd_create(husk-drm): errno {}", errno()),
        );
        return;
    }
    let rc = unsafe { libc::ioctl(fd, DRM_MAGIC_IOCTL as _, 0usize) };
    let e = errno();
    unsafe { libc::close(fd) };
    line(
        NAME,
        rc == -1 && e == 95,
        format!("rc={rc} errno={e} (want -1/EOPNOTSUPP=95)"),
    );
}

/// A forged memfd name must not buy camera emulation.
///
/// Two things are checked, and they need different oracles:
///
/// * a *v4l2* name that is not legal grammar must be unclaimed, which means it
///   has to answer exactly like a neutral-name memfd from the same process. The
///   oracle cannot be "errno == ENOTTY": on the reference device the kernel
///   answers `EACCES(13)` for any ioctl on a memfd (SELinux ioctl xperm), so the
///   earlier "not ENOTTY means claimed" assertion reported a claim that did not
///   exist;
/// * `/memfd:husk-drm` is legitimately claimed - by the *DRM* layer. The v4l2
///   layer must not answer it with a working camera, so the only thing asserted
///   here is "no success"; the errno belongs to whichever DRM layer is live.
///
/// The positive direction - a descriptor the shim really issued is claimed - is
/// covered by the QUERYCAP cases above, which see `husk` as the card name.
fn test_v4l2_name_grammar() {
    const NAME: &str = "v4l2-name-grammar-exact";

    let control = memfd("husk-ctl");
    if control < 0 {
        line(
            NAME,
            false,
            format!("memfd_create(husk-ctl): errno {}", errno()),
        );
        return;
    }
    let control_rc = unsafe { libc::ioctl(control, QUERYCAP as _, 0usize) };
    let control_errno = errno();
    unsafe { libc::close(control) };

    let mut problems: Vec<String> = Vec::new();
    let cases: [(&str, &str); 3] = [
        ("husk-v4l2-0x", "trailing junk"),
        ("husk-v4l2-99", "index out of range"),
        ("husk-v4l2-", "no index at all"),
    ];
    for (name, why) in cases {
        let fd = memfd(name);
        if fd < 0 {
            problems.push(format!("memfd_create({name}): errno {}", errno()));
            continue;
        }
        let rc = unsafe { libc::ioctl(fd, QUERYCAP as _, 0usize) };
        let e = errno();
        if rc != control_rc || e != control_errno {
            problems.push(format!(
                "{name}: rc={rc} errno={e} differs from unclaimed control rc={control_rc} errno={control_errno} ({why})"
            ));
        }
        unsafe { libc::close(fd) };
    }

    // DRM's memfd: claimed by DRM, never by v4l2. Only "no camera" is provable here.
    let mut drm_note = String::from("husk-drm: not measured");
    let fd = memfd("husk-drm");
    if fd >= 0 {
        let rc = unsafe { libc::ioctl(fd, QUERYCAP as _, 0usize) };
        let e = errno();
        if rc == 0 {
            problems.push(format!(
                "husk-drm: QUERYCAP succeeded (rc=0) - the v4l2 layer answered a DRM memfd"
            ));
        }
        drm_note = format!("husk-drm rc={rc} errno={e} (refused, not a camera)");
        unsafe { libc::close(fd) };
    } else {
        problems.push(format!("memfd_create(husk-drm): errno {}", errno()));
    }

    line(
        NAME,
        problems.is_empty(),
        if problems.is_empty() {
            format!(
                "junk names unclaimed (=control rc={control_rc} errno={control_errno}); {drm_note}"
            )
        } else {
            problems.join(" | ")
        },
    );
}

fn main() {
    let rootfs = std::env::var("NR_ROOTFS").unwrap_or_else(|_| "/".into());
    line("start", true, format!("rootfs={rootfs}"));

    // 先跑：此时进程里还没有 dlopen 过 DRM preload 层，drm_api() 为空，答案由
    // path-shim 自己的"无状态假 fd"分支给出（生产里没预载 preload 的情形）。
    test_fake_fd_identity_lost();
    test_pathshim_drm_open_cloexec();
    test_pathshim_fchown_owner();
    test_v4l2_open_flags(&rootfs);
    test_netlink_socket_flags();
    test_v4l2_overwrite(&rootfs, false, "v4l2-dup2-ordinary-over-fake");
    test_v4l2_overwrite(&rootfs, true, "v4l2-dup3-ordinary-over-fake");
    test_v4l2_dup_preserved(&rootfs);

    test_netlink_overwrite(false, "netlink-dup2-ordinary-over-fake");
    test_netlink_overwrite(true, "netlink-dup3-ordinary-over-fake");
    test_netlink_dup_preserved();
    test_drm_preload();
    test_drm_is_fake_exactness();
    test_fake_fd_identity_lost_with_preload();
    test_v4l2_name_grammar();
}
