use std::{
    fs,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context};
use derive_new::new;
use log::info;
use nix::NixPath;
use path_absolutize::Absolutize;
use radix_trie::{Trie, TrieCommon};

use crate::errors::{CliExitAnyhowWrapper, CliExitError, CliResult, IoResultUtils};

#[derive(new, Debug)]
pub struct FileChanges {
    #[new(default)]
    changes: Trie<PathBuf, ChangeType>,
    log_file: PathBuf,
    root: PathBuf,
}

#[derive(Debug, Copy, Clone)]
pub enum ChangeType {
    Modify,
    Remove,
}

impl FileChanges {
    pub fn includes(&self, file: &Path) -> bool {
        self.changes.get(file).is_some()
    }

    pub fn destination(&self, file: &Path) -> PathBuf {
        self.root
            .join(file.absolutize().unwrap().strip_prefix("/").unwrap())
    }

    pub fn restore_from_disk(&mut self) -> CliResult<()> {
        assert!(self.changes.is_empty());

        let open_result = OpenOptions::new().read(true).open(&self.log_file);
        if open_result.as_ref().does_not_exist() {
            return Ok(());
        }
        let reader = BufReader::new(
            open_result
                .with_context(|| format!("Failed to open op log {:?}", self.log_file))
                .with_code(exitcode::IOERR)?,
        );

        for line in reader.lines() {
            let line = line
                .with_context(|| format!("Failed to read op log {:?}", self.log_file))
                .with_code(exitcode::IOERR)?;

            let op_type = match line.as_bytes().get(0) {
                Some(c) => c,
                None => continue,
            };
            let path = PathBuf::from(
                line.get(2..)
                    .ok_or_else(|| anyhow!("Log file parsing error: invalid entry"))
                    .with_code(exitcode::DATAERR)?,
            );

            self.changes.insert(
                path,
                match *op_type {
                    b'M' => ChangeType::Modify,
                    b'R' => ChangeType::Remove,
                    _ => {
                        return Err(anyhow!(
                            "Log file parsing error: unknown op type {:?}",
                            op_type
                        ))
                            .with_code(exitcode::DATAERR);
                    }
                },
            );
        }

        Ok(())
    }

    pub fn on_file_modified(&mut self, file: &Path) -> CliResult<PathBuf> {
        self.on_file_changed(file, ChangeType::Modify)
    }

    pub fn on_file_removed(&mut self, file: &Path) -> CliResult<PathBuf> {
        self.on_file_changed(file, ChangeType::Remove)
    }

    fn on_file_changed(
        &mut self,
        file: &Path,
        change: ChangeType,
    ) -> Result<PathBuf, CliExitError> {
        let relocated = self.destination(file);
        let relocated_parent = relocated.parent().unwrap();

        info!("Creating dir {:?}", relocated_parent);
        fs::create_dir_all(relocated_parent)
            .with_context(|| format!("Failed to create directory {:?}", relocated_parent))
            .with_code(exitcode::IOERR)?;

        info!("Rewriting path {:?} to {:?}", file, relocated);
        self.log_modification(file, change)?;

        if !relocated.exists() && file.exists() {
            info!("Copying file {:?} to {:?}", file, relocated);
            match change {
                ChangeType::Modify => {
                    fs::copy(file, &relocated)
                        .with_context(|| format!("Copy from {:?} to {:?} failed", file, relocated))
                        .with_code(exitcode::IOERR)?;
                }
                ChangeType::Remove => {
                    File::create(&relocated)
                        .with_context(|| format!("Failed to create file {:?}", relocated))
                        .with_code(exitcode::IOERR)?;
                }
            }
        }

        Ok(relocated)
    }

    fn log_modification(&mut self, file: &Path, change: ChangeType) -> CliResult<()> {
        self.changes.insert(file.to_path_buf(), change);

        let mut buf = Vec::with_capacity(2 + file.len() + 1);
        buf.extend_from_slice(
            match change {
                ChangeType::Modify => "M ",
                ChangeType::Remove => "R ",
            }
                .as_bytes(),
        );
        buf.extend_from_slice(file.to_str().unwrap().as_bytes());
        buf.push(b'\n');

        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.log_file)
            .with_context(|| format!("Failed to open op log {:?}", self.log_file))
            .with_code(exitcode::IOERR)?
            .write_all(&buf)
            .with_context(|| format!("Failed to write to op log {:?}", self.log_file))
            .with_code(exitcode::IOERR)?;

        Ok(())
    }
}
