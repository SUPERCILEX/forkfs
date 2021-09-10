use anyhow::{anyhow, Result};
use exitcode::ExitCode;

pub type CliResult<T> = Result<T, CliExitError>;

#[derive(Debug)]
pub struct CliExitError {
    pub code: ExitCode,
    pub source: Option<anyhow::Error>,
}

pub trait CliExitAnyhowWrapper<T> {
    fn with_code(self, error_code: i32) -> Result<T, CliExitError>;
}

impl<T> CliExitAnyhowWrapper<T> for Result<T> {
    fn with_code(self, error_code: i32) -> Result<T, CliExitError> {
        self.map_err(|e| CliExitError {
            code: error_code,
            source: Some(e),
        })
    }
}

pub trait CliExitNixWrapper<T> {
    fn with_backing_code(self, message: impl Fn() -> String) -> Result<T, CliExitError>;
}

impl<T> CliExitNixWrapper<T> for nix::Result<T> {
    fn with_backing_code(self, message: impl Fn() -> String) -> Result<T, CliExitError> {
        self.map_err(|e| CliExitError {
            code: e as i32,
            source: Some(anyhow!(e).context(message())),
        })
    }
}
