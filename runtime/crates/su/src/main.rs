//! husk-su — the guest's own `su` / `runuser`.
//!
//! A container needs `su`: it is how a root-owned script becomes the container
//! user, and it is what LinuxHub 2.0 calls (`su - <user> -c …`) in three
//! places while initialising. util-linux' su is dynamically linked, so it does
//! pass through the shim — but it cannot switch identity here. Measured on the
//! 2.0 tree (2026-11-24):
//!
//!     runuser: cannot set groups: Operation not permitted
//!     su: System error                          (PAM_SYSTEM_ERR)
//!
//! The first is `initgroups()` inside libc: it calls setgroups() without going
//! through the GOT, so the interposer never sees it and the real syscall is
//! refused by the kernel (the process is uid 10512 and has no CAP_SETGID). The
//! second is PAM; the failing module is not isolated yet.
//!
//! Neither matters for this runtime, because identity here is a *fiction* the
//! shim maintains: NR_FAKE_UID / NR_FAKE_GID seed it, and every observer
//! (getuid, getgid, getgroups, stat, /proc) answers in terms of it. So `su`
//! needs no privilege at all — it only has to pick the target account, set the
//! contract variables and exec the shell. That is this program.
//!
//! What it deliberately does not do:
//!
//!   * password authentication. Switching from a non-root identity is refused:
//!     inside this runtime the caller either is root (uid 0 as the shim reports
//!     it) or is the target user already.
//!   * PAM. There is no session manager to configure, and the container user is
//!     not a system account.
//!   * `-w/--whitelist-environment`, `--pty`, `-P/--pty` and `--session-command`
//!     (unknown options are an error, never silently ignored).

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint};
use std::path::Path;

extern "C" {
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char)
        -> c_int;
    fn getuid() -> c_uint;
    fn chdir(path: *const c_char) -> c_int;
}

const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const EXIT_USAGE: c_int = 2;

struct PasswdEntry {
    name: String,
    uid: u32,
    gid: u32,
    home: String,
    shell: String,
}

struct Opts {
    login: bool,
    preserve: bool,
    user: Option<String>,
    command: Option<String>,
    shell: Option<String>,
    group: Option<String>,
    argv: Vec<String>,
    prog: String,
}

fn usage(prog: &str) -> ! {
    eprintln!(
        "usage: {prog} [ -l | - ] [ -c COMMAND ] [ -s SHELL ] [ -p ] [ -g GROUP ] [ user [argument...] ]\n\
         \n\
         {prog} [ -l ] [ -c COMMAND ] [ -s SHELL ] [-u] USER ...\n\
         \n\
         Switch the container identity and run a shell, without PAM or setuid:\n\
           -l, --login                 start a login shell in the target's home\n\
           -c, --command COMMAND       pass COMMAND to the shell with -c\n\
           -s, --shell SHELL           run SHELL instead of the account's\n\
           -p, -m, --preserve-environment\n\
                                       keep the caller's environment\n\
           -g, --group GROUP           use GROUP as the primary group\n\
           -u USER                     runuser spelling of the user argument\n\
           -, --                       end of options / login, as in util-linux\n\
           -h, --help                  this text\n\
           -V, --version               print the version"
    );
    std::process::exit(EXIT_USAGE);
}

fn parse(prog: String) -> Opts {
    let mut o = Opts {
        login: false,
        preserve: false,
        user: None,
        command: None,
        shell: None,
        group: None,
        argv: Vec::new(),
        prog,
    };
    let mut it = std::env::args().skip(1).peekable();
    let mut end_of_opts = false;
    while let Some(a) = it.next() {
        if !end_of_opts && !a.starts_with('-') {
            // positional: first is the user, the rest is the command
            if o.user.is_none() {
                o.user = Some(a);
            } else {
                o.argv.push(a);
            }
            continue;
        }
        match a.as_str() {
            "--" => end_of_opts = true,
            // util-linux spells "login shell" as a bare "-"
            "-" if !end_of_opts && o.user.is_none() && o.command.is_none() => o.login = true,
            "-l" | "--login" => o.login = true,
            "-p" | "-m" | "--preserve-environment" => o.preserve = true,
            "-c" | "--command" | "--session-command" => match it.next() {
                Some(v) => o.command = Some(v),
                None => die(&o.prog, "option requires an argument: --command"),
            },
            "-s" | "--shell" => match it.next() {
                Some(v) => o.shell = Some(v),
                None => die(&o.prog, "option requires an argument: --shell"),
            },
            "-g" | "--group" => match it.next() {
                Some(v) => o.group = Some(v),
                None => die(&o.prog, "option requires an argument: --group"),
            },
            "-u" | "--user" => match it.next() {
                Some(v) => o.user = Some(v),
                None => die(&o.prog, "option requires an argument: --user"),
            },
            "-h" | "--help" => usage(&o.prog),
            "-V" | "--version" => {
                println!("husk-su {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if a.starts_with("--command=") => {
                o.command = Some(a["--command=".len()..].to_string())
            }
            _ if a.starts_with("--shell=") => o.shell = Some(a["--shell=".len()..].to_string()),
            _ if a.starts_with("--group=") => o.group = Some(a["--group=".len()..].to_string()),
            "-" => {}
            other => die(&o.prog, &format!("unrecognized option: {other}")),
        }
    }
    o
}

fn die(prog: &str, msg: &str) -> ! {
    eprintln!("{prog}: {msg}");
    std::process::exit(EXIT_USAGE);
}

fn fail(prog: &str, msg: &str, code: c_int) -> ! {
    eprintln!("{prog}: {msg}");
    std::process::exit(code);
}

fn read_lines(path: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

fn accounts() -> Vec<PasswdEntry> {
    let mut out = Vec::new();
    for line in read_lines("/etc/passwd") {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 7 || f[0].starts_with('#') {
            continue;
        }
        out.push(PasswdEntry {
            name: f[0].to_string(),
            uid: f[2].parse().unwrap_or(u32::MAX),
            gid: f[3].parse().unwrap_or(u32::MAX),
            home: f[5].to_string(),
            shell: f[6].to_string(),
        });
    }
    out
}

/// Primary group: name, or a bare number (util-linux accepts both).
fn gid_of(group: &str) -> Option<u32> {
    if let Ok(n) = group.parse::<u32>() {
        return Some(n);
    }
    for line in read_lines("/etc/group") {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 3 && f[0] == group {
            return f[2].parse().ok();
        }
    }
    None
}

/// Names a login environment may keep. The session needs the display and bus
/// variables to reach the running compositor (a real `su -` would not hand them
/// over and the caller would have to export them again); everything else is
/// dropped, which is what `su -` means.
fn keeps(name: &str) -> bool {
    name.starts_with("NR_")
        || name == "HUSK_DRM_API"
        || name == "GLIBC_TUNABLES"
        || name == "TERM"
        || name == "COLORTERM"
        || name == "TZ"
        || name.starts_with("LC_")
        || name == "LANG"
        || name == "DISPLAY"
        || name == "WAYLAND_DISPLAY"
        || name == "XDG_RUNTIME_DIR"
        || name == "XDG_SESSION_TYPE"
        || name == "DBUS_SESSION_BUS_ADDRESS"
        || name == "DBUS_SYSTEM_BUS_ADDRESS"
}

fn main() {
    let prog = std::env::args()
        .next()
        .and_then(|p| {
            Path::new(&p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "husk-su".to_string());
    let o = parse(prog.clone());
    let want = o.user.clone().unwrap_or_else(|| "root".to_string());
    let all = accounts();
    let entry = match all.iter().find(|e| e.name == want) {
        Some(e) => e,
        None => fail(&prog, &format!("user {want} does not exist"), 1),
    };

    // Who is asking? The shim answers with the *fake* identity, so a session
    // started as a user sees its own uid and a root script sees 0.
    let me = unsafe { getuid() };
    if me != 0 && me != entry.uid {
        fail(
            &prog,
            "password authentication is not available inside this container \
             (only root may switch to another account)",
            1,
        );
    }

    let gid = match &o.group {
        Some(g) => match gid_of(g) {
            Some(v) => v,
            None => fail(&prog, &format!("group {g} does not exist"), 1),
        },
        None => entry.gid,
    };
    let shell = match &o.shell {
        Some(s) => s.clone(),
        None if entry.shell.is_empty() => "/bin/sh".to_string(),
        None => entry.shell.clone(),
    };
    if !Path::new(&shell).is_absolute() {
        fail(&prog, &format!("shell {shell} is not a path"), 1);
    }
    let base = |p: &str| {
        Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string())
    };

    // ---- environment -------------------------------------------------------
    let clean = o.login && !o.preserve;
    let mut env: Vec<(String, String)> = std::env::vars_os()
        .filter(|(k, _)| !clean || keeps(&k.to_string_lossy()))
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let set = |env: &mut Vec<(String, String)>, k: &str, v: &str| {
        if let Some(slot) = env.iter_mut().find(|(n, _)| n == k) {
            slot.1 = v.to_string();
        } else {
            env.push((k.to_string(), v.to_string()));
        }
    };
    let get = |env: &Vec<(String, String)>, k: &str| {
        env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    };
    set(&mut env, "NR_FAKE_UID", &entry.uid.to_string());
    set(&mut env, "NR_FAKE_GID", &gid.to_string());
    if get(&env, "NR_FAKE_ROOT").is_none() {
        set(&mut env, "NR_FAKE_ROOT", "1");
    }
    set(&mut env, "USER", &entry.name);
    set(&mut env, "LOGNAME", &entry.name);
    set(&mut env, "HOME", &entry.home);
    set(&mut env, "SHELL", &shell);
    if clean || get(&env, "PATH").is_none() {
        set(&mut env, "PATH", DEFAULT_PATH);
    }

    // ---- cwd ---------------------------------------------------------------
    if o.login {
        let home = CString::new(entry.home.clone()).unwrap_or_default();
        if unsafe { chdir(home.as_ptr()) } != 0 {
            eprintln!("{prog}: warning: cannot change directory to {}", entry.home);
        }
    }

    // ---- argv --------------------------------------------------------------
    let argv0 = if o.login {
        format!("-{}", base(&shell))
    } else {
        base(&shell)
    };
    let mut argv: Vec<String> = vec![argv0];
    match (&o.command, o.argv.is_empty()) {
        (Some(cmd), _) => {
            argv.push("-c".to_string());
            argv.push(cmd.clone());
        }
        (None, true) => {}
        // `su user -- prog args`: no shell, straight to exec (as util-linux does).
        (None, false) => {
            argv = o.argv.clone();
        }
    }

    // ---- exec --------------------------------------------------------------
    // execve() is the *interposed* symbol: the call goes through the GOT, so the
    // shim resolves a guest path and re-enters the loader with the right
    // library path and --argv0. Calling execvp or spawning from here would go
    // through libc's private path and land on the host root.
    let prog_path = if o.argv.is_empty() || o.command.is_some() {
        shell.clone()
    } else {
        // command words: resolve through the guest PATH so `su user -- ls` works
        let name = o.argv[0].clone();
        if name.contains('/') {
            name
        } else {
            match get(&env, "PATH").and_then(|p| {
                p.split(':')
                    .map(|d| format!("{}/{}", d.trim_end_matches('/'), name))
                    .find(|c| Path::new(c).is_file())
            }) {
                Some(p) => p,
                None => fail(&prog, &format!("{name}: command not found"), 127),
            }
        }
    };
    let cpath = CString::new(prog_path.clone()).unwrap_or_default();
    let cargv: Vec<CString> = argv
        .iter()
        .map(|a| CString::new(a.as_str()).unwrap_or_default())
        .collect();
    let mut pargv: Vec<*const c_char> = cargv.iter().map(|c| c.as_ptr()).collect();
    pargv.push(std::ptr::null());
    let cenv: Vec<CString> = env
        .iter()
        .map(|(k, v)| CString::new(format!("{k}={v}")).unwrap_or_default())
        .collect();
    let mut penv: Vec<*const c_char> = cenv.iter().map(|c| c.as_ptr()).collect();
    penv.push(std::ptr::null());

    unsafe { execve(cpath.as_ptr(), pargv.as_ptr(), penv.as_ptr()) };

    let err = std::io::Error::last_os_error();
    eprintln!("{prog}: {}: {}", prog_path, err);
    std::process::exit(if err.raw_os_error() == Some(2) {
        127
    } else {
        1
    });
}
