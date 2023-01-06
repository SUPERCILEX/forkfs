use std::{
    env,
    env::{current_dir, set_current_dir},
    ffi::OsStr,
    os::unix::{fs::chroot, process::CommandExt},
    path::Path,
    process::Command,
};

use error_stack::Result;

use crate::{get_sessions_dir, sessions::maybe_create_session, Error, IoErr};

pub fn run<T: AsRef<OsStr>>(session: &str, command: &[T], stay_root: bool) -> Result<(), Error> {
    let mut session_dir = get_sessions_dir()?;
    session_dir.push(session);

    maybe_create_session(&mut session_dir)?;

    session_dir.push("merged");
    enter_session(&session_dir)?;

    run_command(command, stay_root)
}

fn enter_session(target: &Path) -> Result<(), Error> {
    // Must be retrieved before chroot-ing
    let current_dir = current_dir().map_io_err("Failed to get current directory")?;

    chroot(target).map_io_err_lazy(|| format!("Failed to change root {target:?}"))?;
    set_current_dir(current_dir)
        .map_io_err_lazy(|| format!("Failed to change current directory {target:?}"))
}

fn run_command(args: &[impl AsRef<OsStr>], stay_root: bool) -> Result<(), Error> {
    let mut command = Command::new(args[0].as_ref());

    // Downgrade privilege level to pre-sudo if possible
    if !stay_root && let Some(uid) = env::var_os("SUDO_UID").as_ref().and_then(|s| s.to_str())
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
