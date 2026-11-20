//! The list of vaults (`vaults.toml`).
//!
//! The session service is the only writer; other components read it through
//! the `Vaults1` D-Bus API. Saves are atomic (temporary file, fsync, rename).

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::id::VaultId;
use crate::paths::create_private_dir_all;
use crate::vault::Vault;

const FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub struct VaultRegistry {
    path: PathBuf,
    vaults: Vec<Vault>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileFormat {
    version: u32,
    #[serde(default, rename = "vault")]
    vaults: Vec<Vault>,
}

impl VaultRegistry {
    /// Loads `path`; a missing file is an empty registry. Refuses files with
    /// a newer format version so they are never overwritten by an old build.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    vaults: Vec::new(),
                })
            }
            Err(e) => return Err(e.into()),
        };
        let config_err = |reason: String| Error::Config {
            path: path.clone(),
            reason,
        };
        let file: FileFormat =
            toml::from_str(&text).map_err(|e| config_err(e.message().to_owned()))?;
        if file.version != FORMAT_VERSION {
            return Err(config_err(format!("unsupported version {}", file.version)));
        }
        let mut ids: Vec<_> = file.vaults.iter().map(Vault::id).collect();
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(config_err("duplicate vault id".into()));
        }
        Ok(Self {
            path,
            vaults: file.vaults,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> &[Vault] {
        &self.vaults
    }

    pub fn get(&self, id: &VaultId) -> Option<&Vault> {
        self.vaults.iter().find(|v| v.id() == id)
    }

    /// Adds a vault and saves.
    pub fn add(&mut self, vault: Vault) -> Result<()> {
        if self.get(vault.id()).is_some() {
            return Err(Error::DuplicateVault);
        }
        self.vaults.push(vault);
        if let Err(e) = self.save() {
            self.vaults.pop();
            return Err(e);
        }
        Ok(())
    }

    /// Removes a vault from the list and saves. Encrypted files are kept.
    pub fn remove(&mut self, id: &VaultId) -> Result<Vault> {
        let index = self
            .vaults
            .iter()
            .position(|v| v.id() == id)
            .ok_or(Error::UnknownVault)?;
        let removed = self.vaults.remove(index);
        if let Err(e) = self.save() {
            self.vaults.insert(index, removed);
            return Err(e);
        }
        Ok(removed)
    }

    fn save(&self) -> Result<()> {
        let invalid = || Error::Config {
            path: self.path.clone(),
            reason: "not a file path".into(),
        };
        let dir = self.path.parent().ok_or_else(invalid)?;
        create_private_dir_all(dir)?;
        let text = toml::to_string_pretty(&FileFormat {
            version: FORMAT_VERSION,
            vaults: self.vaults.clone(),
        })
        .map_err(|e| Error::Config {
            path: self.path.clone(),
            reason: e.to_string(),
        })?;

        let file_name = self.path.file_name().ok_or_else(invalid)?;
        let tmp = dir.join(format!(".{}.tmp", file_name.to_string_lossy()));
        match fs::remove_file(&tmp) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &self.path)?;
        File::open(dir)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn vault(name: &str) -> Vault {
        Vault::new(VaultId::generate(), name, "/tmp/cipher".into()).unwrap()
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let r = VaultRegistry::load(dir.path().join("vaults.toml")).unwrap();
        assert!(r.list().is_empty());
    }

    #[test]
    fn round_trip_and_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/vaults.toml");
        let mut r = VaultRegistry::load(&path).unwrap();
        let a = vault("A");
        let b = vault("B");
        r.add(a.clone()).unwrap();
        r.add(b.clone()).unwrap();
        assert!(matches!(r.add(a.clone()), Err(Error::DuplicateVault)));

        let r2 = VaultRegistry::load(&path).unwrap();
        assert_eq!(r2.list(), &[a.clone(), b.clone()]);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let mut r3 = r2;
        assert_eq!(r3.remove(a.id()).unwrap(), a);
        assert!(matches!(r3.remove(a.id()), Err(Error::UnknownVault)));
        assert_eq!(VaultRegistry::load(&path).unwrap().list(), &[b]);
    }

    #[test]
    fn rejects_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vaults.toml");
        let id = VaultId::generate();
        for (text, why) in [
            ("version = 2\n", "newer version"),
            ("version = 1\nextra = 1\n", "unknown field"),
            (
                "version = 1\n[[vault]]\nid = \"../x\"\nname = \"a\"\ncipher_dir = \"/c\"\n",
                "bad id",
            ),
            (
                &format!(
                    "version = 1\n[[vault]]\nid = \"{id}\"\nname = \"a\"\ncipher_dir = \"rel\"\n"
                ),
                "relative dir",
            ),
            (
                &format!(
                    "version = 1\n[[vault]]\nid = \"{id}\"\nname = \"a\"\ncipher_dir = \"/c\"\n\
                     [[vault]]\nid = \"{id}\"\nname = \"b\"\ncipher_dir = \"/d\"\n"
                ),
                "duplicate id",
            ),
        ] {
            fs::write(&path, text).unwrap();
            assert!(VaultRegistry::load(&path).is_err(), "{why}");
        }
    }
}
