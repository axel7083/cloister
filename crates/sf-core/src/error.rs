use std::path::PathBuf;

use crate::id::InvalidVaultId;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors of the core crate. Messages never contain secret material.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    InvalidId(#[from] InvalidVaultId),

    #[error("environment variable {0} is not set to an absolute path")]
    MissingEnv(&'static str),

    #[error("invalid configuration {path}: {reason}")]
    Config { path: PathBuf, reason: String },

    #[error("invalid vault name: {0}")]
    InvalidName(&'static str),

    #[error("cipher directory must be an absolute path")]
    RelativeCipherDir,

    #[error("a vault with this id already exists")]
    DuplicateVault,

    #[error("no such vault")]
    UnknownVault,

    #[error("invalid passphrase: {0}")]
    InvalidPassphrase(&'static str),

    #[error("wrong passphrase")]
    WrongPassphrase,

    #[error("directory is not empty: {0}")]
    NotEmpty(PathBuf),

    #[error("files in the vault are still in use")]
    Busy,

    #[error("vault did not mount within {0} seconds")]
    MountTimeout(u64),

    #[error("{program} failed (exit code {code:?}): {message}")]
    Tool {
        program: &'static str,
        code: Option<i32>,
        message: String,
    },
}
