//! Verify the guest identity layer's supplementary-group semantics through
//! the same libc entry points used by ordinary Linux programs.

use std::ffi::CString;
use std::io;
use std::process::Command;

fn get_groups() -> io::Result<Vec<u32>> {
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut groups = vec![0 as libc::gid_t; count as usize];
    if count > 0 {
        let actual = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
        if actual < 0 {
            return Err(io::Error::last_os_error());
        }
        groups.truncate(actual as usize);
    }
    Ok(groups.into_iter().map(|group| group as u32).collect())
}

fn set_groups(groups: &[u32]) -> io::Result<()> {
    let raw = groups
        .iter()
        .copied()
        .map(|group| group as libc::gid_t)
        .collect::<Vec<_>>();
    let rc = unsafe { libc::setgroups(raw.len(), raw.as_ptr()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn init_groups(user: &str, gid: u32) -> io::Result<()> {
    let user = CString::new(user).expect("group user has no NUL");
    let rc = unsafe { libc::initgroups(user.as_ptr(), gid as libc::gid_t) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn access(path: &str, mode: libc::c_int) -> bool {
    let path = CString::new(path).expect("access path has no NUL");
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}

fn print_groups(prefix: &str, groups: &[u32]) {
    println!("{prefix} {:?}", groups);
}

pub fn run() -> i32 {
    let initial = match get_groups() {
        Ok(groups) => groups,
        Err(err) => {
            eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=getgroups-initial error={err}");
            return 1;
        }
    };
    print_groups("AFTER_START", &initial);
    println!("UID_GID {} {}", unsafe { libc::getuid() }, unsafe {
        libc::getgid()
    });

    match Command::new("id").arg("-G").output() {
        Ok(output) if output.status.success() => {
            println!(
                "ID_GROUPS {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
        Ok(output) => {
            eprintln!(
                "SUPPLEMENTARY_GROUPS_FAIL step=id-groups status={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return 1;
        }
        Err(err) => {
            eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=id-groups error={err}");
            return 1;
        }
    }
    print_groups("GETGROUPS", &initial);

    if let Err(err) = set_groups(&[4242]) {
        eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=setgroups error={err}");
        return 1;
    }
    let after_setgroups = match get_groups() {
        Ok(groups) => groups,
        Err(err) => {
            eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=getgroups-after-setgroups error={err}");
            return 1;
        }
    };
    print_groups("AFTER_SETGROUPS", &after_setgroups);
    if after_setgroups != [4242] {
        eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=setgroups-replace expected=[4242]");
        return 1;
    }

    let readable = access("/tmp/husk-group-audit/target", libc::R_OK);
    let writable = access("/tmp/husk-group-audit/target", libc::W_OK);
    println!("ACCESS_GROUP_R {readable}");
    println!("ACCESS_GROUP_W {writable}");
    if !readable || !writable {
        eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=access expected read/write");
        return 1;
    }

    if let Err(err) = init_groups("huskuser", 1500) {
        eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=initgroups error={err}");
        return 1;
    }
    let after_initgroups = match get_groups() {
        Ok(groups) => groups,
        Err(err) => {
            eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=getgroups-after-initgroups error={err}");
            return 1;
        }
    };
    print_groups("AFTER_INITGROUPS", &after_initgroups);
    if !after_initgroups.contains(&4242) {
        eprintln!("SUPPLEMENTARY_GROUPS_FAIL step=initgroups membership=4242");
        return 1;
    }

    println!("PASS supplementary-groups setgroups/initgroups/getgroups/access");
    0
}
