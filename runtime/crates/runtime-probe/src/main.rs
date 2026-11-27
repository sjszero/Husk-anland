//! Guest-side probes that exercise Path Shim through libc entry points.
//!
//! Each subcommand replaces a former in-tree C probe without changing its
//! observable PASS/FAIL protocol. The binaries are cross-built for glibc and
//! started through the Husk loader, so the calls below still pass through the
//! same GOT/PLT interposition as ordinary guest programs.

#[cfg(target_os = "linux")]
mod coverage;
#[cfg(any(target_os = "linux", test))]
mod egress;
#[cfg(target_os = "linux")]
mod groups_race;
#[cfg(target_os = "linux")]
mod identity_race;
#[cfg(target_os = "linux")]
mod mount_audit;
#[cfg(target_os = "linux")]
mod path_race;
#[cfg(target_os = "linux")]
mod rootfd_race;
#[cfg(target_os = "linux")]
mod supplementary_groups;
#[cfg(target_os = "linux")]
mod symbol_gap;

#[cfg(target_os = "linux")]
fn usage() {
    eprintln!(
        "usage: runtime-probe <coverage|egress|groups-race|identity-init-race|mount-audit CMD|path-race SENTINEL|rootfd-close-range|supplementary-groups|symbol-gap>"
    );
}

#[cfg(target_os = "linux")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("coverage") => coverage::run(),
        Some("egress") => egress::run(),
        Some("groups-race") => groups_race::run(),
        Some("identity-init-race") => identity_race::run(),
        Some("mount-audit") => mount_audit::run(&args),
        Some("path-race") => {
            if args.len() != 3 {
                eprintln!("usage: runtime-probe path-race <host-sentinel-absolute-path>");
                64
            } else {
                path_race::run(&args[2])
            }
        }
        Some("rootfd-close-range") => rootfd_race::run(),
        Some("supplementary-groups") => supplementary_groups::run(),
        Some("symbol-gap") => symbol_gap::run(),
        Some("-h" | "--help") => {
            usage();
            0
        }
        _ => {
            usage();
            64
        }
    };
    std::process::exit(code);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("runtime-probe only runs on Linux guests");
    std::process::exit(1);
}
