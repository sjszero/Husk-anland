//! husk-record — guest-side camera recorder over `/dev/videoN` + the app's
//! hardware encoder.
//!
//! Two hardware paths, no software codec anywhere:
//!   `/dev/videoN`  -> V4L2 shim -> Camera2 (NV12 in, ~12–30 fps at 640x480)
//!   encoder.sock   -> MediaCodec (`c2.mtk.avc.encoder`) -> Annex-B H.264
//!
//! Output: `.h264` = raw Annex-B elementary stream, `.mp4` = muxed here
//! (`mp4.rs`, because GStreamer's filesrc/filesink go through libc-internal
//! open, which no GOT patch reaches — measured, see docs/design/libc-interpose.md).
//!
//! usage: husk-record <socket> <dev> <seconds> <out.mp4|out.h264> [w] [h] [fps] [kbps]

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

mod mp4;

const S_FMT: u64 = 0xc0d0_5605;
const REQBUFS: u64 = 0xc014_5608;
const QUERYBUF: u64 = 0xc058_5609;
const QBUF: u64 = 0xc058_560f;
const DQBUF: u64 = 0xc058_5611;
const STREAMON: u64 = 0x4004_5612;
const STREAMOFF: u64 = 0x4004_5613;

const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FMT_NV12: u32 = 0x3231_564e;

fn put_u32(p: &mut [u8], o: usize, v: u32) {
    p[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!(
            "usage: husk-record <socket> <dev> <seconds> <out.mp4|.h264> [w] [h] [fps] [kbps]"
        );
        std::process::exit(2);
    }
    let socket = args[1].clone();
    let dev = args[2].clone();
    let seconds: u64 = args[3].parse().unwrap_or(10);
    let out = args[4].clone();
    let w: u32 = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(640);
    let h: u32 = args.get(6).and_then(|v| v.parse().ok()).unwrap_or(480);
    let fps: u32 = args.get(7).and_then(|v| v.parse().ok()).unwrap_or(15);
    let kbps: u32 = args.get(8).and_then(|v| v.parse().ok()).unwrap_or(2000);
    std::process::exit(run(&socket, &dev, seconds, &out, w, h, fps, kbps));
}

#[derive(Default)]
struct Collected {
    frames: Vec<Vec<u8>>,
    keys: Vec<bool>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    aus: u64,
    bytes: u64,
}

fn run(
    socket: &str,
    dev: &str,
    seconds: u64,
    out: &str,
    w: u32,
    h: u32,
    fps: u32,
    kbps: u32,
) -> i32 {
    let stream = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("record|FAIL|connect {socket}: {e}");
            return 1;
        }
    };
    let mut writer = stream.try_clone().expect("clone");
    let reader = stream;

    let cmd = format!("OPEN w={w} h={h} fps={fps} kbps={kbps} codec=avc\n");
    if writer.write_all(cmd.as_bytes()).is_err() {
        eprintln!("record|FAIL|open write");
        return 1;
    }
    let mut line = String::new();
    if !read_line(&mut writer.try_clone().expect("clone"), &mut line) || !line.starts_with("OK") {
        eprintln!("record|FAIL|open reply: {line}");
        return 1;
    }
    println!("record|OPEN|{w}x{h}|{fps}fps|{kbps}kbps");

    let reader_thread = std::thread::spawn(move || collect(reader));

    let began = Instant::now();
    let code = unsafe { capture(dev, w, h, seconds, &mut writer) };
    let elapsed = began.elapsed().as_secs_f64();
    let _ = writer.write_all(b"STOP\n");
    let _ = writer.flush();

    let collected = match reader_thread.join() {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            eprintln!("record|FAIL|reader {e}");
            return 1;
        }
        Err(_) => {
            eprintln!("record|FAIL|reader panicked");
            return 1;
        }
    };

    let n = collected.frames.len();
    let rate = if elapsed > 0.0 {
        n as f64 / elapsed
    } else {
        fps as f64
    };
    println!(
        "record|collect|aus={}|key={}|bytes={}|sps={}|pps={}",
        collected.aus,
        collected.keys.iter().filter(|k| **k).count(),
        collected.bytes,
        collected.sps.len(),
        collected.pps.len()
    );

    if out.ends_with(".mp4") {
        if collected.sps.is_empty() || collected.pps.is_empty() {
            eprintln!("record|FAIL|no SPS/PPS in the stream");
            return 1;
        }
        match mp4::write(
            out,
            w,
            h,
            rate,
            &collected.frames,
            &collected.keys,
            &collected.sps,
            &collected.pps,
        ) {
            Ok(bytes) => println!(
                "record|mp4|{out}|{bytes} bytes|{}x{}|{:.2} fps|{:.2} s",
                w,
                h,
                rate,
                n as f64 / rate.max(0.001)
            ),
            Err(e) => {
                eprintln!("record|FAIL|mp4 {e}");
                return 1;
            }
        }
    } else {
        let mut buf = Vec::new();
        if !collected.sps.is_empty() {
            buf.extend_from_slice(b"\x00\x00\x00\x01");
            buf.extend_from_slice(&collected.sps);
            buf.extend_from_slice(b"\x00\x00\x00\x01");
            buf.extend_from_slice(&collected.pps);
        }
        for f in &collected.frames {
            buf.extend_from_slice(f);
        }
        if let Err(e) = std::fs::write(out, &buf) {
            eprintln!("record|FAIL|write {out}: {e}");
            return 1;
        }
        println!("record|h264|{out}|{} bytes", buf.len());
    }
    println!(
        "record|done|frames={n}|{:.2}s|{:.2}fps|sent_ok={}",
        elapsed,
        rate,
        code == 0
    );
    code
}

/// Read AU records until BYE, keeping access units and parameter sets.
fn collect(mut r: UnixStream) -> std::io::Result<Collected> {
    let mut c = Collected::default();
    let mut csd_seen: u16 = 0;
    loop {
        let mut line = String::new();
        if !read_line(&mut r, &mut line) {
            break;
        }
        if line.starts_with("BYE") {
            break;
        }
        let Some(rest) = line.strip_prefix("AU ") else {
            if !line.is_empty() {
                eprintln!("record|app|{line}");
            }
            continue;
        };
        let mut it = rest.split_whitespace();
        let flags: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let size: usize = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut data = vec![0u8; size];
        if r.read_exact(&mut data).is_err() {
            break;
        }
        c.bytes += size as u64;
        if flags & 2 != 0 {
            // Parameter sets may arrive as several CSD buffers (c2.mtk.avc.encoder
            // sends SPS and PPS separately); keep each NAL type once.
            mp4::param_sets(&data, &mut c.sps, &mut c.pps);
            let mut fresh = false;
            for t in nal_types(&data) {
                if t < 16 && csd_seen & (1 << t) == 0 {
                    csd_seen |= 1 << t;
                    fresh = true;
                }
            }
            if !fresh {
                continue;
            }
            // Parameter sets live in avcC, not as samples: pushing them here
            // would shift every stss/stsz entry by one.
            continue;
        } else {
            c.aus += 1;
            c.keys.push(flags & 1 != 0);
        }
        c.frames.push(data);
    }
    Ok(c)
}

fn nal_types(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < d.len() {
        if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
            out.push(d[i + 3] & 0x1f);
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

fn read_line(r: &mut impl Read, out: &mut String) -> bool {
    out.clear();
    let mut one = [0u8; 1];
    loop {
        match r.read(&mut one) {
            Ok(0) => return false,
            Ok(_) => {
                if one[0] == b'\n' {
                    return true;
                }
                if one[0] != b'\r' {
                    out.push(one[0] as char);
                    if out.len() > 256 {
                        return false;
                    }
                }
            }
            Err(_) => return false,
        }
    }
}

unsafe fn capture(dev: &str, w: u32, h: u32, seconds: u64, out: &mut impl Write) -> i32 {
    const N: usize = 4;
    let cpath = match CString::new(dev) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let fd = libc::open(cpath.as_ptr(), libc::O_RDWR);
    if fd < 0 {
        eprintln!(
            "record|FAIL|open {dev}: {}",
            std::io::Error::last_os_error()
        );
        return 1;
    }

    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, w);
    put_u32(&mut fmt, 12, h);
    put_u32(&mut fmt, 16, FMT_NV12);
    put_u32(&mut fmt, 20, 1); // field = NONE
    if libc::ioctl(fd, S_FMT as _, fmt.as_mut_ptr()) != 0 {
        eprintln!("record|FAIL|S_FMT");
        return 1;
    }
    let (rw, rh) = (
        u32::from_ne_bytes(fmt[8..12].try_into().unwrap()),
        u32::from_ne_bytes(fmt[12..16].try_into().unwrap()),
    );
    println!("record|caps|{rw}x{rh}");

    let mut req = [0u8; 20];
    put_u32(&mut req, 0, N as u32);
    put_u32(&mut req, 4, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut req, 8, MEMORY_MMAP);
    if libc::ioctl(fd, REQBUFS as _, req.as_mut_ptr()) != 0 {
        eprintln!("record|FAIL|REQBUFS");
        return 1;
    }
    let n = u32::from_ne_bytes(req[0..4].try_into().unwrap()) as usize;

    let mut bufs: Vec<(*mut u8, usize)> = Vec::new();
    for i in 0..n {
        let mut b = [0u8; 88];
        put_u32(&mut b, 0, i as u32);
        put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut b, 60, MEMORY_MMAP);
        if libc::ioctl(fd, QUERYBUF as _, b.as_mut_ptr()) != 0 {
            eprintln!("record|FAIL|QUERYBUF {i}");
            return 1;
        }
        let len = u32::from_ne_bytes(b[72..76].try_into().unwrap()) as usize;
        let off = u64::from_ne_bytes(b[64..72].try_into().unwrap());
        let p = libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            off as libc::off_t,
        );
        if p == libc::MAP_FAILED {
            eprintln!("record|FAIL|mmap {i}");
            return 1;
        }
        bufs.push((p as *mut u8, len));
    }
    for i in 0..n {
        let mut b = [0u8; 88];
        put_u32(&mut b, 0, i as u32);
        put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut b, 60, MEMORY_MMAP);
        if libc::ioctl(fd, QBUF as _, b.as_mut_ptr()) != 0 {
            eprintln!("record|FAIL|QBUF {i}");
            return 1;
        }
    }
    let mut type_ = BUF_TYPE_VIDEO_CAPTURE;
    if libc::ioctl(fd, STREAMON as _, &mut type_) != 0 {
        eprintln!("record|FAIL|STREAMON");
        return 1;
    }

    let need = (rw * rh * 3 / 2) as usize;
    let start = Instant::now();
    let mut frames = 0u64;
    let mut errs = 0u64;
    while start.elapsed() < Duration::from_secs(seconds) {
        let mut b = [0u8; 88];
        put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut b, 60, MEMORY_MMAP);
        if libc::ioctl(fd, DQBUF as _, b.as_mut_ptr()) != 0 {
            errs += 1;
            if errs == 1 || errs % 200 == 0 {
                eprintln!("record|DQBUF|{}", std::io::Error::last_os_error());
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let idx = u32::from_ne_bytes(b[0..4].try_into().unwrap()) as usize;
        // struct v4l2_buffer (aarch64): bytesused at 8.
        let used = u32::from_ne_bytes(b[8..12].try_into().unwrap()) as usize;
        if idx < bufs.len() && used >= need {
            let header = format!("FRAME {need}\n");
            if out.write_all(header.as_bytes()).is_err()
                || out
                    .write_all(std::slice::from_raw_parts(bufs[idx].0, need))
                    .is_err()
            {
                eprintln!("record|FAIL|frame write");
                break;
            }
            frames += 1;
        }
        let mut q = [0u8; 88];
        put_u32(&mut q, 0, idx as u32);
        put_u32(&mut q, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut q, 60, MEMORY_MMAP);
        let _ = libc::ioctl(fd, QBUF as _, q.as_mut_ptr());
    }
    let _ = libc::ioctl(fd, STREAMOFF as _, &mut type_);
    let _ = libc::close(fd);
    println!(
        "record|sent|{frames}|frames|{:.2}s|dqb_err={errs}",
        start.elapsed().as_secs_f32()
    );
    0
}
