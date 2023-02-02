use std::{
    env,
    env::{current_dir, set_current_dir},
    ffi::OsStr,
    fs,
    os::unix::{fs::chroot, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};

use error_stack::{IntoReport, Result, ResultExt};
use rustix::{
    io::Errno,
    process::{getuid, Uid},
    thread::{capabilities, set_thread_uid, CapabilityFlags},
};

use crate::{get_sessions_dir, sessions::maybe_create_session, Error, IoErr};

pub fn run<T: AsRef<OsStr>>(session: &str, command: &[T], stay_root: bool) -> Result<(), Error> {
    let uid = getuid();
    validate_permissions(uid)?;

    let mut session_dir = get_sessions_dir();
    session_dir.push(session);

    maybe_create_session(&mut session_dir)?;

    session_dir.push("merged");
    enter_session(&session_dir)?;

    run_command(command, uid, stay_root)
}

fn enter_session(target: &Path) -> Result<(), Error> {
    // Must be retrieved before chroot-ing
    let current_dir = current_dir().map_io_err("Failed to get current directory")?;

    chroot(target).map_io_err_lazy(|| format!("Failed to change root {target:?}"))?;
    set_current_dir(current_dir)
        .map_io_err_lazy(|| format!("Failed to change current directory {target:?}"))
}

fn run_command(args: &[impl AsRef<OsStr>], prev_uid: Uid, stay_root: bool) -> Result<(), Error> {
    let mut command = Command::new(args[0].as_ref());

    // Downgrade privilege level to pre-sudo if possible
    if !stay_root {
        if !prev_uid.is_root() {
            command.uid(prev_uid.as_raw());
        } else if let Some(uid) = env::var_os("SUDO_UID").as_ref().and_then(|s| s.to_str())
            && let Ok(uid) = uid.parse() {
            command.uid(uid);
        }
    }

    Err(command.args(&args[1..]).exec()).map_io_err_lazy(|| {
        format!(
            "Failed to exec {:?}",
            args.iter().map(AsRef::as_ref).collect::<Vec<_>>()
        )
    })
}

fn validate_permissions(uid: Uid) -> Result<(), Error> {
    if uid.is_root() {
        return Ok(());
    }

    match set_thread_uid(Uid::ROOT) {
        Err(Errno::PERM) => {
            // Continue to capability check
        }
        r => {
            return r.map_io_err("Failed to become root");
        }
    }

    {
        let effective_capabilities = capabilities(None)
            .map_io_err("Failed to retrieve capabilities")?
            .effective;
        if effective_capabilities.contains(
            CapabilityFlags::CHOWN
                | CapabilityFlags::DAC_OVERRIDE
                | CapabilityFlags::SYS_CHROOT
                | CapabilityFlags::SYS_ADMIN,
        ) {
            return Ok(());
        }
    }

    let path = env::args_os().next().map(PathBuf::from);
    let path = fs::canonicalize(path.as_deref().unwrap_or_else(|| Path::new("forkfs")));
    let path = path
        .as_deref()
        .ok()
        .unwrap_or_else(|| Path::new("<path-to-forkfs>"));

    Err(Error::SetupRequired)
        .into_report()
        .attach_printable(format!(
            "Welcome to ForkFS!

Under the hood, ForkFS is implemented as a wrapper around OverlayFS. As a
consequence, elevated privileges are required and can be granted in one of
three ways (ordered by recommendation):

- $ sudo chown root {0} && sudo chmod u+s {0}

  This transfers ownership of the `forkfs` binary to root and specifies that
  the binary should be executed as its owner (i.e. root). This is preferable
  because it allows you to pass along root privileges to the sandboxed
  program when necessary.

- $ sudo setcap cap_chown,cap_dac_override,cap_sys_chroot,cap_sys_admin+ep {0}

  This grants `forkfs` precisely the capabilities it needs. Note that the
  `stay-root` flag will not work.

- $ sudo -E forkfs ...

  This simply invokes `forkfs` as root. This option is problematic because
  sudo alters the environment, causing PATH lookups to fail and changing
  your home directory.

  If you do go down this route, be consistent with your usage of `-E`. Bare
  `sudo` vs `sudo -E` will change the forkfs environment, meaning sessions
  that appear in `sudo` will not appear in `sudo -E` and vice versa.

PS: if you've already seen this message, then you probably upgraded to a new
version of ForkFS and will therefore need to rerun this setup.",
            path.to_string_lossy()
        ))
}
