//! Server-surface checks that used to be Python one-liners.
//!
//! The shell driver still owns presentation and the `R|...` protocol. This
//! module owns the operations where Python silently supplied libc semantics:
//! NSS, locks, xattrs, sparse files, shared mappings, UNIX sockets, fd passing,
//! SysV/POSIX IPC, readiness APIs, resolver/socket paths and syslog(3).

use std::ffi::{c_int, c_void, CStr, CString};
use std::io;
use std::mem;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::time::Duration;

const ONE_GIB: libc::off_t = 1 << 30;

struct Fd(RawFd);

impl Fd {
    fn new(fd: RawFd, what: &str) -> io::Result<Self> {
        if fd < 0 {
            Err(os_error(what))
        } else {
            Ok(Self(fd))
        }
    }

    fn raw(&self) -> RawFd {
        self.0
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

struct RemoveFile(PathBuf);

impl Drop for RemoveFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn os_error(what: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

fn c_path(value: &str) -> io::Result<CString> {
    CString::new(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))
}

fn c_value(value: &str) -> io::Result<CString> {
    CString::new(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in value"))
}

fn open_path(path: &str, flags: c_int, mode: libc::mode_t) -> io::Result<Fd> {
    let path = c_path(path)?;
    let fd = unsafe { libc::open(path.as_ptr(), flags, mode) };
    Fd::new(fd, "open")
}

fn require_libc(rc: c_int, what: &str) -> io::Result<()> {
    if rc < 0 {
        Err(os_error(what))
    } else {
        Ok(())
    }
}

fn group_member(name: &str, group: &str) -> io::Result<()> {
    let name = c_value(name)?;
    let group = c_value(group)?;
    unsafe {
        let entry = libc::getgrnam(group.as_ptr());
        if entry.is_null() {
            return Err(os_error("getgrnam"));
        }
        let mut member = (*entry).gr_mem;
        if member.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "group has no member list",
            ));
        }
        while !(*member).is_null() {
            if CStr::from_ptr(*member).to_bytes() == name.as_bytes() {
                return Ok(());
            }
            member = member.add(1);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "{name_original} not in {group_original}",
            name_original = name.to_string_lossy(),
            group_original = group.to_string_lossy()
        ),
    ))
}

fn flock_exclusive(path: &str) -> io::Result<()> {
    let _cleanup = RemoveFile(PathBuf::from(path));
    let fd = open_path(path, libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC, 0o600)?;
    require_libc(
        unsafe { libc::flock(fd.raw(), libc::LOCK_EX | libc::LOCK_NB) },
        "flock",
    )?;
    require_libc(
        unsafe { libc::flock(fd.raw(), libc::LOCK_UN) },
        "flock unlock",
    )
}

fn posix_lock(path: &str) -> io::Result<()> {
    let _cleanup = RemoveFile(PathBuf::from(path));
    let fd = open_path(path, libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC, 0o600)?;
    let mut lock: libc::flock = unsafe { mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    lock.l_start = 0;
    lock.l_len = 0;
    require_libc(
        unsafe { libc::fcntl(fd.raw(), libc::F_SETLK, &lock) },
        "fcntl F_SETLK",
    )?;
    lock.l_type = libc::F_UNLCK as _;
    require_libc(
        unsafe { libc::fcntl(fd.raw(), libc::F_SETLK, &lock) },
        "fcntl unlock",
    )
}

fn inotify() -> io::Result<()> {
    let fd = Fd::new(
        unsafe { libc::inotify_init1(libc::IN_CLOEXEC) },
        "inotify_init1",
    )?;
    drop(fd);
    Ok(())
}

fn xattr_roundtrip(path: &str) -> io::Result<()> {
    let target = c_path(path)?;
    let name = c_value("user.husk")?;
    let value = b"1";
    require_libc(
        unsafe {
            libc::setxattr(
                target.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast::<c_void>(),
                value.len(),
                0,
            )
        },
        "setxattr",
    )?;
    require_libc(
        unsafe { libc::removexattr(target.as_ptr(), name.as_ptr()) },
        "removexattr",
    )
}

fn o_tmpfile(path: &str) -> io::Result<()> {
    let fd = open_path(
        path,
        libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
        0o600,
    )?;
    drop(fd);
    Ok(())
}

fn sparse(path: &str) -> io::Result<()> {
    let _cleanup = RemoveFile(PathBuf::from(path));
    let fd = open_path(
        path,
        libc::O_CREAT | libc::O_TRUNC | libc::O_RDWR | libc::O_CLOEXEC,
        0o600,
    )?;
    require_libc(
        unsafe { libc::ftruncate(fd.raw(), ONE_GIB) },
        "ftruncate 1 GiB",
    )
}

fn mmap_shared(path: &str) -> io::Result<()> {
    let _cleanup = RemoveFile(PathBuf::from(path));
    let fd = open_path(
        path,
        libc::O_CREAT | libc::O_TRUNC | libc::O_RDWR | libc::O_CLOEXEC,
        0o600,
    )?;
    require_libc(unsafe { libc::ftruncate(fd.raw(), 4096) }, "ftruncate")?;
    let address = unsafe {
        libc::mmap(
            ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.raw(),
            0,
        )
    };
    if address == libc::MAP_FAILED {
        return Err(os_error("mmap MAP_SHARED"));
    }
    unsafe {
        ptr::copy_nonoverlapping(b"ok".as_ptr(), address.cast::<u8>(), 2);
    }
    require_libc(
        unsafe { libc::msync(address, 4096, libc::MS_SYNC) },
        "msync",
    )?;
    let mut got = [0_u8; 2];
    let read = unsafe { libc::pread(fd.raw(), got.as_mut_ptr().cast(), got.len(), 0) };
    unsafe {
        libc::munmap(address, 4096);
    }
    if read != 2 || got != *b"ok" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mmap readback mismatch",
        ));
    }
    Ok(())
}

fn sockaddr_un(
    path: Option<&str>,
    abstract_name: Option<&str>,
) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let base = &address as *const _ as usize;
    let sun_path = address.sun_path.as_ptr() as usize;
    let used = if let Some(path) = path {
        let bytes = path.as_bytes();
        if bytes.len() + 1 > address.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "UNIX socket path too long",
            ));
        }
        for (destination, source) in address.sun_path[..bytes.len()].iter_mut().zip(bytes) {
            *destination = *source as libc::c_char;
        }
        sun_path - base + bytes.len() + 1
    } else if let Some(name) = abstract_name {
        let bytes = name.as_bytes();
        if bytes.len() + 1 > address.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "abstract socket name too long",
            ));
        }
        address.sun_path[0] = 0;
        for (destination, source) in address.sun_path[1..1 + bytes.len()].iter_mut().zip(bytes) {
            *destination = *source as libc::c_char;
        }
        sun_path - base + 1 + bytes.len()
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing UNIX address",
        ));
    };
    Ok((address, used as libc::socklen_t))
}

fn unix_stream(path: &str) -> io::Result<()> {
    let _cleanup = RemoveFile(PathBuf::from(path));
    let (address, length) = sockaddr_un(Some(path), None)?;
    let server = Fd::new(
        unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) },
        "socket",
    )?;
    require_libc(
        unsafe {
            libc::bind(
                server.raw(),
                &address as *const _ as *const libc::sockaddr,
                length,
            )
        },
        "bind",
    )?;
    require_libc(unsafe { libc::listen(server.raw(), 1) }, "listen")?;
    let client = Fd::new(
        unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) },
        "socket client",
    )?;
    require_libc(
        unsafe {
            libc::connect(
                client.raw(),
                &address as *const _ as *const libc::sockaddr,
                length,
            )
        },
        "connect",
    )?;
    let accepted = Fd::new(
        unsafe { libc::accept(server.raw(), ptr::null_mut(), ptr::null_mut()) },
        "accept",
    )?;
    drop(accepted);
    Ok(())
}

fn unix_abstract(name: &str) -> io::Result<()> {
    let (address, length) = sockaddr_un(None, Some(name))?;
    let socket = Fd::new(
        unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) },
        "socket",
    )?;
    require_libc(
        unsafe {
            libc::bind(
                socket.raw(),
                &address as *const _ as *const libc::sockaddr,
                length,
            )
        },
        "bind abstract",
    )
}

fn fd_pass(path: &str) -> io::Result<()> {
    let mut pair = [0; 2];
    require_libc(
        unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
                0,
                pair.as_mut_ptr(),
            )
        },
        "socketpair",
    )?;
    let sender = Fd::new(pair[0], "socketpair sender")?;
    let receiver = Fd::new(pair[1], "socketpair receiver")?;
    let file = open_path(path, libc::O_RDONLY | libc::O_CLOEXEC, 0)?;
    let payload = *b"x";
    let mut iov = libc::iovec {
        iov_base: payload.as_ptr() as *mut c_void,
        iov_len: payload.len(),
    };
    let mut control = [0_u8; 64];
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = unsafe { libc::CMSG_SPACE(mem::size_of::<c_int>() as u32) } as _;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        if header.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "no SCM_RIGHTS header",
            ));
        }
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(mem::size_of::<c_int>() as u32) as _;
        ptr::write(libc::CMSG_DATA(header).cast::<c_int>(), file.raw());
    }
    let sent = unsafe { libc::sendmsg(sender.raw(), &message, 0) };
    if sent < 0 {
        return Err(os_error("sendmsg SCM_RIGHTS"));
    }

    let mut received_data = [0_u8; 1];
    let mut received_iov = libc::iovec {
        iov_base: received_data.as_mut_ptr().cast(),
        iov_len: received_data.len(),
    };
    let mut received_control = [0_u8; 64];
    let mut received: libc::msghdr = unsafe { mem::zeroed() };
    received.msg_iov = &mut received_iov;
    received.msg_iovlen = 1;
    received.msg_control = received_control.as_mut_ptr().cast();
    received.msg_controllen = received_control.len() as _;
    let got = unsafe { libc::recvmsg(receiver.raw(), &mut received, 0) };
    if got < 0 {
        return Err(os_error("recvmsg SCM_RIGHTS"));
    }
    let header = unsafe { libc::CMSG_FIRSTHDR(&received) };
    if header.is_null() || unsafe { (*header).cmsg_type } != libc::SCM_RIGHTS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "received no SCM_RIGHTS",
        ));
    }
    let received_fd = unsafe { ptr::read(libc::CMSG_DATA(header).cast::<c_int>()) };
    let received_fd = Fd::new(received_fd, "received fd")?;
    drop(received_fd);
    Ok(())
}

fn sysv_shm() -> io::Result<()> {
    let key: libc::key_t = libc::IPC_PRIVATE;
    let id = unsafe { libc::shmget(key, 4096, libc::IPC_CREAT | 0o600) };
    if id < 0 {
        return Err(os_error("shmget"));
    }
    require_libc(
        unsafe { libc::shmctl(id, libc::IPC_RMID, ptr::null_mut()) },
        "shmctl IPC_RMID",
    )
}

fn posix_mq() -> io::Result<()> {
    let name = format!("/huskq-{}", std::process::id());
    let name = c_path(&name)?;
    let mut attributes: libc::mq_attr = unsafe { mem::zeroed() };
    let descriptor = unsafe {
        libc::mq_open(
            name.as_ptr(),
            libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
            &mut attributes as *mut libc::mq_attr,
        )
    };
    if descriptor < 0 {
        return Err(os_error("mq_open"));
    }
    let close = unsafe { libc::mq_close(descriptor) };
    let unlink = unsafe { libc::mq_unlink(name.as_ptr()) };
    require_libc(close, "mq_close")?;
    require_libc(unlink, "mq_unlink")
}

fn hostname() -> io::Result<()> {
    let mut buffer = [0 as libc::c_char; 256];
    require_libc(
        unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len()) },
        "gethostname",
    )?;
    let name = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    println!("{}", name.to_string_lossy());
    Ok(())
}

fn raw_icmp() -> io::Result<()> {
    let fd = Fd::new(
        unsafe { libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP) },
        "raw ICMP socket",
    )?;
    drop(fd);
    println!("raw ICMP allowed");
    Ok(())
}

fn eventfd() -> io::Result<()> {
    let fd = Fd::new(unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) }, "eventfd")?;
    drop(fd);
    Ok(())
}

fn timerfd() -> io::Result<()> {
    let fd = Fd::new(
        unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_CLOEXEC) },
        "timerfd_create",
    )?;
    drop(fd);
    Ok(())
}

fn epoll() -> io::Result<()> {
    let fd = Fd::new(
        unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) },
        "epoll_create1",
    )?;
    drop(fd);
    Ok(())
}

fn openpty() -> io::Result<()> {
    let master = Fd::new(
        unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) },
        "posix_openpt",
    )?;
    require_libc(unsafe { libc::grantpt(master.raw()) }, "grantpt")?;
    require_libc(unsafe { libc::unlockpt(master.raw()) }, "unlockpt")?;
    let mut name = [0 as libc::c_char; 128];
    let rc = unsafe { libc::ptsname_r(master.raw(), name.as_mut_ptr(), name.len()) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    let slave_path = unsafe { CStr::from_ptr(name.as_ptr()) };
    let slave = Fd::new(
        unsafe {
            libc::open(
                slave_path.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
            )
        },
        "open pty slave",
    )?;
    drop(slave);
    Ok(())
}

fn getaddrinfo(host: &str, port: &str) -> io::Result<()> {
    let port: u16 = port
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, format!("port: {error}")))?;
    (host, port)
        .to_socket_addrs()?
        .next()
        .map(|_| ())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no address resolved"))
}

fn tcp_outbound(address: &str, port: &str) -> io::Result<()> {
    let port: u16 = port
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, format!("port: {error}")))?;
    let address: SocketAddr = format!("{address}:{port}").parse().map_err(|error| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("address: {error}"))
    })?;
    TcpStream::connect_timeout(&address, Duration::from_secs(5)).map(drop)
}

fn tcp_listen() -> io::Result<()> {
    TcpListener::bind(("0.0.0.0", 0)).map(drop)
}

fn loopback() -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;
    let client = TcpStream::connect(address)?;
    let (server, _) = listener.accept()?;
    drop((client, server));
    Ok(())
}

fn udp() -> io::Result<()> {
    let socket = UdpSocket::bind(("127.0.0.1", 0))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let address = socket.local_addr()?;
    socket.send_to(b"x", address)?;
    let mut buffer = [0_u8; 4];
    let (size, _) = socket.recv_from(&mut buffer)?;
    if size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty UDP datagram",
        ));
    }
    Ok(())
}

fn ipv6() -> io::Result<()> {
    let fd = Fd::new(
        unsafe { libc::socket(libc::AF_INET6, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) },
        "AF_INET6 socket",
    )?;
    drop(fd);
    Ok(())
}

fn netlink() -> io::Result<()> {
    let fd = Fd::new(
        unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) },
        "AF_NETLINK socket",
    )?;
    let mut address: libc::sockaddr_nl = unsafe { mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as _;
    require_libc(
        unsafe {
            libc::bind(
                fd.raw(),
                &address as *const _ as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        },
        "netlink bind",
    )
}

fn parse_dbus_output(output: &str) -> Option<(String, i32)> {
    let mut address = None;
    let mut pid = None;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("unix:") {
            address = Some(line.to_owned());
        } else if let Ok(value) = line.parse::<i32>() {
            pid = Some(value);
        }
    }
    Some((address?, pid?))
}

fn dbus_session() -> io::Result<()> {
    let output = Command::new("dbus-daemon")
        .args(["--session", "--fork", "--print-address=1", "--print-pid=1"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let (address, pid) = parse_dbus_output(&stdout).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("dbus-daemon output: stdout={stdout:?} stderr={stderr:?}"),
        )
    })?;
    let stopped = unsafe { libc::kill(pid, libc::SIGTERM) } == 0;
    if !address.starts_with("unix:") || !stopped {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("dbus-daemon address={address} pid={pid} stopped={stopped}"),
        ));
    }
    Ok(())
}

fn syslog(identifier: &str, message: &str) -> io::Result<()> {
    let identifier = c_value(identifier)?;
    let message = c_value(message)?;
    let format = c_value("%s")?;
    unsafe {
        libc::openlog(identifier.as_ptr(), libc::LOG_PID, libc::LOG_USER);
        libc::syslog(libc::LOG_INFO, format.as_ptr(), message.as_ptr());
        libc::closelog();
    }
    Ok(())
}

fn cap_eff() -> io::Result<String> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    parse_cap_eff(&status)
        .map(str::to_owned)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "CapEff missing"))
}

fn parse_cap_eff(status: &str) -> Option<&str> {
    status.lines().find_map(|line| {
        line.strip_prefix("CapEff:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn dpkg_inconsistent() -> io::Result<()> {
    let output = Command::new("dpkg").arg("-l").output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    println!("{}", count_dpkg_bad(&text));
    Ok(())
}

fn count_dpkg_bad(output: &str) -> usize {
    output
        .lines()
        .skip(5)
        .filter(|line| {
            let Some(status) = line.split_whitespace().next() else {
                return false;
            };
            let bytes = status.as_bytes();
            bytes.len() >= 2 && bytes[1] != b'i' && bytes[1] != b'c'
        })
        .count()
}

fn one_arg<'a>(args: &'a [String], operation: &str, usage: &str) -> Result<&'a str, i32> {
    args.first()
        .filter(|_| args.len() == 1)
        .map(String::as_str)
        .ok_or_else(|| {
            eprintln!("runtime-probe server-parity {operation}: {usage}");
            64
        })
}

fn two_args<'a>(
    args: &'a [String],
    operation: &str,
    usage: &str,
) -> Result<(&'a str, &'a str), i32> {
    if args.len() != 2 {
        eprintln!("runtime-probe server-parity {operation}: {usage}");
        return Err(64);
    }
    Ok((&args[0], &args[1]))
}

fn finish(operation: &str, result: io::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{operation}: {error}");
            1
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let Some(operation) = args.first().map(String::as_str) else {
        eprintln!("usage: runtime-probe server-parity <operation> [...]");
        return 64;
    };
    let rest = &args[1..];
    match operation {
        "group-member" => match two_args(rest, operation, "<user> <group>") {
            Ok((user, group)) => finish(operation, group_member(user, group)),
            Err(code) => code,
        },
        "flock" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, flock_exclusive(path)),
            Err(code) => code,
        },
        "posixlock" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, posix_lock(path)),
            Err(code) => code,
        },
        "inotify" => finish(operation, inotify()),
        "xattr" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, xattr_roundtrip(path)),
            Err(code) => code,
        },
        "otmpfile" => match one_arg(rest, operation, "<dir>") {
            Ok(path) => finish(operation, o_tmpfile(path)),
            Err(code) => code,
        },
        "sparse" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, sparse(path)),
            Err(code) => code,
        },
        "mmap-shared" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, mmap_shared(path)),
            Err(code) => code,
        },
        "unix-stream" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, unix_stream(path)),
            Err(code) => code,
        },
        "unix-abstract" => match one_arg(rest, operation, "<name>") {
            Ok(name) => finish(operation, unix_abstract(name)),
            Err(code) => code,
        },
        "fdpass" => match one_arg(rest, operation, "<path>") {
            Ok(path) => finish(operation, fd_pass(path)),
            Err(code) => code,
        },
        "sysv-shm" => finish(operation, sysv_shm()),
        "posix-mq" => finish(operation, posix_mq()),
        "eventfd" => finish(operation, eventfd()),
        "timerfd" => finish(operation, timerfd()),
        "epoll" => finish(operation, epoll()),
        "pty" => finish(operation, openpty()),
        "getaddrinfo" => match two_args(rest, operation, "<host> <port>") {
            Ok((host, port)) => finish(operation, getaddrinfo(host, port)),
            Err(code) => code,
        },
        "tcp-outbound" => match two_args(rest, operation, "<address> <port>") {
            Ok((address, port)) => finish(operation, tcp_outbound(address, port)),
            Err(code) => code,
        },
        "tcp-listen" => finish(operation, tcp_listen()),
        "loopback" => finish(operation, loopback()),
        "udp" => finish(operation, udp()),
        "ipv6" => finish(operation, ipv6()),
        "netlink" => finish(operation, netlink()),
        "hostname" => finish(operation, hostname()),
        "icmp" => finish(operation, raw_icmp()),
        "dbus-session" => finish(operation, dbus_session()),
        "syslog" => match two_args(rest, operation, "<identifier> <message>") {
            Ok((identifier, message)) => finish(operation, syslog(identifier, message)),
            Err(code) => code,
        },
        "cap-eff" => match cap_eff() {
            Ok(value) => {
                println!("{value}");
                0
            }
            Err(error) => {
                eprintln!("{operation}: {error}");
                1
            }
        },
        "dpkg-inconsistent" => finish(operation, dpkg_inconsistent()),
        _ => {
            eprintln!("runtime-probe server-parity: unknown operation {operation}");
            64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{count_dpkg_bad, parse_cap_eff, parse_dbus_output};

    #[test]
    fn cap_eff_parser_ignores_other_lines() {
        let status = "Name:\tprobe\nCapInh:\t0\nCapEff:\t0000000000000000\n";
        assert_eq!(parse_cap_eff(status), Some("0000000000000000"));
        assert_eq!(parse_cap_eff("Name:\tprobe\n"), None);
    }

    #[test]
    fn dbus_parser_accepts_address_then_pid() {
        assert_eq!(
            parse_dbus_output("unix:path=/tmp/dbus-abc\n1234\n"),
            Some(("unix:path=/tmp/dbus-abc".to_owned(), 1234))
        );
        assert_eq!(parse_dbus_output("not-a-bus\n"), None);
    }

    #[test]
    fn dpkg_parser_matches_the_old_awk_rule() {
        let listing = "\
Desired=Unknown/Install/Remove/Purge/Hold
| Status=Not/Inst/Conf-files/Unpacked/halF-conf/Half-inst/trig-aWait/Trig-pend
|/ Err?=(none)/Reinst-required (Status,Err: uppercase=bad)
||/ Name Version Architecture Description
+++-====-=======-============-===========
ii  good 1.0 arm64 okay
hi  held 1.0 arm64 okay
rc  removed 1.0 arm64 conffiles only
iU  broken 1.0 arm64 unpacked
";
        assert_eq!(count_dpkg_bad(listing), 1);
    }
}
