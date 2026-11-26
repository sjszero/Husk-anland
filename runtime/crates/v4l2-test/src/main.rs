//! Acceptance harness for the guest `/dev/videoN` emulation.
//!
//! Mirrors DroidNative's `tests/host/v4l2/run.sh` list (see
//! docs/design/v4l2-prior-art.md): stat says char 81:N, QUERYCAP, ENUM_FMT,
//! ENUM_FRAMESIZES, S_FMT, mmap streaming with REQBUFS/QUERYBUF/QBUF/DQBUF.
//!
//! usage: v4l2-test [dev] [frames] [fourcc] [out-prefix] [width] [height]
//!   v4l2-test /dev/video0 20 NV12 /tmp/frame 1280 720

use std::ffi::CString;

const QUERYCAP: u64 = 0x8068_5600;
const ENUM_FMT: u64 = 0xc040_5602;
const G_FMT: u64 = 0xc0d0_5604;
const S_FMT: u64 = 0xc0d0_5605;
const REQBUFS: u64 = 0xc014_5608;
const QUERYBUF: u64 = 0xc058_5609;
const QBUF: u64 = 0xc058_560f;
const DQBUF: u64 = 0xc058_5611;
const STREAMON: u64 = 0x4004_5612;
const STREAMOFF: u64 = 0x4004_5613;
const CROPCAP: u64 = 0xc02c_563a;
const G_CROP: u64 = 0xc014_563b;
const S_CROP: u64 = 0x4014_563c;
const ENUM_FRAMESIZES: u64 = 0xc02c_564a;
const G_SELECTION: u64 = 0xc040_565e;

const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const CAP_READWRITE: u32 = 0x0100_0000;
const CAP_STREAMING: u32 = 0x0400_0000;
const FMT_NV12: u32 = 0x3231_564e;
const FMT_YUYV: u32 = 0x5659_5559;
const SEL_TGT_CROP: u32 = 0;
const SEL_TGT_CROP_DEFAULT: u32 = 1;
const SEL_TGT_CROP_BOUNDS: u32 = 2;
const BUF_FLAG_TIMESTAMP_MASK: u32 = 0x0000_e000;
const BUF_FLAG_TIMESTAMP_MONOTONIC: u32 = 0x0000_2000;
const TIMESTAMP_SKEW_NS: u64 = 10_000_000_000;

unsafe fn u32_at(p: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes(p[o..o + 4].try_into().unwrap())
}
fn put_u32(p: &mut [u8], o: usize, v: u32) {
    p[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}
fn cstr(p: &[u8]) -> String {
    let e = p.iter().position(|c| *c == 0).unwrap_or(p.len());
    String::from_utf8_lossy(&p[..e]).into_owned()
}

unsafe fn check_rect(p: &[u8], off: usize, w: u32, h: u32, label: &str) {
    let (left, top, rw, rh) = (
        u32_at(p, off),
        u32_at(p, off + 4),
        u32_at(p, off + 8),
        u32_at(p, off + 12),
    );
    if (left, top, rw, rh) != (0, 0, w, h) {
        eprintln!("v4l2test|FAIL|{label}|rect={left},{top},{rw}x{rh}|want=0,0,{w}x{h}");
        std::process::exit(1);
    }
}

/// Crop/selection is not needed by every application, but GStreamer probes it before it
/// starts streaming. Returning ENOTTY here made two standard warnings look like a broken
/// camera. A capture facade owns the full frame, so bounds/default/current must all describe
/// that same frame and setting the identical rectangle must be accepted.
unsafe fn check_crop_contract(fd: libc::c_int, w: u32, h: u32) {
    let mut cap = [0u8; 44];
    put_u32(&mut cap, 0, BUF_TYPE_VIDEO_CAPTURE);
    if libc::ioctl(fd, CROPCAP as _, cap.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|CROPCAP: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    check_rect(&cap, 4, w, h, "CROPCAP bounds");
    check_rect(&cap, 20, w, h, "CROPCAP defrect");
    if (u32_at(&cap, 36), u32_at(&cap, 40)) != (1, 1) {
        eprintln!(
            "v4l2test|FAIL|CROPCAP pixelaspect={}/{}",
            u32_at(&cap, 36),
            u32_at(&cap, 40)
        );
        std::process::exit(1);
    }

    for target in [SEL_TGT_CROP_BOUNDS, SEL_TGT_CROP_DEFAULT, SEL_TGT_CROP] {
        let mut sel = [0u8; 64];
        put_u32(&mut sel, 0, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut sel, 4, target);
        if libc::ioctl(fd, G_SELECTION as _, sel.as_mut_ptr()) != 0 {
            eprintln!(
                "v4l2test|FAIL|G_SELECTION target={target}: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        check_rect(&sel, 12, w, h, "G_SELECTION crop");
    }

    let mut crop = [0u8; 20];
    put_u32(&mut crop, 0, BUF_TYPE_VIDEO_CAPTURE);
    if libc::ioctl(fd, G_CROP as _, crop.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|G_CROP: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    check_rect(&crop, 4, w, h, "G_CROP");

    put_u32(&mut crop, 12, w);
    put_u32(&mut crop, 16, h);
    if libc::ioctl(fd, S_CROP as _, crop.as_mut_ptr()) != 0 {
        eprintln!(
            "v4l2test|FAIL|S_CROP full-frame: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    println!("v4l2test|crop|bounds=yes|default=yes|selection=yes|full-crop=yes");
}

unsafe fn check_buffer_timestamp(buf: &[u8]) {
    let flags = u32_at(buf, 12);
    let ts_s = u64::from_ne_bytes(buf[24..32].try_into().unwrap());
    let ts_us = u64::from_ne_bytes(buf[32..40].try_into().unwrap());
    let stamp = ts_s
        .saturating_mul(1_000_000_000)
        .saturating_add(ts_us.saturating_mul(1_000));
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) != 0 {
        eprintln!(
            "v4l2test|FAIL|CLOCK_MONOTONIC: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    let now_ns = (now.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(now.tv_nsec as u64);
    if flags & BUF_FLAG_TIMESTAMP_MASK != BUF_FLAG_TIMESTAMP_MONOTONIC {
        eprintln!("v4l2test|FAIL|timestamp flags={flags:#x} (want MONOTONIC)");
        std::process::exit(1);
    }
    if stamp == 0 || stamp > now_ns || now_ns - stamp > TIMESTAMP_SKEW_NS {
        eprintln!("v4l2test|FAIL|timestamp={stamp} now={now_ns} skew>10s or future");
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dev = args.get(1).cloned().unwrap_or("/dev/video0".into());
    let frames: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(10);
    let fourcc_arg = args.get(3).cloned().unwrap_or("NV12".into());
    let out = args.get(4).cloned().unwrap_or("/tmp/husk-v4l2".into());
    let width: u32 = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(640);
    let height: u32 = args.get(6).and_then(|v| v.parse().ok()).unwrap_or(480);
    let fourcc = match fourcc_arg.as_str() {
        "YUYV" => FMT_YUYV,
        _ => FMT_NV12,
    };
    match args.get(7).map(String::as_str) {
        Some("read") => unsafe { run_read(&dev, frames, fourcc, &out, width, height) },
        Some("readv") => unsafe { run_readv(&dev, frames, fourcc, &out, width, height) },
        Some("contract") => unsafe { run_contract(&dev, fourcc, width, height) },
        Some("nonblock") => unsafe { run_nonblock(&dev, fourcc, width, height) },
        _ => unsafe { run(&dev, frames, fourcc, &out, width, height) },
    }
}

/// The same facade through V4L2 read() streaming: no REQBUFS, no mmap.
unsafe fn run_read(dev: &str, frames: usize, fourcc: u32, out: &str, want_w: u32, want_h: u32) {
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(std::ffi::OsStr::new(dev).as_bytes()).unwrap();
    let mut st = std::mem::MaybeUninit::<libc::stat>::zeroed().assume_init();
    if libc::stat(cpath.as_ptr(), &mut st) != 0 {
        eprintln!(
            "v4l2test|FAIL|stat {dev}: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    let fd = libc::open(cpath.as_ptr(), libc::O_RDWR);
    if fd < 0 {
        eprintln!("v4l2test|FAIL|open: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, want_w);
    put_u32(&mut fmt, 12, want_h);
    put_u32(&mut fmt, 16, fourcc);
    if libc::ioctl(fd, S_FMT as _, fmt.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|S_FMT: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let (w, h) = (u32_at(&fmt, 8), u32_at(&fmt, 12));
    let sizeimage = u32_at(&fmt, 28) as usize;
    println!(
        "v4l2test|read|setfmt|{w}x{h}|{:08x}|sizeimage={sizeimage}",
        u32_at(&fmt, 16)
    );
    let mut frame = vec![0u8; sizeimage];
    let started = std::time::Instant::now();
    for k in 0..frames {
        let n = libc::read(fd, frame.as_mut_ptr().cast(), frame.len());
        if n < 0 {
            eprintln!(
                "v4l2test|FAIL|read {k}: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        if n as usize != sizeimage {
            eprintln!("v4l2test|FAIL|read {k}: {n} != {sizeimage}");
            std::process::exit(1);
        }
        if k == 0 || k + 1 == frames {
            let min = *frame.iter().min().unwrap();
            let max = *frame.iter().max().unwrap();
            let mean = frame.iter().map(|b| *b as u64).sum::<u64>() as f64 / frame.len() as f64;
            println!("v4l2test|read|frame|{k}|bytes={n}|min={min}|max={max}|mean={mean:.2}");
        }
    }
    let secs = started.elapsed().as_secs_f64().max(0.001);
    println!(
        "v4l2test|read|done|frames={frames}|bytes={}|fps={:.2}",
        sizeimage * frames,
        frames as f64 / secs
    );
    let raw = format!("{out}.read.raw");
    let _ = std::fs::write(&raw, &frame);
    if fourcc == FMT_NV12 && frame.len() >= (w * h * 3 / 2) as usize {
        let ppm = format!("{out}.read.ppm");
        let _ = std::fs::write(&ppm, nv12_to_ppm(&frame, w as usize, h as usize));
        println!("v4l2test|read|saved|{raw}|{ppm}");
    } else {
        println!("v4l2test|read|saved|{raw}");
    }
    libc::close(fd);
    println!("v4l2test|read|OK");
}

/// readv() must gather one frame across the caller's iovec. A plain readv on the
/// backing memfd would return EOF, which is the failure this mode detects.
unsafe fn run_readv(dev: &str, frames: usize, fourcc: u32, out: &str, want_w: u32, want_h: u32) {
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(std::ffi::OsStr::new(dev).as_bytes()).unwrap();
    let fd = libc::open(cpath.as_ptr(), libc::O_RDWR);
    if fd < 0 {
        eprintln!("v4l2test|FAIL|open: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, want_w);
    put_u32(&mut fmt, 12, want_h);
    put_u32(&mut fmt, 16, fourcc);
    if libc::ioctl(fd, S_FMT as _, fmt.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|S_FMT: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let (w, h) = (u32_at(&fmt, 8), u32_at(&fmt, 12));
    let sizeimage = u32_at(&fmt, 28) as usize;
    let mut frame = vec![0u8; sizeimage];
    let split1 = 4096.min(sizeimage);
    let split2 = (split1 + 32768).min(sizeimage);
    let iov = [
        libc::iovec {
            iov_base: frame.as_mut_ptr().cast(),
            iov_len: split1,
        },
        libc::iovec {
            iov_base: frame.as_mut_ptr().add(split1).cast(),
            iov_len: split2 - split1,
        },
        libc::iovec {
            iov_base: frame.as_mut_ptr().add(split2).cast(),
            iov_len: sizeimage - split2,
        },
    ];
    println!(
        "v4l2test|readv|setfmt|{w}x{h}|{:08x}|sizeimage={sizeimage}|iov=3",
        u32_at(&fmt, 16)
    );
    let started = std::time::Instant::now();
    for k in 0..frames {
        let n = libc::readv(fd, iov.as_ptr(), iov.len() as i32);
        if n < 0 {
            eprintln!(
                "v4l2test|FAIL|readv {k}: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        if n as usize != sizeimage {
            eprintln!("v4l2test|FAIL|readv {k}: {n} != {sizeimage}");
            std::process::exit(1);
        }
        if k == 0 || k + 1 == frames {
            let min = *frame.iter().min().unwrap();
            let max = *frame.iter().max().unwrap();
            let mean = frame.iter().map(|b| *b as u64).sum::<u64>() as f64 / frame.len() as f64;
            println!("v4l2test|readv|frame|{k}|bytes={n}|min={min}|max={max}|mean={mean:.2}");
        }
    }
    let secs = started.elapsed().as_secs_f64().max(0.001);
    println!(
        "v4l2test|readv|done|frames={frames}|bytes={}|fps={:.2}",
        sizeimage * frames,
        frames as f64 / secs
    );
    let raw = format!("{out}.readv.raw");
    let _ = std::fs::write(&raw, &frame);
    println!("v4l2test|readv|saved|{raw}");
    libc::close(fd);
    println!("v4l2test|readv|OK");
}

/// Standards checks that do not need a sensor: QUERYCAP advertises read I/O,
/// short read buffers are EINVAL, and mixing read with REQBUFS is EINVAL.
unsafe fn run_contract(dev: &str, fourcc: u32, want_w: u32, want_h: u32) {
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(std::ffi::OsStr::new(dev).as_bytes()).unwrap();
    let fd = libc::open(cpath.as_ptr(), libc::O_RDWR);
    if fd < 0 {
        eprintln!("v4l2test|FAIL|open: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let mut cap = [0u8; 104];
    if libc::ioctl(fd, QUERYCAP as _, cap.as_mut_ptr()) != 0 {
        eprintln!(
            "v4l2test|FAIL|QUERYCAP: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    let caps = u32_at(&cap, 84);
    if caps & (CAP_READWRITE | CAP_STREAMING) != (CAP_READWRITE | CAP_STREAMING) {
        eprintln!("v4l2test|FAIL|caps={caps:#x} missing READWRITE/STREAMING");
        std::process::exit(1);
    }
    println!("v4l2test|contract|caps={caps:#x}|readwrite=yes|streaming=yes");

    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, want_w);
    put_u32(&mut fmt, 12, want_h);
    put_u32(&mut fmt, 16, fourcc);
    if libc::ioctl(fd, S_FMT as _, fmt.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|S_FMT: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let sizeimage = u32_at(&fmt, 28) as usize;
    let mut short = vec![0u8; sizeimage.saturating_sub(1)];
    let n = libc::read(fd, short.as_mut_ptr().cast(), short.len());
    if n != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
        eprintln!(
            "v4l2test|FAIL|short read: n={n} errno={:?}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    println!("v4l2test|contract|short-read|EINVAL=yes");

    let mut req = [0u8; 20];
    put_u32(&mut req, 0, 1);
    put_u32(&mut req, 4, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut req, 8, MEMORY_MMAP);
    if libc::ioctl(fd, REQBUFS as _, req.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|REQBUFS: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let mut frame = vec![0u8; sizeimage];
    let n = libc::read(fd, frame.as_mut_ptr().cast(), frame.len());
    if n != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
        eprintln!(
            "v4l2test|FAIL|read+REQBUFS: n={n} errno={:?}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    println!("v4l2test|contract|read+reqbufs|EINVAL=yes");
    libc::close(fd);
    println!("v4l2test|contract|OK");
}

/// With a frame source that answers NOFRAME, O_NONBLOCK must surface EAGAIN
/// immediately instead of the shim busy-looping until a frame appears.
unsafe fn run_nonblock(dev: &str, fourcc: u32, want_w: u32, want_h: u32) {
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(std::ffi::OsStr::new(dev).as_bytes()).unwrap();
    let fd = libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK);
    if fd < 0 {
        eprintln!("v4l2test|FAIL|open: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, want_w);
    put_u32(&mut fmt, 12, want_h);
    put_u32(&mut fmt, 16, fourcc);
    if libc::ioctl(fd, S_FMT as _, fmt.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|S_FMT: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let sizeimage = u32_at(&fmt, 28) as usize;
    let mut frame = vec![0u8; sizeimage];
    let started = std::time::Instant::now();
    let n = libc::read(fd, frame.as_mut_ptr().cast(), frame.len());
    let elapsed = started.elapsed();
    if n != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EAGAIN) {
        eprintln!(
            "v4l2test|FAIL|nonblock read: n={n} errno={:?}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    println!(
        "v4l2test|nonblock|EAGAIN=yes|elapsed_ms={:.2}",
        elapsed.as_secs_f64() * 1000.0
    );
    libc::close(fd);
    println!("v4l2test|nonblock|OK");
}

unsafe fn run(dev: &str, frames: usize, fourcc: u32, out: &str, want_w: u32, want_h: u32) {
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(std::ffi::OsStr::new(dev).as_bytes()).unwrap();

    // 1. stat
    let mut st = std::mem::MaybeUninit::<libc::stat>::zeroed().assume_init();
    if libc::stat(cpath.as_ptr(), &mut st) != 0 {
        eprintln!(
            "v4l2test|FAIL|stat {dev}: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    let mode = st.st_mode & libc::S_IFMT;
    let major = libc::major(st.st_rdev);
    let minor = libc::minor(st.st_rdev);
    let chr = mode == libc::S_IFCHR;
    println!("v4l2test|stat|chr={chr}|rdev={major}:{minor}");
    if !chr || major != 81 {
        eprintln!("v4l2test|FAIL|not a character device 81:N");
        std::process::exit(1);
    }

    // 2. open
    let fd = libc::open(cpath.as_ptr(), libc::O_RDWR);
    if fd < 0 {
        eprintln!("v4l2test|FAIL|open: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }

    // 3. QUERYCAP
    let mut cap = [0u8; 104];
    if libc::ioctl(fd, QUERYCAP as _, cap.as_mut_ptr()) != 0 {
        eprintln!(
            "v4l2test|FAIL|QUERYCAP: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    println!(
        "v4l2test|querycap|driver={}|card={}|caps=0x{:x}",
        cstr(&cap[0..16]),
        cstr(&cap[16..48]),
        u32_at(&cap, 88)
    );

    // 4. ENUM_FMT
    for i in 0..8u32 {
        let mut d = [0u8; 64];
        put_u32(&mut d, 0, i);
        put_u32(&mut d, 4, BUF_TYPE_VIDEO_CAPTURE);
        if libc::ioctl(fd, ENUM_FMT as _, d.as_mut_ptr()) != 0 {
            break;
        }
        println!(
            "v4l2test|fmt|{}|{:08x}|{}",
            cstr(&d[12..44]),
            u32_at(&d, 44),
            u32_at(&d, 44)
        );
    }

    // 5. ENUM_FRAMESIZES
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    for i in 0..64u32 {
        let mut d = [0u8; 44];
        put_u32(&mut d, 0, i);
        put_u32(&mut d, 4, fourcc);
        if libc::ioctl(fd, ENUM_FRAMESIZES as _, d.as_mut_ptr()) != 0 {
            break;
        }
        sizes.push((u32_at(&d, 12), u32_at(&d, 16)));
    }
    println!(
        "v4l2test|sizes|n={}|{:?}",
        sizes.len(),
        &sizes[..sizes.len().min(4)]
    );
    if sizes.is_empty() {
        eprintln!("v4l2test|FAIL|no frame sizes");
        std::process::exit(1);
    }

    // 6. S_FMT requested size
    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, want_w);
    put_u32(&mut fmt, 12, want_h);
    put_u32(&mut fmt, 16, fourcc);
    if libc::ioctl(fd, S_FMT as _, fmt.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|S_FMT: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let (w, h) = (u32_at(&fmt, 8), u32_at(&fmt, 12));
    let sizeimage = u32_at(&fmt, 28);
    println!(
        "v4l2test|setfmt|{w}x{h}|{:08x}|sizeimage={sizeimage}",
        u32_at(&fmt, 16)
    );
    check_crop_contract(fd, w, h);
    let mut got = [0u8; 208];
    put_u32(&mut got, 0, BUF_TYPE_VIDEO_CAPTURE);
    if libc::ioctl(fd, G_FMT as _, got.as_mut_ptr()) == 0 {
        println!(
            "v4l2test|getfmt|{}x{}|{:08x}|sizeimage={}",
            u32_at(&got, 8),
            u32_at(&got, 12),
            u32_at(&got, 16),
            u32_at(&got, 28)
        );
    }

    // 7. REQBUFS
    const N: usize = 4;
    let mut req = [0u8; 20];
    put_u32(&mut req, 0, N as u32);
    put_u32(&mut req, 4, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut req, 8, MEMORY_MMAP);
    if libc::ioctl(fd, REQBUFS as _, req.as_mut_ptr()) != 0 {
        eprintln!("v4l2test|FAIL|REQBUFS: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let n = u32_at(&req, 0) as usize;
    println!("v4l2test|reqbufs|{n}");

    // 8. QUERYBUF + mmap
    let mut bufs: Vec<(*mut u8, u32)> = Vec::new();
    for i in 0..n {
        let mut b = [0u8; 88];
        put_u32(&mut b, 0, i as u32);
        put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut b, 60, MEMORY_MMAP);
        if libc::ioctl(fd, QUERYBUF as _, b.as_mut_ptr()) != 0 {
            eprintln!("v4l2test|FAIL|QUERYBUF {i}");
            std::process::exit(1);
        }
        let len = u32_at(&b, 72) as usize;
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
            eprintln!(
                "v4l2test|FAIL|mmap {i}: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        println!("v4l2test|buf|{i}|len={len}|off={off}");
        bufs.push((p as *mut u8, len as u32));
    }

    // 9. QBUF all
    for i in 0..n {
        let mut b = [0u8; 88];
        put_u32(&mut b, 0, i as u32);
        put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut b, 60, MEMORY_MMAP);
        if libc::ioctl(fd, QBUF as _, b.as_mut_ptr()) != 0 {
            eprintln!("v4l2test|FAIL|QBUF {i}");
            std::process::exit(1);
        }
    }

    // 10. STREAMON
    let mut type_ = BUF_TYPE_VIDEO_CAPTURE;
    if libc::ioctl(fd, STREAMON as _, &mut type_) != 0 {
        eprintln!(
            "v4l2test|FAIL|STREAMON: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    println!("v4l2test|streamon");

    // 11. capture
    let mut total_bytes: u64 = 0;
    let mut last_frame: Option<Vec<u8>> = None;
    let started = std::time::Instant::now();
    for k in 0..frames {
        let mut b = [0u8; 88];
        put_u32(&mut b, 0, 0);
        put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut b, 60, MEMORY_MMAP);
        let mut tries = 0;
        loop {
            if libc::ioctl(fd, DQBUF as _, b.as_mut_ptr()) == 0 {
                break;
            }
            tries += 1;
            if tries > 3 {
                eprintln!(
                    "v4l2test|FAIL|DQBUF {k}: {}",
                    std::io::Error::last_os_error()
                );
                std::process::exit(1);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let idx = u32_at(&b, 0) as usize;
        let used = u32_at(&b, 8);
        let seq = u32_at(&b, 56);
        let ts_s = u64::from_ne_bytes(b[24..32].try_into().unwrap());
        let ts_us = u64::from_ne_bytes(b[32..40].try_into().unwrap());
        if k == 0 {
            check_buffer_timestamp(&b);
        }
        let src = std::slice::from_raw_parts(bufs[idx].0, used as usize);
        total_bytes += used as u64;
        if k == 0 || k + 1 == frames {
            println!("v4l2test|frame|{k}|idx={idx}|bytes={used}|seq={seq}|ts={ts_s}.{ts_us:06}");
        }
        last_frame = Some(src.to_vec());
        // requeue
        let mut q = [0u8; 88];
        put_u32(&mut q, 0, idx as u32);
        put_u32(&mut q, 4, BUF_TYPE_VIDEO_CAPTURE);
        put_u32(&mut q, 60, MEMORY_MMAP);
        if libc::ioctl(fd, QBUF as _, q.as_mut_ptr()) != 0 {
            eprintln!("v4l2test|FAIL|requeue {idx}");
            std::process::exit(1);
        }
    }
    let secs = started.elapsed().as_secs_f64().max(0.001);
    println!(
        "v4l2test|done|frames={frames}|bytes={total_bytes}|fps={:.2}",
        frames as f64 / secs
    );

    // 12. save
    if let Some(f) = last_frame {
        let raw = format!("{out}.raw");
        let _ = std::fs::write(&raw, &f);
        if fourcc == FMT_NV12 && f.len() >= (w * h * 3 / 2) as usize {
            let ppm = format!("{out}.ppm");
            let _ = std::fs::write(&ppm, nv12_to_ppm(&f, w as usize, h as usize));
            println!("v4l2test|saved|{raw}|{ppm}");
        } else {
            println!("v4l2test|saved|{raw}");
        }
    }

    let _ = libc::ioctl(fd, STREAMOFF as _, &mut type_);
    println!("v4l2test|streamoff");
    libc::close(fd);
    println!("v4l2test|OK");
}

fn nv12_to_ppm(f: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.reserve(w * h * 3);
    let y = &f[..w * h];
    let uv = &f[w * h..];
    for row in 0..h {
        for col in 0..w {
            let yy = y[row * w + col] as f32;
            let u = uv[(row / 2) * w + (col & !1)] as f32 - 128.0;
            let v = uv[(row / 2) * w + (col | 1)] as f32 - 128.0;
            let c = (yy - 16.0).max(0.0);
            let r = (1.164 * c + 1.596 * v).clamp(0.0, 255.0) as u8;
            let g = (1.164 * c - 0.392 * u - 0.813 * v).clamp(0.0, 255.0) as u8;
            let b = (1.164 * c + 2.017 * u).clamp(0.0, 255.0) as u8;
            out.push(r);
            out.push(g);
            out.push(b);
        }
    }
    out
}
