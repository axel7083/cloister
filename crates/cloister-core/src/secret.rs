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

/// Reads `reader` to EOF, refusing more than `max` bytes. One trailing `\n`
/// is dropped. Intermediate buffers are zeroized.
pub fn read_limited(reader: impl std::io::Read, max: usize) -> std::io::Result<SecretBytes> {
    use std::io::{Error, ErrorKind, Read};
    // Full capacity up front: a reallocation would free an unwiped copy.
    let mut buf = zeroize::Zeroizing::new(Vec::with_capacity(max + 1));
    reader.take(max as u64 + 1).read_to_end(&mut buf)?;
    if buf.len() > max {
        return Err(Error::new(ErrorKind::InvalidData, "secret too long"));
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    Ok(secret_from_vec(std::mem::take(&mut *buf)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_limited_strips_one_newline_and_caps_size() {
        assert_eq!(read_limited(&b"ab\n\n"[..], 8).unwrap().expose_secret(), b"ab\n");
        assert!(read_limited(&b"123456789"[..], 8).is_err());
        assert_eq!(read_limited(&b"12345678"[..], 8).unwrap().expose_secret(), b"12345678");
    }

    #[test]
    fn debug_does_not_leak() {
        let s = secret_from_vec(b"hunter2".to_vec());
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.expose_secret(), b"hunter2");
    }
}
