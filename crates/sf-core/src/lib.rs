//! Shared building blocks for every Secure Folder component.
//!
//! This crate has no D-Bus, UI or async dependency so that each component
//! (session service, key service, front-ends) can depend on it cheaply.

pub mod id;
pub mod secret;

pub use id::{InvalidVaultId, VaultId};
pub use secret::SecretBytes;
