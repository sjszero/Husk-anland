//! Idempotent guest-side provisioning for the Husk PulseAudio audio path.
//!
//! PulseAudio 16.1 in Ubuntu 24.04 looks for its helper libraries in the main
//! aarch64-linux-gnu directory. Copy those files and the pipe modules without a
//! Python/shell provisioning step, create the private runtime directory, and
//! write the minimal `module-pipe-sink` configuration used by `audio-relay`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn usage() -> ! {
    eprintln!("usage: audio-setup <rootfs> [--state DIR] [--enable] [--rate HZ] [--channels N]");
    std::process::exit(2)
}

fn copy_if_needed(src: &Path, dst: &Path) -> io::Result<bool> {
    if dst.exists() {
        let old = fs::read(dst)?;
        let new = fs::read(src)?;
        if old == new {
            return Ok(false);
        }
    }
    fs::copy(src, dst)?;
    Ok(true)
}

fn copy_named(dir: &Path, dst_dir: &Path, prefix: &str, suffix: &str) -> io::Result<usize> {
    let mut count = 0;
    if !dir.is_dir() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_file() && name.starts_with(prefix) && name.ends_with(suffix) {
            if copy_if_needed(&path, &dst_dir.join(&name))? {
                println!(
                    "audio-setup|COPY|{} -> {}",
                    path.display(),
                    dst_dir.join(&name).display()
                );
            }
            count += 1;
        }
    }
    Ok(count)
}

/// Return the physical rootfs path when the guest was started with `/` as its
/// logical root. `/dev` is intentionally a passthrough tree, so creating
/// `/dev/shm` through guest path resolution would target the Android host
/// namespace instead of the private rootfs.
fn physical_root(root: &Path) -> PathBuf {
    if root == Path::new("/") {
        if let Some(path) = std::env::var_os("NR_ROOTFS") {
            let path = PathBuf::from(path);
            if !path.as_os_str().is_empty() {
                return path;
            }
        }
    }
    root.to_path_buf()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root_arg) = args.first() else {
        usage();
    };
    let root = PathBuf::from(root_arg);
    let lib = root.join("usr/lib/aarch64-linux-gnu");
    let pulse_private = lib.join("pulseaudio");
    let physical = physical_root(&root);
    let mut state = physical.join("tmp/husk-pulse");
    let mut enable = false;
    let mut enable_capture = false;
    let mut rate = 48_000u32;
    let mut channels = 2u32;

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--state" => {
                i += 1;
                state = PathBuf::from(args.get(i).unwrap_or_else(|| usage()));
            }
            "--enable" => enable = true,
            "--enable-capture" => {
                enable = true;
                enable_capture = true;
            }
            "--rate" => {
                i += 1;
                rate = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--channels" => {
                i += 1;
                channels = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            other => {
                eprintln!("audio-setup|FAIL|unknown argument {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    if !root.join("usr/bin/pulseaudio").is_file() {
        eprintln!(
            "audio-setup|FAIL|{} has no usr/bin/pulseaudio",
            root.display()
        );
        std::process::exit(1);
    }
    if fs::create_dir_all(&lib).is_err() {
        eprintln!("audio-setup|FAIL|cannot create {}", lib.display());
        std::process::exit(1);
    }

    let result = (|| -> io::Result<()> {
        let mut files_seen = 0;
        files_seen += copy_named(&pulse_private, &lib, "libpulsecore-", ".so")?;
        files_seen += copy_named(&pulse_private, &lib, "libpulsecommon-", ".so")?;

        let mut module_dirs = 0;
        for entry in fs::read_dir(root.join("usr/lib"))? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && entry.file_name().to_string_lossy().starts_with("pulse-") {
                module_dirs += 1;
                files_seen += copy_named(&path.join("modules"), &lib, "lib", ".so")?;
            }
        }

        fs::create_dir_all(&state)?;
        fs::create_dir_all(state.join("runtime"))?;
        // PulseAudio probes /dev/shm; keep the guest's private directory inside the rootfs.
        fs::create_dir_all(physical.join("dev/shm"))?;

        // Keep the caller's absolute host/run-as path. Do not use canonicalize here:
        // inside the path-shim this can return the guest view (`/tmp/...`), which
        // PulseAudio's mkfifo path bypasses and therefore cannot open.
        let state_abs = state.clone();
        let capture_lines = if enable_capture {
            // The guest side of the microphone: PulseAudio reads our FIFO, the
            // Husk capture client writes it. `module-pipe-source` needs the
            // FIFO to exist, so create it here instead of at record time.
            // `module-pipe-source` calls mkfifo() and treats EEXIST as fatal
            // (unlike `module-pipe-sink`, which tolerates it), so the module
            // creates the FIFO itself. All we do is clear a stale one from a
            // previous run; a leftover FIFO would make `load-module` fail.
            let mic = state_abs.join("mic.pcm");
            match fs::remove_file(&mic) {
                Ok(()) => println!("audio-setup|UNCLEAN|{}", mic.display()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => println!("audio-setup|WARN|{}: {e}", mic.display()),
            }
            format!(
                "load-module module-pipe-source file={} source_name=husk_mic format=s16le rate={} channels={}\n\
                 set-default-source husk_mic\n",
                mic.display(),
                rate,
                channels
            )
        } else {
            String::new()
        };
        let config = format!(
            "load-module module-native-protocol-unix socket={}/native auth-anonymous=1\n\
             load-module module-pipe-sink file={}/audio.pcm sink_name=husk_pipe format=s16le rate={} channels={}\n\
             set-default-sink husk_pipe\n{}",
            state_abs.display(),
            state_abs.display(),
            rate,
            channels,
            capture_lines
        );
        let config_path = state.join("minimal.pa");
        fs::write(&config_path, config)?;

        if enable {
            let marker = state.join("enabled");
            fs::write(&marker, b"audio-v1\n")?;
            println!("audio-setup|ENABLE|{}", marker.display());
        }
        if enable_capture {
            let marker = state.join("enabled-in");
            fs::write(&marker, b"audio-in-v1\n")?;
            println!("audio-setup|ENABLE_CAPTURE|{}", marker.display());
        }

        println!(
            "audio-setup|PASS|root={}|helper_modules={files_seen}|module_dirs={module_dirs}|state={}",
            root.display(),
            state_abs.display()
        );
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("audio-setup|FAIL|{e}");
        std::process::exit(1);
    }
}
