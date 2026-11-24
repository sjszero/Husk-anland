//! Acceptance harness for the guest `/dev/videoN` emulation.
//!
//! Mirrors DroidNative's `tests/host/v4l2/run.sh` list (see
//! docs/design/v4l2-prior-art.md): stat says char 81:N, QUERYCAP, ENUM_FMT,
//! ENUM_FRAMESIZES, S_FMT, mmap streaming with REQBUFS/QUERYBUF/QBUF/DQBUF.
//!
//! usage: v4l2-test [dev] [frames] [fourcc] [out-prefix]
//!   v4l2-test /dev/video0 20 NV12 /tmp/frame

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
const ENUM_FRAMESIZES: u64 = 0xc02c_564a;

const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FMT_NV12: u32 = 0x3231_564e;
const FMT_YUYV: u32 = 0x5659_5559;

unsafe fn u32_at(p: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes(p[o..o + 4].try_into().unwrap())
}
fn put_u32(p: &mut [u8], o: usize, v: u32) {
    p[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}
fn put_u64(p: &mut [u8], o: usize, v: u64) {
    p[o..o + 8].copy_from_slice(&v.to_ne_bytes());
}
fn cstr(p: &[u8]) -> String {
    let e = p.iter().position(|c| *c == 0).unwrap_or(p.len());
    String::from_utf8_lossy(&p[..e]).into_owned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dev = args.get(1).cloned().unwrap_or("/dev/video0".into());
    let frames: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(10);
    let fourcc_arg = args.get(3).cloned().unwrap_or("NV12".into());
    let out = args.get(4).cloned().unwrap_or("/tmp/husk-v4l2".into());
    let fourcc = match fourcc_arg.as_str() {
        "YUYV" => FMT_YUYV,
        _ => FMT_NV12,
    };
    unsafe { run(&dev, frames, fourcc, &out) }
}

unsafe fn run(dev: &str, frames: usize, fourcc: u32, out: &str) {
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

    // 6. S_FMT 640x480
    let mut fmt = [0u8; 208];
    put_u32(&mut fmt, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut fmt, 8, 640);
    put_u32(&mut fmt, 12, 480);
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
