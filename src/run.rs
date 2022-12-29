use std::{
    env,
    env::{current_dir, set_current_dir},
    ffi::{CStr, CString, OsStr},
    fmt::Write,
    fs,
    os::unix::{fs::chroot, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};

use error_stack::{IntoReport, Result, ResultExt};
use nix::mount::{mount, MsFlags};

use crate::{get_sessions_dir, is_active_session, path_undo::TmpPath, Error, IoErr};

pub fn run<T: AsRef<OsStr>>(session: &str, command: &[T]) -> Result<(), Error> {
    let mut session_dir = get_sessions_dir()?;
    session_dir.push(session);

    if !maybe_create_session(&mut session_dir)? {
        mount_session(&mut session_dir)?;
    }

    session_dir.push("merged");
    enter_session(&session_dir)?;

    run_command(command)
}

fn maybe_create_session(dir: &mut PathBuf) -> Result<bool, Error> {
    let session_active = is_active_session(dir, false)?;
    if !session_active {
        for path in ["diff", "work", "merged"] {
            let dir = TmpPath::new(dir, path);
            fs::create_dir_all(&dir)
                .map_io_err_lazy(|| format!("Failed to create directory {dir:?}"))?;
        }
    }
    Ok(session_active)
}

fn mount_session(dir: &mut PathBuf) -> Result<(), Error> {
    const OVERLAY: &CStr = CStr::from_bytes_with_nul(b"overlay\0").ok().unwrap();
    const PROC: &CStr = CStr::from_bytes_with_nul(b"/proc\0").ok().unwrap();

    let command = {
        let mut command = String::from("lowerdir=/,");
        {
            let diff = TmpPath::new(dir, "diff");
            write!(command, "upperdir={},", diff.display()).unwrap();
        }
        {
            let work = TmpPath::new(dir, "work");
            write!(command, "workdir={}", work.display()).unwrap();
        }

        CString::new(command.into_bytes())
            .into_report()
            .attach_printable("Invalid path bytes")
            .change_context(Error::InvalidArgument)?
    };

    let mut merged = TmpPath::new(dir, "merged");
    mount(
        Some(OVERLAY),
        &*merged,
        Some(OVERLAY),
        MsFlags::empty(),
        Some(command.as_c_str()),
    )
    .map_io_err_lazy(|| format!("Failed to mount directory {merged:?}"))?;

    let proc = TmpPath::new(&mut merged, "proc");
    mount(
        Some(PROC),
        &*proc,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_io_err_lazy(|| format!("Failed to mount directory {proc:?}"))
}

fn enter_session(target: &Path) -> Result<(), Error> {
    // Must be retrieved before chroot-ing
    let current_dir = current_dir().map_io_err("Failed to get current directory")?;

    chroot(target).map_io_err_lazy(|| format!("Failed to change root {target:?}"))?;
    set_current_dir(current_dir)
        .map_io_err_lazy(|| format!("Failed to change current directory {target:?}"))
}

fn run_command(args: &[impl AsRef<OsStr>]) -> Result<(), Error> {
    let mut command = Command::new(args[0].as_ref());

    // Downgrade privilege level to pre-sudo if possible
    if let Some(uid) = env::var_os("SUDO_UID").as_ref().and_then(|s| s.to_str())
        && let Ok(uid) = uid.parse() {
        command.uid(uid);
    }

    Err(command.args(&args[1..]).exec()).map_io_err_lazy(|| {
        format!(
            "Failed to exec {:?}",
            args.iter().map(AsRef::as_ref).collect::<Vec<_>>()
        )
    })
}
