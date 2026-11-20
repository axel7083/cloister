//! The public D-Bus contract of Secure Folder.
//!
//! Front-ends (GVfs monitor, file manager extensions, CLI, app) depend on this
//! crate only. Every name, signature and error here is public API: change it
//! only in a backwards-compatible way, or bump the interface version (`…2`).
//!
//! Secrets (passphrases) are always passed as file descriptors (`h`) so they
//! never appear in message bodies seen by `dbus-monitor`.

use serde::{Deserialize, Serialize};
use zbus::{proxy, DBusError, Fd, Type};

/// Well-known name of the session service (`sf-session`).
pub const SESSION_BUS_NAME: &str = "io.github.axel7083.SecureFolder.Session";
/// Object path of the session service.
pub const SESSION_OBJECT_PATH: &str = "/io/github/axel7083/SecureFolder/Session";
/// Interface implemented by the session service.
pub const VAULTS_INTERFACE: &str = "io.github.axel7083.SecureFolder.Vaults1";

/// Value of [`VaultInfo::state`] for a locked vault.
pub const STATE_LOCKED: &str = "locked";
/// Value of [`VaultInfo::state`] for an unlocked (mounted) vault.
pub const STATE_UNLOCKED: &str = "unlocked";

/// Maximum passphrase size accepted through a file descriptor, in bytes.
pub const MAX_PASSPHRASE_BYTES: usize = 4096;

/// One vault as returned by `List`. Wire signature: `(ssss)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VaultInfo {
    pub id: String,
    pub name: String,
    /// [`STATE_LOCKED`] or [`STATE_UNLOCKED`].
    pub state: String,
    /// Empty while locked.
    pub mount_point: String,
}

impl VaultInfo {
    pub fn is_unlocked(&self) -> bool {
        self.state == STATE_UNLOCKED
    }
}

/// Errors returned by the session service. D-Bus names are
/// `io.github.axel7083.SecureFolder.Error.<Variant>`.
#[derive(Debug, DBusError)]
#[zbus(prefix = "io.github.axel7083.SecureFolder.Error")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    UnknownVault(String),
    InvalidArgument(String),
    WrongPassphrase(String),
    /// Files in the vault are in use; retry or lock with `force`.
    Busy(String),
    /// The vault cannot be unlocked because automatic locking is not active.
    AutoLockUnavailable(String),
    Failed(String),
}

impl From<sf_core::Error> for Error {
    fn from(e: sf_core::Error) -> Self {
        use sf_core::Error as E;
        let msg = e.to_string();
        match e {
            E::UnknownVault => Self::UnknownVault(msg),
            E::WrongPassphrase => Self::WrongPassphrase(msg),
            E::Busy => Self::Busy(msg),
            E::InvalidId(_)
            | E::InvalidName(_)
            | E::InvalidPassphrase(_)
            | E::RelativeCipherDir
            | E::DuplicateVault => Self::InvalidArgument(msg),
            _ => Self::Failed(msg),
        }
    }
}

/// Client proxy for `io.github.axel7083.SecureFolder.Vaults1`.
#[proxy(
    interface = "io.github.axel7083.SecureFolder.Vaults1",
    default_service = "io.github.axel7083.SecureFolder.Session",
    default_path = "/io/github/axel7083/SecureFolder/Session"
)]
pub trait Vaults1 {
    /// All known vaults and their state.
    async fn list(&self) -> Result<Vec<VaultInfo>, Error>;

    /// Creates a vault; the passphrase is read from `passphrase` (one trailing
    /// newline is ignored). Returns the new vault id.
    async fn create(&self, name: &str, passphrase: Fd<'_>) -> Result<String, Error>;

    /// Unlocks with the vault passphrase. Returns the mount point.
    async fn unlock_with_passphrase(&self, id: &str, passphrase: Fd<'_>) -> Result<String, Error>;

    /// Locks a vault. Without `force`, fails with `Busy` while files are open.
    async fn lock(&self, id: &str, force: bool) -> Result<(), Error>;

    /// Locks every vault, forcing busy ones.
    async fn lock_all(&self) -> Result<(), Error>;

    /// Forgets a locked vault. Its encrypted files are kept on disk.
    async fn remove(&self, id: &str) -> Result<(), Error>;

    /// A vault was locked or unlocked (`mount_point` is empty when locked).
    #[zbus(signal)]
    async fn state_changed(&self, id: &str, state: &str, mount_point: &str) -> zbus::Result<()>;

    /// A vault was added or removed: call `List` again.
    #[zbus(signal)]
    async fn vaults_changed(&self) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_info_signature_is_frozen() {
        assert_eq!(VaultInfo::SIGNATURE.to_string(), "(ssss)");
    }

    #[test]
    fn error_names_are_frozen() {
        use zbus::DBusError as _;
        assert_eq!(
            Error::WrongPassphrase(String::new()).name().as_str(),
            "io.github.axel7083.SecureFolder.Error.WrongPassphrase"
        );
        assert_eq!(
            Error::Busy(String::new()).name().as_str(),
            "io.github.axel7083.SecureFolder.Error.Busy"
        );
    }
}
