use std::{
    fs,
    fs::DirEntry,
    io,
    io::{ErrorKind, Write},
    os::unix::fs::DirEntryExt2,
    path::{Path, PathBuf},
};

use error_stack::Result;
use nix::mount::umount;

use crate::{get_sessions_dir, is_active_session, path_undo::TmpPath, Error, IoErr};

pub enum Op<'a, S: AsRef<str>> {
    All,
    List(&'a [S]),
}

pub fn list() -> Result<(), Error> {
    let mut stdout = io::stdout().lock();
    let mut is_first = true;
    iter_all_sessions(|entry, session| {
        let name = entry.file_name_ref().to_string_lossy();
        let session_active = is_active_session(session)?;

        let mut print = || {
            if !is_first {
                write!(stdout, ", ")?;
            }
            if session_active {
                write!(stdout, "[{name}]")
            } else {
                write!(stdout, "{name}")
            }
        };

        print().map_io_err("Failed to write to stdout")?;
        is_first = false;

        Ok(())
    })
}

pub fn stop<S: AsRef<str>>(sessions: Op<S>) -> Result<(), Error> {
    iter_op(sessions, stop_session)
}

pub fn delete<S: AsRef<str>>(sessions: Op<S>) -> Result<(), Error> {
    iter_op(sessions, |session| {
        stop_session(session)?;
        delete_session(session)
    })
}

fn stop_session(session: &mut PathBuf) -> Result<(), Error> {
    if !is_active_session(session)? {
        return Ok(());
    }

    let merged = TmpPath::new(session, "merged");
    umount(merged.as_path()).map_io_err_lazy(|| format!("Failed to unmount directory {merged:?}"))
}

fn delete_session(session: &Path) -> Result<(), Error> {
    fs::remove_dir_all(session)
        .map_io_err_lazy(|| format!("Failed to delete directory {session:?}"))
}

fn iter_all_sessions(
    mut f: impl FnMut(DirEntry, &mut PathBuf) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut sessions_dir = get_sessions_dir()?;
    for entry in match fs::read_dir(&sessions_dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        r => r.map_io_err_lazy(|| format!("Failed to open directory {sessions_dir:?}"))?,
    } {
        let entry =
            entry.map_io_err_lazy(|| format!("Failed to read directory {sessions_dir:?}"))?;
        let mut session = TmpPath::new(&mut sessions_dir, entry.file_name_ref());

        f(entry, &mut session)?;
    }
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn iter_op<S: AsRef<str>>(
    sessions: Op<S>,
    mut f: impl FnMut(&mut PathBuf) -> Result<(), Error>,
) -> Result<(), Error> {
    match sessions {
        Op::All => iter_all_sessions(|_, session| f(session)),
        Op::List(sessions) => {
            let mut sessions_dir = get_sessions_dir()?;
            for session in sessions {
                let mut session = TmpPath::new(&mut sessions_dir, session.as_ref());
                f(&mut session)?;
            }
            Ok(())
        }
    }
}
