//! Secret material (passphrases, master keys).
//!
//! Wrapping in [`SecretBytes`] gives zeroize-on-drop and hides the content
//! from `Debug`; reading it requires an explicit `expose_secret()` call,
//! which keeps every access easy to audit with `grep`.

pub use secrecy::ExposeSecret;

/// Heap-allocated secret bytes, zeroized on drop, never printed.
pub type SecretBytes = secrecy::SecretBox<[u8]>;

/// Takes ownership of `bytes` as a secret.
pub fn secret_from_vec(bytes: Vec<u8>) -> SecretBytes {
    SecretBytes::from(bytes.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak() {
        let s = secret_from_vec(b"hunter2".to_vec());
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.expose_secret(), b"hunter2");
    }
}
