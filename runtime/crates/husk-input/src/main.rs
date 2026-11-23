//! husk-input — decode the app's evdev stream inside the guest.
//!
//! Two uses:
//!   husk-input <socket> [--count N] [--raw]   decode and print
//!   husk-input <socket> --fifo <path>         replay into a FIFO that mutter's
//!                                             libinput opens as /dev/input/eventN
//!
//! Records are 24-byte `struct input_event` (aarch64), preceded by one text
//! header line `INPUT w=<px> h=<px> max=<n>`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: husk-input <socket> [--count N] [--raw] [--fifo path]");
        std::process::exit(2);
    }
    let socket = &args[1];
    let mut count = u64::MAX;
    let mut raw = false;
    let mut fifo: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--count" => {
                count = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(count);
                i += 2;
            }
            "--raw" => {
                raw = true;
                i += 1;
            }
            "--fifo" => {
                fifo = args.get(i + 1).cloned();
                i += 2;
            }
            other => {
                eprintln!("unknown arg {other}");
                std::process::exit(2);
            }
        }
    }

    let stream = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("input|FAIL|connect {socket}: {e}");
            std::process::exit(1);
        }
    };
    // NB: records must be read through the BufReader. Reading `r.get_mut()`
    // (the raw socket) after read_line() silently discards whatever the header
    // read had already buffered — that is what made the first events vanish.
    let mut r = BufReader::new(stream);
    let mut header = String::new();
    if r.read_line(&mut header).is_err() || !header.starts_with("INPUT ") {
        eprintln!("input|FAIL|no header: {header}");
        std::process::exit(1);
    }
    println!("input|header|{}", header.trim_end());

    let mut sink: Box<dyn Write> = match &fifo {
        Some(path) => match std::fs::OpenOptions::new().write(true).open(path) {
            Ok(f) => {
                println!("input|fifo|{path}");
                Box::new(f)
            }
            Err(e) => {
                eprintln!("input|FAIL|fifo {path}: {e}");
                std::process::exit(1);
            }
        },
        None => Box::new(std::io::sink()),
    };

    let mut rec = [0u8; 24];
    let mut seen = 0u64;
    loop {
        if let Err(e) = read_full(&mut r, &mut rec) {
            if seen > 0 {
                break;
            }
            eprintln!("input|FAIL|read: {e}");
            std::process::exit(1);
        }
        let ty = u16::from_ne_bytes([rec[16], rec[17]]);
        let code = u16::from_ne_bytes([rec[18], rec[19]]);
        let value = i32::from_ne_bytes([rec[20], rec[21], rec[22], rec[23]]);
        if fifo.is_some() {
            let _ = sink.write_all(&rec);
        }
        if raw || ty != EV_SYN {
            println!("input|ev|{}|{}|{}", type_name(ty), code_name(ty, code), value);
        }
        if ty != EV_SYN {
            seen += 1;
            if seen >= count {
                break;
            }
        }
    }
    println!("input|done|{seen}");
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<()> {
    let mut off = 0;
    while off < buf.len() {
        let n = r.read(&mut buf[off..])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "eof",
            ));
        }
        off += n;
    }
    Ok(())
}

fn type_name(t: u16) -> &'static str {
    match t {
        EV_SYN => "SYN",
        EV_KEY => "KEY",
        EV_REL => "REL",
        EV_ABS => "ABS",
        _ => "?",
    }
}

fn code_name(t: u16, c: u16) -> String {
    if t == EV_ABS {
        return match c {
            0 => "ABS_X".into(),
            1 => "ABS_Y".into(),
            0x2f => "ABS_MT_SLOT".into(),
            0x39 => "ABS_MT_TRACKING_ID".into(),
            _ => format!("ABS_{c}"),
        };
    }
    if t == EV_REL {
        return match c {
            8 => "REL_WHEEL".into(),
            6 => "REL_HWHEEL".into(),
            _ => format!("REL_{c}"),
        };
    }
    if t == EV_KEY {
        return match c {
            0x110 => "BTN_LEFT".into(),
            0x111 => "BTN_RIGHT".into(),
            0x112 => "BTN_MIDDLE".into(),
            28 => "KEY_ENTER".into(),
            14 => "KEY_BACKSPACE".into(),
            1 => "KEY_ESC".into(),
            15 => "KEY_TAB".into(),
            57 => "KEY_SPACE".into(),
            42 => "KEY_LEFTSHIFT".into(),
            29 => "KEY_LEFTCTRL".into(),
            103 => "KEY_UP".into(),
            108 => "KEY_DOWN".into(),
            105 => "KEY_LEFT".into(),
            106 => "KEY_RIGHT".into(),
            30..=55 => return format!("KEY_{}", (b'A' + (c - 30) as u8) as char),
            2..=11 => return format!("KEY_{}", (c - 1) % 10),
            _ => format!("KEY_{c}"),
        };
    }
    format!("{c}")
}
