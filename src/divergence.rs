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

    pub fn is_direct_child_of_included_parent(&self, file: &Path) -> bool {
        let file = file.absolutize().unwrap(); // TODO create type that wraps an absolutized path
        let file = file.as_ref();
        let nearest_parent = self.changes.get_raw_descendant(file)
            .and_then(|trie| trie.keys().next())
            .and_then(|buf| buf.as_path().parent());

        nearest_parent == Some(file)
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
                    // TODO add tests validating correct behavior
                    //  (in particular what happens in edge cases like trying to open a directory)
                    fs::copy(file, &relocated)
                        .with_context(|| format!("Copy from {:?} to {:?} failed", file, relocated))
                        .with_code(exitcode::IOERR)?;
                }
                ChangeType::Remove => {
                    // TODO don't create file since we know it's going to immediately be deleted.
                    File::create(&relocated)
                        .with_context(|| format!("Failed to create file {:?}", relocated))
                        .with_code(exitcode::IOERR)?;
                }
            }
        }

        Ok(relocated)
    }

    pub fn on_read_dir(
        &mut self,
        file: &Path,
    ) -> Result<PathBuf, CliExitError> {
        let relocated = self.destination(file);

        let entries = file.read_dir()
            .with_context(|| format!("Unable to read dir {:?}", file))
            .with_code(exitcode::IOERR)?;

        for entry in entries {
            let entry = entry
                .with_context(|| format!("Unable to read dir entry in {:?}", file))
                .with_code(exitcode::IOERR)?;
            let metadata = entry.metadata()
                .with_context(|| format!("Unable to read metadata for {:?}", entry))
                .with_code(exitcode::IOERR)?;

            let relocated_path = relocated.join(entry.file_name());
            if relocated_path.exists() { continue; }

            if metadata.is_dir() {
                fs::create_dir(&relocated_path)
                    .with_context(|| format!("Creating dir {:?} failed", relocated_path))
                    .with_code(exitcode::IOERR)?;
            } else {
                let path = entry.path();
                fs::copy(&path, &relocated_path)
                    .with_context(|| format!("Copy from {:?} to {:?} failed", path, relocated_path))
                    .with_code(exitcode::IOERR)?;
            }
        }

        Ok(relocated)
    }

    fn log_modification(&mut self, file: &Path, change: ChangeType) -> CliResult<()> {
        let file = file.absolutize().unwrap();
        self.changes.insert(file.to_path_buf(), change);

        // TODO replace this garbage format with https://github.com/bincode-org/bincode
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
