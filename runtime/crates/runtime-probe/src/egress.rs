//! Outbound network probe used by the device egress gate.
//!
//! The old shell check invoked Python for DNS/socket work and curl/ping for the
//! other paths. This probe keeps the checks in Rust so the gate does not depend
//! on those guest packages, while still going through the same libc socket,
//! connect, send and TLS entry points as an ordinary guest program.

#[cfg(target_os = "linux")]
use std::io::{self, Read, Write};
#[cfg(target_os = "linux")]
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
const HOST: &str = "ports.ubuntu.com";
#[cfg(target_os = "linux")]
const DNS_HOST: &str = "one.one.one.one";
#[cfg(target_os = "linux")]
const PORT: u16 = 80;
#[cfg(target_os = "linux")]
const HTTPS_PORT: u16 = 443;
#[cfg(target_os = "linux")]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(target_os = "linux")]
const IO_TIMEOUT: Duration = Duration::from_secs(8);

#[cfg(target_os = "linux")]
fn request(host: &str) -> String {
    format!("GET /ubuntu-ports/dists/noble/Release HTTP/1.0\r\nHost: {host}\r\n\r\n")
}

fn response_ok(sent: usize, expected: usize, head: &[u8]) -> bool {
    sent == expected && head.starts_with(b"HTTP/1.")
}

#[cfg(target_os = "linux")]
fn resolve(host: &str, port: u16) -> io::Result<SocketAddr> {
    (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no address resolved"))
}

#[cfg(target_os = "linux")]
fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
}

#[cfg(target_os = "linux")]
fn read_http_head(stream: &mut impl Read) -> io::Result<[u8; 64]> {
    let mut head = [0_u8; 64];
    let received = stream.read(&mut head)?;
    if received == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty response",
        ));
    }
    Ok(head)
}

#[cfg(target_os = "linux")]
fn tls_http(addr: SocketAddr) -> io::Result<[u8; 64]> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(HOST.to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let stream = connect(addr)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut stream = rustls::StreamOwned::new(connection, stream);
    stream.write_all(request(HOST).as_bytes())?;
    stream.flush()?;
    read_http_head(&mut stream)
}

#[cfg(target_os = "linux")]
fn icmp_result() -> io::Result<()> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        println!("ICMP raw socket allowed");
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EPERM | libc::EACCES | libc::EPROTONOSUPPORT) => {
            println!("ICMP raw socket refused ({error}; documented platform limit)");
            Ok(())
        }
        _ => Err(error),
    }
}

#[cfg(target_os = "linux")]
pub fn run() -> i32 {
    let ports_addr = match resolve(HOST, PORT) {
        Ok(addr) => addr,
        Err(error) => {
            eprintln!("DNS {HOST} failed: {error}");
            return 3;
        }
    };
    println!("DNS {HOST} {ports_addr}");
    match resolve(DNS_HOST, HTTPS_PORT) {
        Ok(addr) => println!("DNS {DNS_HOST} {addr}"),
        Err(error) => {
            eprintln!("DNS {DNS_HOST} failed: {error}");
            return 3;
        }
    }

    let stream = match connect(ports_addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("connect failed: {error}");
            return 3;
        }
    };
    if let Err(error) = stream.set_read_timeout(Some(IO_TIMEOUT)) {
        eprintln!("set read timeout failed: {error}");
        return 4;
    }

    let request = request(HOST);
    let sent = unsafe {
        libc::send(
            stream.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
        )
    };
    if sent < 0 {
        eprintln!("send failed: {}", io::Error::last_os_error());
        return 4;
    }
    println!("send returned {sent}");

    let mut head = [0_u8; 64];
    let received =
        unsafe { libc::recv(stream.as_raw_fd(), head.as_mut_ptr().cast(), head.len(), 0) };
    if received < 0 {
        eprintln!("recv failed: {}", io::Error::last_os_error());
        return 4;
    }
    let head = &head[..received as usize];
    if !response_ok(sent as usize, request.len(), head) {
        eprintln!(
            "bad HTTP response: {:?}",
            String::from_utf8_lossy(&head[..head.len().min(20)])
        );
        return 4;
    }

    let https_addr = match resolve(HOST, HTTPS_PORT) {
        Ok(addr) => addr,
        Err(error) => {
            eprintln!("DNS {HOST}:{HTTPS_PORT} failed: {error}");
            return 3;
        }
    };
    match tls_http(https_addr) {
        Ok(head) if head.starts_with(b"HTTP/1.") => {
            println!("HTTPS {}", String::from_utf8_lossy(&head[..12]));
        }
        Ok(head) => {
            eprintln!(
                "bad HTTPS response: {:?}",
                String::from_utf8_lossy(&head[..20])
            );
            return 4;
        }
        Err(error) => {
            eprintln!("HTTPS failed: {error}");
            return 4;
        }
    }

    if let Err(error) = icmp_result() {
        eprintln!("ICMP probe failed: {error}");
        return 4;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::response_ok;

    #[test]
    fn accepts_complete_http_response() {
        assert!(response_ok(7, 7, b"HTTP/1.1 200 OK\r\n"));
    }

    #[test]
    fn rejects_short_send() {
        assert!(!response_ok(6, 7, b"HTTP/1.1 200 OK\r\n"));
    }

    #[test]
    fn rejects_non_http_response() {
        assert!(!response_ok(7, 7, b"SSH-2.0\r\n"));
    }
}
