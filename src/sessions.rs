use std::{
    ffi::CString,
    fmt::Write as FmtWrite,
    fs,
    fs::DirEntry,
    io,
    io::{ErrorKind, Write},
    os::unix::fs::DirEntryExt2,
    path::{Path, PathBuf},
};

use error_stack::{Result, ResultExt};
use rustix::{
    fs::{AtFlags, CWD, StatxFlags, statx},
    mount::{
        MountFlags, MountPropagationFlags, UnmountFlags, mount, mount_bind_recursive, mount_change,
        unmount,
    },
};

use crate::{Error, IoErr, get_sessions_dir, path_undo::TmpPath};

#[derive(Copy, Clone)]
pub enum Op<'a, S> {
    All,
    List(&'a [S]),
}

pub fn list() -> Result<(), Error> {
    let mut stdout = io::stdout().lock();
    let mut is_first = true;
    iter_all_sessions(|entry, session| {
        let name = entry.file_name_ref().to_string_lossy();
        let session_active = is_active_session(session, true)?;

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

pub fn maybe_create_session(dir: &mut PathBuf) -> Result<(), Error> {
    if is_active_session(dir, false)? {
        return Ok(());
    }

    for path in ["diff", "work", "merged"] {
        let dir = TmpPath::new(dir, path);
        fs::create_dir_all(&dir)
            .map_io_err_lazy(|| format!("Failed to create directory {dir:?}"))?;
    }
    start_session(dir)
}

fn start_session(dir: &mut PathBuf) -> Result<(), Error> {
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
            .attach_printable("Invalid path bytes")
            .change_context(Error::InvalidArgument)?
    };

    let mut merged = TmpPath::new(dir, "merged");
    mount(
        c"overlay",
        &*merged,
        c"overlay",
        MountFlags::empty(),
        command.as_c_str(),
    )
    .map_io_err_lazy(|| format!("Failed to mount directory {merged:?}"))?;

    for (source, target) in [
        (c"/proc", "proc"),
        (c"/dev", "dev"),
        (c"/run", "run"),
        (c"/tmp", "tmp"),
    ] {
        let target = TmpPath::new(&mut merged, target);
        mount_bind_recursive(source, &*target)
            .map_io_err_lazy(|| format!("Failed to bind mount directory {target:?}"))?;
        mount_change(
            &*target,
            MountPropagationFlags::DOWNSTREAM | MountPropagationFlags::REC,
        )
        .map_io_err_lazy(|| format!("Failed to enslave mount {target:?}"))?;
    }

    Ok(())
}

fn stop_session(session: &mut PathBuf) -> Result<(), Error> {
    if !is_active_session(session, true)? {
        return Ok(());
    }

    let mut merged = TmpPath::new(session, "merged");

    for target in ["proc", "dev", "run", "tmp"] {
        let target = TmpPath::new(&mut merged, target);
        unmount(&*target, UnmountFlags::DETACH)
            .map_io_err_lazy(|| format!("Failed to unmount directory {target:?}"))?;
    }

    unmount(&*merged, UnmountFlags::empty())
        .map_io_err_lazy(|| format!("Failed to unmount directory {merged:?}"))
}

fn delete_session(session: &Path) -> Result<(), Error> {
    fuc_engine::remove_dir_all(session)
        .attach_printable_lazy(|| format!("Failed to delete directory {session:?}"))
        .change_context(Error::Io)
}

fn iter_all_sessions(
    mut f: impl FnMut(DirEntry, &mut PathBuf) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut sessions_dir = get_sessions_dir();
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
            let mut sessions_dir = get_sessions_dir();
            for session in sessions {
                let mut session = TmpPath::new(&mut sessions_dir, session.as_ref());
                f(&mut session)?;
            }
            Ok(())
        }
    }
}

fn is_active_session(session: &mut PathBuf, must_exist: bool) -> Result<bool, Error> {
    let mount = {
        let merged = TmpPath::new(session, "merged");
        match statx(CWD, &*merged, AtFlags::empty(), StatxFlags::MNT_ID) {
            Err(e) if !must_exist && e.kind() == ErrorKind::NotFound => {
                return Ok(false);
            }
            r => r,
        }
        .map_io_err_lazy(|| format!("Failed to stat {merged:?}"))
        .change_context(Error::SessionNotFound)?
        .stx_mnt_id
    };

    let parent_mount = statx(CWD, &*session, AtFlags::empty(), StatxFlags::MNT_ID)
        .map_io_err_lazy(|| format!("Failed to stat {session:?}"))?
        .stx_mnt_id;

    Ok(parent_mount != mount)
}
