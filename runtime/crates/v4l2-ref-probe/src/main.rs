//! Guest-side regression probe for V4L2 fake-fd lifetime.
//!
//! A real camera is not needed: the probe runs the same `camera.sock`
//! handshake against an in-process Unix listener, then exercises the exact
//! descriptor path that used to fail:
//!
//!   open -> dup above fd 1024 -> close the original -> ioctl on the dup.
//!
//! The old `refs()` scan only looked at 0..1023. It saw zero references after
//! closing the original, destroyed the camera session, and the high duplicate
//! stopped being a V4L2 fd. The listener staying connected here is the second
//! half of the assertion: it must close only after the duplicate is closed.

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use v4l2_shim::Ctx;

fn main() {
    if let Err(e) = run() {
        eprintln!("V4L2_REF|FAIL|{e}");
        std::process::exit(1);
    }
    println!("V4L2_REF|PASS");
}

fn run() -> Result<(), String> {
    let rootfs = std::env::var("NR_ROOTFS").map_err(|e| format!("NR_ROOTFS: {e}"))?;
    let sock_path = PathBuf::from(&rootfs).join("run/husk/camera.sock");
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let _ = std::fs::remove_file(&sock_path);
    let listener =
        UnixListener::bind(&sock_path).map_err(|e| format!("bind {}: {e}", sock_path.display()))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| format!("listener blocking mode: {e}"))?;

    let (eof_tx, eof_rx) = mpsc::channel::<()>();
    let server = std::thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|e| format!("accept: {e}"))?;
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
        let mut buf = [0u8; 64];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => return Err(format!("wait EOF: {e}")),
            }
        }
        let _ = eof_tx.send(());
        Ok(())
    });

    let mut ctx = Ctx::empty();
    let path = sock_path.to_string_lossy();
    if !unsafe { ctx.init(path.as_bytes()) } {
        return Err("Ctx::init rejected camera.sock path".into());
    }
    let fd = unsafe { ctx.open(0) };
    if fd < 0 {
        return Err(format!("open /dev/video0 returned {fd}"));
    }
    println!("V4L2_REF|open|fd={fd}");

    let high = unsafe { libc::fcntl(fd, libc::F_DUPFD, 1100) };
    if high < 1100 {
        return Err(format!(
            "fcntl(F_DUPFD, 1100) returned {high}: {}",
            std::io::Error::last_os_error()
        ));
    }
    println!("V4L2_REF|dup|high={high}");

    unsafe {
        ctx.close(fd);
        libc::close(fd);
    }
    std::thread::sleep(Duration::from_millis(100));
    if eof_rx.try_recv().is_ok() {
        return Err("camera socket closed while the high duplicate was still open".into());
    }
    println!("V4L2_REF|original-close|socket=alive");

    let mut cap = [0u8; 104];
    let rc = unsafe { ctx.ioctl(high, v4l2_shim::QUERYCAP, cap.as_mut_ptr()) };
    if rc != 0 {
        return Err(format!("QUERYCAP on high duplicate returned {rc}"));
    }
    if &cap[..4] != b"husk" {
        return Err(format!("unexpected QUERYCAP driver bytes: {:?}", &cap[..4]));
    }
    println!("V4L2_REF|ioctl-high|rc=0|driver=husk");

    unsafe {
        ctx.close(high);
        libc::close(high);
    }
    match eof_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(()) => println!("V4L2_REF|last-close|socket=eof"),
        Err(_) => return Err("camera socket stayed open after the last reference closed".into()),
    }

    match server.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("fake camera server: {e}")),
        Err(_) => return Err("fake camera server panicked".into()),
    }
    let _ = std::fs::remove_file(&sock_path);
    Ok(())
}
