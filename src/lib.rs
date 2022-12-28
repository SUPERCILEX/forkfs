#![feature(let_chains)]
#![feature(const_option)]
#![feature(const_result_drop)]
#![feature(const_cstr_methods)]
#![feature(dir_entry_ext2)]
#![allow(clippy::missing_errors_doc)]

use std::{
    fmt::{Debug, Display},
    io,
    path::PathBuf,
};

use error_stack::{IntoReport, Result, ResultExt};
pub use run::run;
use rustix::{
    fs::{cwd, statx, AtFlags, StatxFlags},
    process::getuid,
};
pub use sessions::{
    delete as delete_sessions, list as list_sessions, stop as stop_sessions, Op as SessionOperand,
};

use crate::path_undo::TmpPath;

mod run;
mod sessions;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("An IO error occurred.")]
    Io,
    #[error("Invalid argument.")]
    InvalidArgument,
    #[error("ForkFS must be run as root.")]
    NotRoot,
    #[error("Session not found.")]
    SessionNotFound,
}

fn get_sessions_dir() -> Result<PathBuf, Error> {
    if !getuid().is_root() {
        return Err(Error::NotRoot).into_report();
    }

    let mut sessions_dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    sessions_dir.push("forkfs");
    Ok(sessions_dir)
}

fn is_active_session(session: &mut PathBuf, must_exist: bool) -> Result<bool, Error> {
    let mount = {
        let merged = TmpPath::new(session, "merged");
        match statx(cwd(), &*merged, AtFlags::empty(), StatxFlags::MNT_ID) {
            Err(e) if !must_exist && e.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            r => r,
        }
        .map_io_err_lazy(|| format!("Failed to stat {merged:?}"))
        .change_context(Error::SessionNotFound)?
        .stx_mnt_id
    };

    let parent_mount = statx(
        cwd(),
        session.as_path(),
        AtFlags::empty(),
        StatxFlags::MNT_ID,
    )
    .map_io_err_lazy(|| format!("Failed to stat {session:?}"))?
    .stx_mnt_id;

    Ok(parent_mount != mount)
}

trait IoErr<Out> {
    fn map_io_err_lazy<P: Display + Debug + Send + Sync + 'static>(
        self,
        f: impl FnOnce() -> P,
    ) -> Out;

    fn map_io_err<P: Display + Debug + Send + Sync + 'static>(self, p: P) -> Out;
}

impl<T> IoErr<Result<T, Error>> for io::Result<T> {
    fn map_io_err_lazy<P: Display + Debug + Send + Sync + 'static>(
        self,
        f: impl FnOnce() -> P,
    ) -> Result<T, Error> {
        self.into_report()
            .attach_printable_lazy(f)
            .change_context(Error::Io)
    }

    fn map_io_err<P: Display + Debug + Send + Sync + 'static>(self, p: P) -> Result<T, Error> {
        self.into_report()
            .attach_printable(p)
            .change_context(Error::Io)
    }
}

impl<T> IoErr<Result<T, Error>> for std::result::Result<T, rustix::io::Errno> {
    fn map_io_err_lazy<P: Display + Debug + Send + Sync + 'static>(
        self,
        f: impl FnOnce() -> P,
    ) -> Result<T, Error> {
        self.map_err(io::Error::from).map_io_err_lazy(f)
    }

    fn map_io_err<P: Display + Debug + Send + Sync + 'static>(self, p: P) -> Result<T, Error> {
        self.map_err(io::Error::from).map_io_err(p)
    }
}

impl<T> IoErr<Result<T, Error>> for std::result::Result<T, nix::Error> {
    fn map_io_err_lazy<P: Display + Debug + Send + Sync + 'static>(
        self,
        f: impl FnOnce() -> P,
    ) -> Result<T, Error> {
        self.map_err(io::Error::from).map_io_err_lazy(f)
    }

    fn map_io_err<P: Display + Debug + Send + Sync + 'static>(self, p: P) -> Result<T, Error> {
        self.map_err(io::Error::from).map_io_err(p)
    }
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
