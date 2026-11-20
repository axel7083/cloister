//! Where Secure Folder keeps its files, following the XDG base directories.

use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::id::VaultId;

const APP_DIR: &str = "secure-folder";

/// Resolved base directories. Build with [`Paths::from_env`] in services,
/// [`Paths::new`] in tests.
#[derive(Debug, Clone)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl Paths {
    pub fn new(config_dir: PathBuf, data_dir: PathBuf, runtime_dir: PathBuf) -> Self {
        Self {
            config_dir,
            data_dir,
            runtime_dir,
        }
    }

    /// Reads `XDG_CONFIG_HOME`, `XDG_DATA_HOME` (falling back to `$HOME`) and
    /// `XDG_RUNTIME_DIR` (required: mounts must live on the per-user tmpfs).
    pub fn from_env() -> Result<Self> {
        let home = || abs_env("HOME").ok_or(Error::MissingEnv("HOME"));
        let config_dir = match abs_env("XDG_CONFIG_HOME") {
            Some(p) => p,
            None => home()?.join(".config"),
        };
        let data_dir = match abs_env("XDG_DATA_HOME") {
            Some(p) => p,
            None => home()?.join(".local/share"),
        };
        let runtime_dir = abs_env("XDG_RUNTIME_DIR").ok_or(Error::MissingEnv("XDG_RUNTIME_DIR"))?;
        Ok(Self::new(config_dir, data_dir, runtime_dir))
    }

    /// `~/.config/secure-folder/vaults.toml`
    pub fn registry_file(&self) -> PathBuf {
        self.config_dir.join(APP_DIR).join("vaults.toml")
    }

    /// `~/.local/share/secure-folder/<id>`: encrypted files of a new vault.
    pub fn default_cipher_dir(&self, id: &VaultId) -> PathBuf {
        self.data_dir.join(APP_DIR).join(id.to_string())
    }

    /// `$XDG_RUNTIME_DIR/secure-folder/<id>`: where a vault is mounted. On
    /// tmpfs (gone at reboot) and outside `$HOME`, so GIO does not list the
    /// raw FUSE mount next to our own sidebar volume.
    pub fn mount_point(&self, id: &VaultId) -> PathBuf {
        self.runtime_dir.join(APP_DIR).join(id.to_string())
    }
}

fn abs_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// `mkdir -p` where every directory created gets mode 0700.
pub fn create_private_dir_all(path: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let p = Paths::new("/c".into(), "/d".into(), "/r".into());
        let id: VaultId = "67e55044-10b1-426f-9247-bb680e5fe0c8".parse().unwrap();
        assert_eq!(p.registry_file(), Path::new("/c/secure-folder/vaults.toml"));
        assert_eq!(
            p.default_cipher_dir(&id),
            Path::new("/d/secure-folder/67e55044-10b1-426f-9247-bb680e5fe0c8")
        );
        assert_eq!(
            p.mount_point(&id),
            Path::new("/r/secure-folder/67e55044-10b1-426f-9247-bb680e5fe0c8")
        );
    }
}
