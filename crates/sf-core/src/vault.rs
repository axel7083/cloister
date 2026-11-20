use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::id::VaultId;

const MAX_NAME_CHARS: usize = 64;

/// A vault as stored in the registry. Always valid: construction and
/// deserialization go through [`Vault::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawVault")]
pub struct Vault {
    id: VaultId,
    name: String,
    cipher_dir: PathBuf,
}

impl Vault {
    /// Validates the name (trimmed, 1–64 chars, no control characters) and
    /// requires an absolute cipher directory.
    pub fn new(id: VaultId, name: &str, cipher_dir: PathBuf) -> Result<Self> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidName("empty"));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(Error::InvalidName("longer than 64 characters"));
        }
        if name.chars().any(char::is_control) {
            return Err(Error::InvalidName("contains control characters"));
        }
        if !cipher_dir.is_absolute() {
            return Err(Error::RelativeCipherDir);
        }
        Ok(Self {
            id,
            name: name.to_owned(),
            cipher_dir,
        })
    }

    pub fn id(&self) -> &VaultId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn cipher_dir(&self) -> &std::path::Path {
        &self.cipher_dir
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVault {
    id: VaultId,
    name: String,
    cipher_dir: PathBuf,
}

impl TryFrom<RawVault> for Vault {
    type Error = Error;

    fn try_from(raw: RawVault) -> Result<Self> {
        Vault::new(raw.id, &raw.name, raw.cipher_dir)
    }
}

/// Runtime state of a vault, owned by the session service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultState {
    Locked,
    Unlocked { mount_point: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> VaultId {
        VaultId::generate()
    }

    #[test]
    fn trims_and_accepts_unicode() {
        let v = Vault::new(id(), "  Coffre-fort 🔒 ", "/x".into()).unwrap();
        assert_eq!(v.name(), "Coffre-fort 🔒");
    }

    #[test]
    fn rejects_bad_names_and_relative_dirs() {
        assert!(matches!(Vault::new(id(), "  ", "/x".into()), Err(Error::InvalidName(_))));
        assert!(matches!(Vault::new(id(), "a\nb", "/x".into()), Err(Error::InvalidName(_))));
        assert!(matches!(
            Vault::new(id(), &"a".repeat(65), "/x".into()),
            Err(Error::InvalidName(_))
        ));
        assert!(matches!(Vault::new(id(), "ok", "x".into()), Err(Error::RelativeCipherDir)));
    }
}
