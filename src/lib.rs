#![feature(let_chains)]
#![feature(const_option)]
#![feature(const_result_drop)]
#![feature(const_cstr_methods)]

use std::{
    env,
    env::{current_dir, set_current_dir},
    ffi::{CStr, CString, OsStr},
    fmt::Write,
    fs, io,
    os::unix::{fs::chroot, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};

use error_stack::{IntoReport, Result, ResultExt};
use nix::mount::{mount, MsFlags};
use rustix::{
    fs::{cwd, statx, AtFlags, StatxFlags},
    process::getuid,
};

use crate::path_undo::TmpPath;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("An IO error occurred.")]
    Io,
    #[error("Invalid argument.")]
    InvalidArgument,
    #[error("ForkFS must be run as root.")]
    NotRoot,
}

/// # Errors
///
/// Forwards I/O errors.
pub fn forkfs<T: AsRef<OsStr>>(session: &str, command: &[T]) -> Result<(), Error> {
    if !getuid().is_root() {
        return Err(Error::NotRoot).into_report();
    }

    let session_dir = dirs::cache_dir();
    let mut session_dir = session_dir
        .as_deref()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("forkfs");
    session_dir.push(session);

    if !maybe_create_session(&mut session_dir)? {
        mount_session(&mut session_dir)?;
    }

    session_dir.push("merged");
    enter_session(&session_dir)?;

    run_command(command)
}

fn maybe_create_session(dir: &mut PathBuf) -> Result<bool, Error> {
    let session_active = 'active: {
        let mount = {
            let merged = TmpPath::new(dir, "merged");
            match statx(cwd(), &*merged, AtFlags::empty(), StatxFlags::MNT_ID) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    break 'active false;
                }
                r => r,
            }
            .into_report()
            .attach_printable_lazy(|| format!("Failed to stat {merged:?}"))
            .change_context(Error::Io)?
            .stx_mnt_id
        };

        let parent_mount = statx(cwd(), dir.as_path(), AtFlags::empty(), StatxFlags::MNT_ID)
            .into_report()
            .attach_printable_lazy(|| format!("Failed to stat {dir:?}"))
            .change_context(Error::Io)?
            .stx_mnt_id;

        parent_mount != mount
    };

    if !session_active {
        for path in ["diff", "work", "merged"] {
            let dir = TmpPath::new(dir, path);
            fs::create_dir_all(&dir)
                .into_report()
                .attach_printable_lazy(|| format!("Failed to create directory {dir:?}"))
                .change_context(Error::Io)?;
        }
    }

    Ok(session_active)
}

fn mount_session(dir: &mut PathBuf) -> Result<(), Error> {
    const OVERLAY: &CStr = CStr::from_bytes_with_nul(b"overlay\0").ok().unwrap();

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

    let dir = TmpPath::new(dir, "merged");
    mount(
        Some(OVERLAY),
        &*dir,
        Some(OVERLAY),
        MsFlags::empty(),
        Some(command.as_c_str()),
    )
    .into_report()
    .attach_printable_lazy(|| format!("Failed to mount directory {dir:?}"))
    .change_context(Error::Io)
}

fn enter_session(target: &Path) -> Result<(), Error> {
    // Must be retrieved before chroot-ing
    let current_dir = current_dir()
        .into_report()
        .attach_printable("Failed to get current directory")
        .change_context(Error::Io)?;

    chroot(target)
        .into_report()
        .attach_printable_lazy(|| format!("Failed to change root {target:?}"))
        .change_context(Error::Io)?;
    set_current_dir(current_dir)
        .into_report()
        .attach_printable_lazy(|| format!("Failed to change current directory {target:?}"))
        .change_context(Error::Io)
}

fn run_command(args: &[impl AsRef<OsStr>]) -> Result<(), Error> {
    let mut command = Command::new(args[0].as_ref());

    // Downgrade privilege level to pre-sudo if possible
    if let Some(uid) = env::var_os("SUDO_UID").as_ref().and_then(|s| s.to_str())
        && let Ok(uid) = uid.parse() {
        command.uid(uid);
    }

    Err(command.args(&args[1..]).exec())
        .into_report()
        .attach_printable_lazy(|| {
            format!(
                "Failed to exec {:?}",
                args.iter().map(AsRef::as_ref).collect::<Vec<_>>()
            )
        })
        .change_context(Error::Io)
}

mod path_undo {
    use std::{
        fmt::{Debug, Formatter},
        ops::{Deref, DerefMut},
        path::{Path, PathBuf},
    };

    pub struct TmpPath<'a>(&'a mut PathBuf);

    impl<'a> TmpPath<'a> {
        pub fn new(path: &'a mut PathBuf, child: impl AsRef<Path>) -> Self {
            path.push(child);
            Self(path)
        }
    }

    impl<'a> Deref for TmpPath<'a> {
        type Target = PathBuf;

        fn deref(&self) -> &Self::Target {
            self.0
        }
    }

    impl<'a> AsRef<Path> for TmpPath<'a> {
        fn as_ref(&self) -> &Path {
            self.0
        }
    }

    impl<'a> DerefMut for TmpPath<'a> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.0
        }
    }

    impl<'a> Debug for TmpPath<'a> {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            Debug::fmt(&**self, f)
        }
    }

    impl<'a> Drop for TmpPath<'a> {
        fn drop(&mut self) {
            self.pop();
        }
    }
}
