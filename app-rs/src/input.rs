//! Input bridge (app half): Android `MotionEvent`/`KeyEvent` -> Linux evdev
//! records for the guest desktop.
//!
//! Prior art read first (`docs/design/input-prior-art.md`): LinuxHub 2.0's Lorie
//! `TouchInputHandler` (1184 lines) and DroidNative's `TouchInput.kt` (262 lines)
//! both translate finger gestures into a *pointer* — tap = left click, drag =
//! left drag, long press = right click, two-finger vertical = wheel — instead of
//! forwarding raw multi-touch. That is the contract this module implements, so
//! the guest only ever needs two boring evdev devices (pointer + keyboard).
//!
//! Transport: `rootfs-full/run/husk/input.sock`. On connect the app sends one
//! text header (`INPUT w=<px> h=<px> max=32767\n`), then a stream of 24-byte
//! `struct input_event` records — the kernel ABI, not a private format, so the
//! guest side can either decode them itself or replay them into a FIFO that
//! mutter's libinput reads.

use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

const INPUT_SOCKET: &str = "rootfs-full/run/husk/input.sock";
const INPUT_MARKER: &str = "input";
const INPUT_GUEST_MARKER: &str = "rootfs-full/tmp/husk-input/enabled";

// evdev
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const ABS_MAX: i32 = 32767;

static STARTED: AtomicBool = AtomicBool::new(false);
static DIAG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static TX: Mutex<Option<Sender<Vec<u8>>>> = Mutex::new(None);
static STATE: Mutex<Gesture> = Mutex::new(Gesture::new());

/// Gesture state, mirroring the two prior implementations' MOUSE mode.
struct Gesture {
    down: bool,
    down_x: i32,
    down_y: i32,
    down_at: Option<std::time::Instant>,
    moved: bool,
    last_x: i32,
    last_y: i32,
    two_finger_y: i32,
    button_down: bool,
    width: i32,
    height: i32,
}

impl Gesture {
    const fn new() -> Self {
        Gesture {
            down: false,
            down_x: 0,
            down_y: 0,
            down_at: None,
            moved: false,
            last_x: 0,
            last_y: 0,
            two_finger_y: 0,
            button_down: false,
            width: 0,
            height: 0,
        }
    }
}

pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let (tx, rx) = channel::<Vec<u8>>();
    if let Ok(mut g) = TX.lock() {
        *g = Some(tx);
    }
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(|| run(rx));
    });
}

fn surface_size() -> (i32, i32) {
    let w = crate::WINDOW.load(Ordering::SeqCst);
    if w.is_null() {
        return (0, 0);
    }
    unsafe {
        (
            crate::sys::ANativeWindow_getWidth(w),
            crate::ANativeWindow_getHeight(w),
        )
    }
}

fn run(rx: Receiver<Vec<u8>>) {
    let Some(files) = crate::ahb::files_dir_pub() else {
        crate::log("input|SKIP|no files dir");
        return;
    };
    let marker = files.join(INPUT_MARKER);
    let guest_marker = files.join(INPUT_GUEST_MARKER);
    let enabled = || marker.exists() || guest_marker.exists();
    let (w0, h0) = surface_size();
    crate::log(&format!("input|surface|{w0}x{h0}"));
    loop {
        if !enabled() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            continue;
        }
        let path = files.join(INPUT_SOCKET);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(&path);
        let listener = match std::os::unix::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                crate::log(&format!("input|FAIL|bind {e}"));
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };
        crate::log(&format!("input|LISTEN|{}", path.display()));
        // An event already taken off the channel when the old client turned out
        // to be dead must go to the next client, not into the void.
        let mut carry: Option<Vec<u8>> = None;
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let (w, h) = surface_size();
            let header = format!("INPUT w={w} h={h} max={ABS_MAX}\n");
            if std::io::Write::write_all(&mut s, header.as_bytes()).is_err() {
                continue;
            }
            let sync = state_sync();
            if std::io::Write::write_all(&mut s, &sync).is_err() {
                continue;
            }
            if let Some(buf) = carry.take() {
                if std::io::Write::write_all(&mut s, &buf).is_err() {
                    continue;
                }
            }
            crate::log(&format!("input|CLIENT|{w}x{h}"));
            loop {
                match rx.recv() {
                    Ok(buf) => {
                        if peer_gone(&s) {
                            crate::log("input|CLIENT|gone|poll");
                            carry = Some(buf);
                            break;
                        }
                        if std::io::Write::write_all(&mut s, &buf).is_err() {
                            crate::log("input|CLIENT|gone");
                            break;
                        }
                    }
                    Err(_) => return,
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// A client that has gone away must be noticed *before* the next write, or the
/// event goes into a dead socket and is silently lost (observed: a reconnecting
/// guest tool missed the BTN_LEFT down of the first drag). poll() with a zero
/// timeout is non-blocking and also catches the half-closed case.
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
}

fn peer_gone(s: &std::os::unix::net::UnixStream) -> bool {
    use std::os::unix::io::AsRawFd;
    const POLLIN: i16 = 0x001;
    const POLLERR: i16 = 0x008;
    const POLLHUP: i16 = 0x010;
    let mut p = PollFd {
        fd: s.as_raw_fd(),
        events: POLLIN | POLLERR | POLLHUP,
        revents: 0,
    };
    let r = unsafe { poll(&mut p as *mut PollFd, 1, 0) };
    r > 0 && (p.revents & (POLLIN | POLLERR | POLLHUP)) != 0
}

fn push(buf: Vec<u8>) {
    if DIAG.fetch_add(1, Ordering::Relaxed) < 200 {
        crate::log(&format!("input|push|{}", buf.len()));
    }
    if let Ok(g) = TX.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(buf);
        }
    }
}

/// One 24-byte `struct input_event` (aarch64: timeval 8+8, type, code, value).
fn ev(t: u16, code: u16, value: i32) -> [u8; 24] {
    let mut b = [0u8; 24];
    // time is left zero: libinput/evdev consumers treat it as "now"
    b[16..18].copy_from_slice(&t.to_ne_bytes());
    b[18..20].copy_from_slice(&code.to_ne_bytes());
    b[20..24].copy_from_slice(&value.to_ne_bytes());
    b
}

fn synced(records: &[[u8; 24]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(24 * (records.len() + 1));
    for r in records {
        v.extend_from_slice(r);
    }
    v.extend_from_slice(&ev(EV_SYN, SYN_REPORT, 0));
    v
}

/// Device state as an absolute sync. A socket is not an evdev device: records
/// pushed while no client is attached are gone, so every new client starts with
/// the current pointer position and button state instead of guessing.
pub fn state_sync() -> Vec<u8> {
    let (sw, sh) = surface_size();
    let Ok(g) = STATE.lock() else {
        return Vec::new();
    };
    let w = if g.width > 1 { g.width } else { sw };
    let h = if g.height > 1 { g.height } else { sh };
    let mut v = Vec::with_capacity(96);
    v.extend_from_slice(&ev(EV_ABS, ABS_X, abs_scale(g.last_x, w.max(1))));
    v.extend_from_slice(&ev(EV_ABS, ABS_Y, abs_scale(g.last_y, h.max(1))));
    if g.button_down || g.down {
        v.extend_from_slice(&ev(EV_KEY, BTN_LEFT, 1));
    }
    v.extend_from_slice(&ev(EV_SYN, SYN_REPORT, 0));
    v
}

fn abs_scale(v: i32, span: i32) -> i32 {
    if span <= 1 {
        return 0;
    }
    (((v.clamp(0, span - 1) as i64) * ABS_MAX as i64) / (span - 1) as i64) as i32
}

/// Touch/mouse input from `drain_input`. `pointers` is the pointer count.
pub fn motion(action: i32, x: i32, y: i32, pointers: i32) {
    // android.view.MotionEvent action masks
    const DOWN: i32 = 0;
    const UP: i32 = 1;
    const MOVE: i32 = 2;
    const POINTER_DOWN: i32 = 5;
    const POINTER_UP: i32 = 6;
    const LONG_PRESS_MS: u64 = 500;

    if DIAG.load(Ordering::Relaxed) < 200 {
        crate::log(&format!("input|motion|{action}|{x}|{y}|p={pointers}"));
    }
    let (w, h) = surface_size();
    let mut g = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    g.width = w;
    g.height = h;
    let ax = abs_scale(x, w.max(1));
    let ay = abs_scale(y, h.max(1));

    match action {
        DOWN | POINTER_DOWN => {
            if action == DOWN && pointers >= 2 {
                g.two_finger_y = y;
                return;
            }
            g.down = true;
            g.down_x = x;
            g.down_y = y;
            g.down_at = Some(std::time::Instant::now());
            g.moved = false;
            g.last_x = x;
            g.last_y = y;
            // Pointer appears where the finger landed, button held from the start:
            // this is the MOUSE mode of both prior implementations.
            push(synced(&[
                ev(EV_ABS, ABS_X, ax),
                ev(EV_ABS, ABS_Y, ay),
                ev(EV_KEY, BTN_LEFT, 1),
            ]));
            g.button_down = true;
        }
        MOVE => {
            if pointers >= 2 && g.down {
                // Two fingers: wheel, content follows the fingers.
                let dy = g.two_finger_y - y;
                if dy.abs() >= 40 {
                    let notches = dy / 40;
                    if notches != 0 {
                        push(synced(&[ev(EV_REL, REL_WHEEL, notches)]));
                    }
                    g.two_finger_y = y;
                }
                return;
            }
            if !g.down {
                return;
            }
            if (x - g.down_x).abs() > 8 || (y - g.down_y).abs() > 8 {
                g.moved = true;
            }
            if g.moved && (x != g.last_x || y != g.last_y) {
                push(synced(&[
                    ev(EV_ABS, ABS_X, ax),
                    ev(EV_ABS, ABS_Y, ay),
                ]));
                g.last_x = x;
                g.last_y = y;
            }
        }
        UP | POINTER_UP => {
            if !g.down {
                return;
            }
            let held = g.down_at.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
            let moved = g.moved;
            g.down = false;
            g.moved = false;
            if g.button_down {
                push(synced(&[ev(EV_KEY, BTN_LEFT, 0)]));
                g.button_down = false;
            }
            if !moved && held >= LONG_PRESS_MS {
                // Long press = right click (both prior implementations).
                push(synced(&[
                    ev(EV_ABS, ABS_X, ax),
                    ev(EV_ABS, ABS_Y, ay),
                    ev(EV_KEY, BTN_RIGHT, 1),
                    ev(EV_KEY, BTN_RIGHT, 0),
                ]));
            }
        }
        _ => {}
    }
}

/// Keyboard from `drain_input`: Android keycode -> Linux keycode.
pub fn key(android_code: i32, down: bool) {
    let Some(code) = linux_keycode(android_code) else {
        return;
    };
    push(synced(&[ev(EV_KEY, code, if down { 1 } else { 0 })]));
}

/// Android `AKEYCODE_*` -> Linux `KEY_*` (`input-event-codes.h`).
fn linux_keycode(a: i32) -> Option<u16> {
    // Letters and digits are contiguous in both worlds, at different bases.
    let c = match a {
        29..=54 => 30 + (a - 29) as u16,      // A..Z -> KEY_A(30)..KEY_Z(55)
        7..=16 => 2 + (a - 7) as u16,         // 0..9 -> KEY_1(2)..KEY_0(11)
        66 => 28,                             // ENTER
        67 => 14,                             // DEL -> KEY_BACKSPACE
        61 => 15,                             // TAB
        62 => 57,                             // SPACE
        111 => 1,                             // ESCAPE
        112 => 111,                           // FORWARD_DEL -> KEY_DELETE
        19 => 103,                            // DPAD_UP
        20 => 108,                            // DPAD_DOWN
        21 => 105,                            // DPAD_LEFT
        22 => 106,                            // DPAD_RIGHT
        59 => 42,                             // SHIFT_LEFT
        60 => 54,                             // SHIFT_RIGHT
        113 => 29,                            // CTRL_LEFT
        114 => 97,                            // CTRL_RIGHT
        57 => 56,                             // ALT_LEFT
        58 => 100,                            // ALT_RIGHT
        115 => 125,                           // CAPS_LOCK
        122 => 102,                           // MOVE_HOME
        123 => 107,                           // MOVE_END
        92 => 104,                            // PAGE_UP
        93 => 109,                            // PAGE_DOWN
        68 => 41,                             // GRAVE -> KEY_GRAVE
        69 => 12,                             // MINUS
        70 => 13,                             // EQUALS
        71 => 26,                             // LEFT_BRACKET
        72 => 27,                             // RIGHT_BRACKET
        73 => 43,                             // BACKSLASH
        74 => 39,                             // SEMICOLON
        75 => 40,                             // APOSTROPHE
        55 => 51,                             // COMMA
        56 => 52,                             // PERIOD
        76 => 53,                             // SLASH
        _ => return None,
    };
    Some(c)
}
