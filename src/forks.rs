use std::{
    fs::{read_dir, remove_dir_all, remove_file},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context};
use log::info;

use crate::{
    CliResult,
    errors::{CliExitAnyhowWrapper, IoResultUtils},
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

pub fn remove_forks(forks: Vec<String>) -> CliResult<()> {
    let forks_dir = forks_dir()?;

    if forks.is_empty() {
        info!("Deleting directory {:?}", forks_dir);

        let result = remove_dir_all(&forks_dir);
        return if result.as_ref().does_not_exist() {
            info!("No forks to remove");
            Ok(())
        } else {
            result
                .with_context(|| format!("Failed to delete {:?}", forks_dir))
                .with_code(exitcode::IOERR)
        };
    }

    let mut had_errors = false;
    for fork in forks {
        let fork_dir = forks_dir.join(&fork);
        let log_file = fork_dir.with_extension("changes");
        info!("Deleting dir {:?} and file {:?}", fork_dir, log_file);

        // TODO parallelize this
        let result = remove_dir_all(&fork_dir).and(remove_file(log_file));

        if result.is_err() {
            had_errors = true;

            if result.as_ref().does_not_exist() {
                eprintln!("Fork '{}' not found", fork);
            } else {
                eprintln!(
                    "{:?}",
                    result.with_context(|| format!("Failed to delete {:?}", forks_dir))
                );
            }
        }
    }

    if had_errors {
        println!();
        Err(anyhow!("Failed to delete some forks")).with_code(exitcode::IOERR)
    } else {
        Ok(())
    }
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
