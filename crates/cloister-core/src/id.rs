use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifier of a vault. It is the only id type accepted by every API.
///
/// Parsing is strict (hyphenated lowercase UUID) so that an id coming from
/// D-Bus can safely be used as a file name: no `/`, `..` or other surprises.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VaultId(Uuid);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid vault id")]
pub struct InvalidVaultId;

impl VaultId {
    /// Creates a new random id.
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

impl FromStr for VaultId {
    type Err = InvalidVaultId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::try_parse(s).map_err(|_| InvalidVaultId)?;
        // Reject every alternative spelling (braced, urn:, simple, uppercase):
        // one vault must map to exactly one string.
        if uuid.hyphenated().to_string() != s {
            return Err(InvalidVaultId);
        }
        Ok(Self(uuid))
    }
}

impl TryFrom<String> for VaultId {
    type Error = InvalidVaultId;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<VaultId> for String {
    fn from(id: VaultId) -> Self {
        id.to_string()
    }
}

impl fmt::Display for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl fmt::Debug for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VaultId({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let id = VaultId::generate();
        assert_eq!(id.to_string().parse::<VaultId>(), Ok(id));
    }

    #[test]
    fn rejects_non_canonical_and_hostile_input() {
        for bad in [
            "",
            "..",
            "../etc/passwd",
            "a/b",
            "67e55044-10b1-426f-9247-bb680e5fe0c8/..",
            "67E55044-10B1-426F-9247-BB680E5FE0C8",
            "{67e55044-10b1-426f-9247-bb680e5fe0c8}",
            "urn:uuid:67e55044-10b1-426f-9247-bb680e5fe0c8",
            "67e5504410b1426f9247bb680e5fe0c8",
        ] {
            assert_eq!(bad.parse::<VaultId>(), Err(InvalidVaultId), "{bad:?}");
        }
    }

    #[test]
    fn accepts_canonical() {
        assert!("67e55044-10b1-426f-9247-bb680e5fe0c8"
            .parse::<VaultId>()
            .is_ok());
    }
}
