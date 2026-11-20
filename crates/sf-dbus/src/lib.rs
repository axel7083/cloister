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

/// Well-known name of the application (`secure-folder`), D-Bus activatable.
pub const APP_BUS_NAME: &str = "io.github.axel7083.SecureFolder";
/// Object path of the application (derived from the application id by GApplication).
pub const APP_OBJECT_PATH: &str = "/io/github/axel7083/SecureFolder";
/// Interface the application implements to prompt the user.
pub const PROMPT_INTERFACE: &str = "io.github.axel7083.SecureFolder.Prompt1";

/// Well-known name of the key service (`sf-keyd`), on the **system** bus.
pub const KEYS_BUS_NAME: &str = "io.github.axel7083.SecureFolder.KeyService";
/// Object path of the key service.
pub const KEYS_OBJECT_PATH: &str = "/io/github/axel7083/SecureFolder/KeyService";
/// Interface implemented by the key service.
pub const KEYS_INTERFACE: &str = "io.github.axel7083.SecureFolder.KeyService1";

/// polkit action checked before releasing a stored key (every time).
pub const ACTION_UNLOCK: &str = "io.github.axel7083.SecureFolder.unlock";
/// polkit action checked before storing a key.
pub const ACTION_ENROLL: &str = "io.github.axel7083.SecureFolder.enroll";
/// polkit action checked before deleting a stored key.
pub const ACTION_REVOKE: &str = "io.github.axel7083.SecureFolder.revoke";

/// Maximum size of a stored master key, in bytes.
pub const MAX_KEY_BYTES: usize = 512;

/// Prefix of every Secure Folder D-Bus error name.
pub const ERROR_PREFIX: &str = "io.github.axel7083.SecureFolder.Error";

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

/// Fingerprint unlock state of one vault. Wire signature: `(bbb)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FingerprintStatus {
    /// The key service is installed and reachable.
    pub available: bool,
    /// A key for this vault is stored, so fingerprint unlock is on.
    pub enrolled: bool,
    /// The disk holding stored keys is encrypted (LUKS). When false, turning
    /// fingerprint unlock on weakens the vault: warn the user.
    pub system_disk_encrypted: bool,
}

/// Errors returned by the session service and the key service. D-Bus names are
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
    /// Fingerprint unlock is not turned on for this vault.
    NotEnrolled(String),
    /// Authentication (polkit) failed or was cancelled.
    Denied(String),
    /// The key service is not installed or not reachable.
    KeyServiceUnavailable(String),
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

impl Error {
    /// Error name without the [`ERROR_PREFIX`] (e.g. `"WrongPassphrase"`),
    /// also for errors that came back undecoded from the bus.
    pub fn kind(&self) -> Option<&str> {
        match self {
            Self::UnknownVault(_) => Some("UnknownVault"),
            Self::InvalidArgument(_) => Some("InvalidArgument"),
            Self::WrongPassphrase(_) => Some("WrongPassphrase"),
            Self::Busy(_) => Some("Busy"),
            Self::AutoLockUnavailable(_) => Some("AutoLockUnavailable"),
            Self::NotEnrolled(_) => Some("NotEnrolled"),
            Self::Denied(_) => Some("Denied"),
            Self::KeyServiceUnavailable(_) => Some("KeyServiceUnavailable"),
            Self::Failed(_) => Some("Failed"),
            Self::ZBus(zbus::Error::MethodError(name, _, _)) => name
                .as_str()
                .strip_prefix(ERROR_PREFIX)
                .and_then(|n| n.strip_prefix('.')),
            Self::ZBus(_) => None,
        }
    }

    /// Human-readable message, suitable for a CLI or a dialog.
    pub fn message(&self) -> String {
        match self {
            Self::ZBus(zbus::Error::MethodError(name, msg, _)) => {
                msg.clone().unwrap_or_else(|| name.as_str().to_owned())
            }
            Self::ZBus(e) => e.to_string(),
            Self::UnknownVault(m)
            | Self::InvalidArgument(m)
            | Self::WrongPassphrase(m)
            | Self::Busy(m)
            | Self::AutoLockUnavailable(m)
            | Self::NotEnrolled(m)
            | Self::Denied(m)
            | Self::KeyServiceUnavailable(m)
            | Self::Failed(m) => m.clone(),
        }
    }
}

/// Puts `secret` in a pipe and returns its read end, ready to be sent as an
/// `h` argument. The secret never enters a D-Bus message body.
pub fn secret_fd(secret: &[u8]) -> std::io::Result<Fd<'static>> {
    use std::io::Write;
    if secret.len() > MAX_PASSPHRASE_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "passphrase too long"));
    }
    let (reader, mut writer) = std::io::pipe()?;
    // Fits in the pipe buffer (64 KiB), so this cannot block.
    writer.write_all(secret)?;
    drop(writer);
    Ok(Fd::from(std::os::fd::OwnedFd::from(reader)))
}

/// Client proxy for `io.github.axel7083.SecureFolder.Prompt1`, served by the
/// application. Calls wait for a human: do not set a short timeout.
#[proxy(
    interface = "io.github.axel7083.SecureFolder.Prompt1",
    default_service = "io.github.axel7083.SecureFolder",
    default_path = "/io/github/axel7083/SecureFolder"
)]
pub trait Prompt1 {
    /// Asks the user to unlock the vault. Returns `true` once it is unlocked,
    /// `false` if the user cancelled.
    async fn unlock(&self, id: &str) -> Result<bool, Error>;
}

/// Client proxy for the key service. Only `sf-session` should use it: keys
/// must not reach front-ends. The caller's uid (from the bus) scopes every key.
#[proxy(
    interface = "io.github.axel7083.SecureFolder.KeyService1",
    default_service = "io.github.axel7083.SecureFolder.KeyService",
    default_path = "/io/github/axel7083/SecureFolder/KeyService"
)]
pub trait KeyService1 {
    /// Stores `key` (read from the fd) for vault `id`. polkit: [`ACTION_ENROLL`].
    async fn enroll(&self, id: &str, key: Fd<'_>) -> Result<(), Error>;
    /// Writes the key into `sink` (the write end of the caller's pipe), then
    /// closes it. polkit: [`ACTION_UNLOCK`], every call. The service never
    /// sends file descriptors, it only receives them.
    async fn release(&self, id: &str, sink: Fd<'_>) -> Result<(), Error>;
    /// Deletes the key. polkit: [`ACTION_REVOKE`].
    async fn revoke(&self, id: &str) -> Result<(), Error>;
    async fn is_enrolled(&self, id: &str) -> Result<bool, Error>;
    /// Whether the disk holding the keys is encrypted.
    async fn system_disk_encrypted(&self) -> Result<bool, Error>;
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

    /// Unlocks with the key stored in the key service, after polkit
    /// authentication (fingerprint or login password). Returns the mount point.
    /// Errors: `NotEnrolled`, `Denied`, `KeyServiceUnavailable`.
    async fn unlock_with_key_service(&self, id: &str) -> Result<String, Error>;

    /// Turns fingerprint unlock on: checks the passphrase, then stores the
    /// vault master key in the key service (polkit authentication).
    async fn enroll(&self, id: &str, passphrase: Fd<'_>) -> Result<(), Error>;

    /// Turns fingerprint unlock off (deletes the stored key).
    async fn unenroll(&self, id: &str) -> Result<(), Error>;

    async fn get_fingerprint_status(&self, id: &str) -> Result<FingerprintStatus, Error>;

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
        assert_eq!(FingerprintStatus::SIGNATURE.to_string(), "(bbb)");
    }

    #[test]
    fn kind_works_for_raw_method_errors() {
        let raw = Error::ZBus(zbus::Error::MethodError(
            "io.github.axel7083.SecureFolder.Error.Busy".try_into().unwrap(),
            Some("in use".into()),
            zbus::message::Message::method_call("/", "X").build(&()).unwrap(),
        ));
        assert_eq!(raw.kind(), Some("Busy"));
        assert_eq!(raw.message(), "in use");
    }

    #[test]
    fn secret_fd_round_trip() {
        use std::io::Read;
        let fd = secret_fd(b"pw").unwrap();
        let mut f = std::fs::File::from(std::os::fd::OwnedFd::from(zbus::OwnedFd::from(fd)));
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!(s, "pw");
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
