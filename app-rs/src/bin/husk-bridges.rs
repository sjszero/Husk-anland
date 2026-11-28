//! `husk-bridges` — one hardware process for the `.husk` host app.
//!
//! The Kotlin host provides the X server, Surface and desktop lifecycle. This
//! helper owns the endpoints a glibc guest opens under `/run/husk`:
//! camera, microphone, speaker, sensors, GNSS, decoder and encoder. It runs in
//! the host app UID, so Android permission checks are exactly the permissions
//! the user granted to the app; nothing is escalated.
//!
//!   HUSK_FILES=<app files dir>  HUSK_ROOTFS=<active guest rootfs> husk-bridges
use std::path::{Path, PathBuf};
use std::time::Duration;

fn socket_ready(rootfs: &Path, relative: &str) -> bool {
    rootfs.join(relative).exists()
}

fn view_ready(rootfs: &Path, relative: &str) -> bool {
    rootfs.join(relative).exists()
}

fn write_capabilities(files: &Path, rootfs: &Path) {
    let camera = socket_ready(rootfs, "run/husk/camera.sock");
    let audio_out = socket_ready(rootfs, "run/husk/audio-out.sock");
    let audio_in = socket_ready(rootfs, "run/husk/audio-in.sock");
    let location = socket_ready(rootfs, "run/husk/gps.sock");
    let sensors = view_ready(rootfs, "sys/class/iio/iio:device0");
    let video = socket_ready(rootfs, "run/husk/video.sock");
    let encoder = socket_ready(rootfs, "run/husk/encoder.sock");

    let caps = format!(
        "# husk-capabilities v1\n\
         caps|camera|present=yes|husk={}|backend=Camera2 -> V4L2|detail=Camera2 socket and guest facade\n\
         caps|audio-out|present=yes|husk={}|backend=AAudio -> PulseAudio|detail=speaker bridge\n\
         caps|audio-in|present=yes|husk={}|backend=AAudio capture|detail=microphone bridge\n\
         caps|sensors|present=yes|husk={}|backend=ASensorManager -> IIO|detail=guest IIO views\n\
         caps|location|present=yes|husk={}|backend=Android Location -> gpsd|detail=last-known fix\n\
         caps|video-decode|present=yes|husk={}|backend=MediaCodec decoder|detail=run/husk/video.sock\n\
         caps|video-encode|present=yes|husk={}|backend=MediaCodec encoder|detail=run/husk/encoder.sock\n",
        if camera { "ready" } else { "degraded" },
        if audio_out { "ready" } else { "degraded" },
        if audio_in { "ready" } else { "degraded" },
        if sensors { "ready" } else { "degraded" },
        if location { "ready" } else { "degraded" },
        if video { "ready" } else { "degraded" },
        if encoder { "ready" } else { "degraded" },
    );
    let _ = std::fs::write(files.join("husk-capabilities.txt"), &caps);
    let _ = std::fs::write(rootfs.join("run/husk/capabilities.txt"), caps);
}

fn main() {
    let files = PathBuf::from(std::env::var("HUSK_FILES").expect("HUSK_FILES"));
    let rootfs = PathBuf::from(std::env::var("HUSK_ROOTFS").expect("HUSK_ROOTFS"));
    println!(
        "husk-bridges|start|files={}|rootfs={}",
        files.display(),
        rootfs.display()
    );
    for dir in [
        rootfs.join("run/husk"),
        rootfs.join("tmp/husk-pulse"),
        rootfs.join("tmp/husk-encoder"),
    ] {
        let _ = std::fs::create_dir_all(dir);
    }

    // The product endpoints are always enabled in this dedicated helper. The
    // guest still does not get an implicit open: each bridge accepts one real
    // client and reports failures by name.
    let _ = std::fs::write(files.join("encoder"), "enabled\n");
    let _ = std::fs::write(rootfs.join("tmp/husk-encoder/enabled"), "enabled\n");
    let _ = std::fs::write(files.join("video-bridge"), "enabled\n");
    let _ = std::fs::write(rootfs.join("run/husk/enabled"), "enabled\n");

    write_capabilities(&files, &rootfs);
    huskapp::audio::start_once_at(files.clone(), rootfs.clone());
    huskapp::camera::start_once_at(files.clone(), rootfs.clone());
    huskapp::sensors::start_once_at(rootfs.clone());
    huskapp::location::start_once_at(files.clone(), rootfs.clone());
    huskapp::encoder::start_once_at(files.clone(), rootfs.clone());
    huskapp::video::start_once_at(files.clone(), rootfs.clone());

    let parent_pid = std::env::var("HUSK_PARENT_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    loop {
        write_capabilities(&files, &rootfs);
        if let Some(pid) = parent_pid {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_file(rootfs.join("run/husk/video.sock"));
                let _ = std::fs::remove_file(rootfs.join("run/husk/encoder.sock"));
                std::process::exit(0);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
