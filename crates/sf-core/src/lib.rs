//! Shared building blocks for every Secure Folder component.
//!
//! This crate has no D-Bus, UI or async dependency so that each component
//! (session service, key service, front-ends) can depend on it cheaply.

pub mod error;
pub mod gocryptfs;
pub mod id;
pub mod mountinfo;
pub mod paths;
pub mod registry;
pub mod secret;
pub mod vault;

pub use error::{Error, Result};
pub use gocryptfs::{Gocryptfs, MountHandle, Unlock};
pub use id::{InvalidVaultId, VaultId};
pub use paths::Paths;
pub use registry::VaultRegistry;
pub use secret::{secret_from_vec, ExposeSecret, SecretBytes};
pub use vault::{Vault, VaultState};
