//! Reading a passphrase from a file descriptor received over D-Bus.

use std::fs::File;
use std::io::Read;
use std::time::Duration;

use sf_core::{secret_from_vec, SecretBytes};
use sf_dbus::{Error, MAX_PASSPHRASE_BYTES};
use zbus::OwnedFd;
use zeroize::Zeroizing;

/// A client that never closes its end must not block us forever.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Reads at most [`MAX_PASSPHRASE_BYTES`] until EOF; one trailing `\n` is dropped.
pub async fn read_passphrase(fd: OwnedFd) -> Result<SecretBytes, Error> {
    let file = File::from(std::os::fd::OwnedFd::from(fd));
    let task = tokio::task::spawn_blocking(move || read_limited(file));
    match tokio::time::timeout(READ_TIMEOUT, task).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err(Error::Failed("passphrase reader panicked".into())),
        Err(_) => Err(Error::InvalidArgument("passphrase fd was not closed".into())),
    }
}

fn read_limited(file: File) -> Result<SecretBytes, Error> {
    let mut buf = Zeroizing::new(Vec::with_capacity(256));
    file.take(MAX_PASSPHRASE_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| Error::InvalidArgument(format!("cannot read passphrase: {e}")))?;
    if buf.len() > MAX_PASSPHRASE_BYTES {
        return Err(Error::InvalidArgument("passphrase too long".into()));
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    Ok(secret_from_vec(std::mem::take(&mut *buf)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::ExposeSecret;
    use std::io::Write;

    fn pipe_with(data: &[u8]) -> OwnedFd {
        let (r, mut w) = std::io::pipe().unwrap();
        w.write_all(data).unwrap();
        drop(w);
        OwnedFd::from(std::os::fd::OwnedFd::from(r))
    }

    #[tokio::test]
    async fn reads_and_strips_one_newline() {
        let p = read_passphrase(pipe_with(b"secret\n")).await.unwrap();
        assert_eq!(p.expose_secret(), b"secret");
        let p = read_passphrase(pipe_with(b"a b\n\n")).await.unwrap();
        assert_eq!(p.expose_secret(), b"a b\n");
    }

    #[tokio::test]
    async fn rejects_oversized() {
        let big = vec![b'x'; MAX_PASSPHRASE_BYTES + 1];
        assert!(matches!(read_passphrase(pipe_with(&big)).await, Err(Error::InvalidArgument(_))));
    }
}
