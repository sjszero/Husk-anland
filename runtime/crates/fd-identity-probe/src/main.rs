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

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const QUERYCAP: libc::c_ulong = 0x8068_5600;
const AF_NETLINK: libc::c_int = 16;
const NETLINK_ROUTE: libc::c_int = 0;
const COPY_FD: RawFd = 73;

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

fn main() {
    let rootfs = std::env::var("NR_ROOTFS").unwrap_or_else(|_| "/".into());
    line("start", true, format!("rootfs={rootfs}"));

    test_v4l2_overwrite(&rootfs, false, "v4l2-dup2-ordinary-over-fake");
    test_v4l2_overwrite(&rootfs, true, "v4l2-dup3-ordinary-over-fake");
    test_v4l2_dup_preserved(&rootfs);

    test_netlink_overwrite(false, "netlink-dup2-ordinary-over-fake");
    test_netlink_overwrite(true, "netlink-dup3-ordinary-over-fake");
    test_netlink_dup_preserved();
}
