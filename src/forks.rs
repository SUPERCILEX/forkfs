use std::{
    fs::{read_dir, remove_dir, remove_file, symlink_metadata},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context};
use log::info;
use tokio::{runtime::Builder, task::JoinHandle};

use crate::{
    CliResult,
    errors::{CliExitAnyhowWrapper, IoResultUtils}, RemoveForks,
};

pub fn get_fork(fork: String) -> CliResult<PathBuf> {
    let mut fork_dir = forks_dir()?;
    if fork.is_empty() {
        loop {
            fork_dir.push(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    .to_string(),
            );

            if !fork_dir.exists() {
                break;
            }

            fork_dir.pop();
        }
    } else {
        fork_dir.push(fork);
    }

    Ok(fork_dir)
}

pub fn list_forks() -> CliResult<()> {
    fn show_no_forks_help() {
        println!("No forks found. Learn how to create a fork:");
        println!("  forkfs run --help");
    }

    let forks_dir = forks_dir()?;
    let forks_result = read_dir(&forks_dir);

    if forks_result.as_ref().does_not_exist() {
        show_no_forks_help();
        return Ok(());
    }

    let forks = forks_result
        .with_context(|| format!("Failed to list files in dir: {:?}", forks_dir))
        .with_code(exitcode::IOERR)?;

    let mut had_entries = false;
    for fork in forks {
        let entry = fork
            .with_context(|| format!("Failed to read entry in dir: {:?}", forks_dir))
            .with_code(exitcode::IOERR)?;

        let file_name = entry.file_name();
        let file_name = file_name.to_str().unwrap();
        if file_name.ends_with(".changes") {
            continue;
        }

        had_entries = true;
        print!("{} ", file_name);
    }

    if had_entries {
        println!();
    } else {
        show_no_forks_help();
    }

    Ok(())
}

pub fn diff_fork(_fork: String) -> CliResult<()> {
    todo!()
}

pub fn apply_fork(_fork: String) -> CliResult<()> {
    todo!()
}

pub fn remove_forks(remove: RemoveForks) -> CliResult<()> {
    let runtime = Builder::new_current_thread()
        .build()
        .with_context(|| "Failed to create tokio runtime")
        .with_code(exitcode::OSERR)?;

    runtime.block_on(remove_forks_async(remove))
}

async fn fast_remove_dir_all(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    let path = tokio::task::spawn_blocking(|| -> io::Result<_> {
        let filetype = symlink_metadata(&path)?.file_type();
        if filetype.is_symlink() {
            remove_file(&path)?;
            Ok(None)
        } else {
            Ok(Some(path))
        }
    })
        .await??;

    match path {
        None => Ok(()),
        Some(path) => spawn_remove_dir_all_recursive(path).await?,
    }
}

async fn remove_dir_all_recursive(path: PathBuf) -> io::Result<()> {
    let mut tasks = Vec::new();

    for child in read_dir(&path)? {
        let child = child?;
        if child.file_type()?.is_dir() {
            tasks.push(spawn_remove_dir_all_recursive(child.path()));
        } else {
            remove_file(&child.path())?;
        }
    }

    for task in tasks {
        task.await??;
    }

    remove_dir(path)
}

#[inline]
fn spawn_remove_dir_all_recursive(path: PathBuf) -> JoinHandle<io::Result<()>> {
    tokio::task::spawn_blocking(|| {
        futures::executor::block_on(remove_dir_all_recursive(path))
    })
}

async fn remove_forks_async(remove: RemoveForks) -> CliResult<()> {
    let forks_dir = forks_dir()?;

    if remove.all {
        info!("Deleting directory {:?}", forks_dir);

        let result = fast_remove_dir_all(&forks_dir).await;
        return if result.as_ref().does_not_exist() {
            info!("No forks to remove");
            Ok(())
        } else {
            result
                .with_context(|| format!("Failed to delete {:?}", forks_dir))
                .with_code(exitcode::IOERR)
        };
    }

    let mut deletions = Vec::with_capacity(remove.forks.len());
    for fork in remove.forks {
        let forks_dir = forks_dir.clone();

        deletions.push(tokio::spawn(async move {
            let fork_dir = forks_dir.join(&fork);
            info!("Deleting dir {:?}", fork_dir);

            let result = fast_remove_dir_all(&fork_dir).await;
            if result.is_err() {
                if result.as_ref().does_not_exist() {
                    eprintln!("Fork '{}' not found", fork);
                } else {
                    eprintln!(
                        "{:?}",
                        result.with_context(|| format!("Failed to delete {:?}", forks_dir))
                    );
                }

                Err(())
            } else {
                Ok(())
            }
        }));
    }

    for result in deletions {
        if result.await.unwrap().is_err() {
            println!();
            return Err(anyhow!("Failed to delete some forks")).with_code(exitcode::IOERR);
        }
    }
    Ok(())
}

fn forks_dir() -> CliResult<PathBuf> {
    let mut fork_dir = forkfs_dir()?;
    fork_dir.push("forks");
    Ok(fork_dir)
}

fn forkfs_dir() -> CliResult<PathBuf> {
    let mut config_dir = dirs::config_dir()
        .context("Failed to retrieve system configuration directory.")
        .with_code(exitcode::CONFIG)?;
    config_dir.push("forkfs");
    Ok(config_dir)
}
