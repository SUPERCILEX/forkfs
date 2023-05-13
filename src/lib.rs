#![feature(let_chains)]
#![feature(dir_entry_ext2)]
#![feature(c_str_literals)]
#![allow(clippy::missing_errors_doc)]

use std::{
    fmt::{Debug, Display},
    io,
    path::PathBuf,
};

use error_stack::{IntoReport, Result, ResultExt};
pub use run::run;
pub use sessions::{
    delete as delete_sessions, list as list_sessions, stop as stop_sessions, Op as SessionOperand,
};

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
    #[error("Setup required.")]
    SetupRequired,
}

fn get_sessions_dir() -> PathBuf {
    let mut sessions_dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    sessions_dir.push("forkfs");
    sessions_dir
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
