//! Client side of the key service (system bus). The master key only travels
//! between sf-keyd and this process; front-ends never see it.

use futures_util::future::BoxFuture;
use sf_core::{ExposeSecret, SecretBytes, VaultId};
use sf_dbus::{secret_fd, Error, KeyService1Proxy, MAX_KEY_BYTES};
use tokio::sync::OnceCell;
use zbus::Connection;

use crate::passfd::read_secret;

pub trait KeyBackend: Send + Sync + 'static {
    fn enroll(&self, id: VaultId, key: SecretBytes) -> BoxFuture<'_, Result<(), Error>>;
    fn release(&self, id: VaultId) -> BoxFuture<'_, Result<SecretBytes, Error>>;
    fn revoke(&self, id: VaultId) -> BoxFuture<'_, Result<(), Error>>;
    fn is_enrolled(&self, id: VaultId) -> BoxFuture<'_, Result<bool, Error>>;
    fn system_disk_encrypted(&self) -> BoxFuture<'_, Result<bool, Error>>;
}

/// Production backend: sf-keyd on the system bus, connected on first use.
#[derive(Default)]
pub struct SystemKeyService {
    proxy: OnceCell<KeyService1Proxy<'static>>,
}

impl SystemKeyService {
    async fn proxy(&self) -> Result<&KeyService1Proxy<'static>, Error> {
        self.proxy
            .get_or_try_init(|| async {
                let conn = Connection::system().await?;
                Ok::<_, zbus::Error>(KeyService1Proxy::new(&conn).await?)
            })
            .await
            .map_err(unavailable)
    }
}

fn unavailable(e: zbus::Error) -> Error {
    Error::KeyServiceUnavailable(format!("key service unavailable: {e}"))
}

/// The service not being installed shows up as a bus error, not ours.
fn map(e: Error) -> Error {
    match e {
        Error::ZBus(zbus::Error::MethodError(ref name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" =>
        {
            Error::KeyServiceUnavailable("fingerprint unlock is not installed".into())
        }
        Error::ZBus(e @ (zbus::Error::InputOutput(_) | zbus::Error::Address(_))) => unavailable(e),
        e => e,
    }
}

impl KeyBackend for SystemKeyService {
    fn enroll(&self, id: VaultId, key: SecretBytes) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let fd = secret_fd(key.expose_secret()).map_err(|e| Error::Failed(e.to_string()))?;
            drop(key);
            self.proxy().await?.enroll(&id.to_string(), fd).await.map_err(map)
        })
    }

    fn release(&self, id: VaultId) -> BoxFuture<'_, Result<SecretBytes, Error>> {
        Box::pin(async move {
            // We pass the write end; the service never sends us an fd.
            let (reader, writer) = std::io::pipe().map_err(|e| Error::Failed(e.to_string()))?;
            let sink = zbus::Fd::from(std::os::fd::OwnedFd::from(writer));
            self.proxy().await?.release(&id.to_string(), sink).await.map_err(map)?;
            // The writer copy we sent is closed by now: read until EOF.
            read_secret(zbus::OwnedFd::from(std::os::fd::OwnedFd::from(reader)), MAX_KEY_BYTES).await
        })
    }

    fn revoke(&self, id: VaultId) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.proxy().await?.revoke(&id.to_string()).await.map_err(map) })
    }

    fn is_enrolled(&self, id: VaultId) -> BoxFuture<'_, Result<bool, Error>> {
        Box::pin(async move { self.proxy().await?.is_enrolled(&id.to_string()).await.map_err(map) })
    }

    fn system_disk_encrypted(&self) -> BoxFuture<'_, Result<bool, Error>> {
        Box::pin(async move { self.proxy().await?.system_disk_encrypted().await.map_err(map) })
    }
}

/// In-memory backend for tests.
#[cfg(test)]
pub mod fake {
    use super::*;
    use sf_core::secret_from_vec;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeKeys {
        pub keys: Mutex<HashMap<VaultId, Vec<u8>>>,
        pub deny: Mutex<bool>,
        pub unavailable: Mutex<bool>,
    }

    impl FakeKeys {
        fn gate(&self) -> Result<(), Error> {
            if *self.unavailable.lock().unwrap() {
                return Err(Error::KeyServiceUnavailable("not installed".into()));
            }
            if *self.deny.lock().unwrap() {
                return Err(Error::Denied("cancelled".into()));
            }
            Ok(())
        }
    }

    impl KeyBackend for FakeKeys {
        fn enroll(&self, id: VaultId, key: SecretBytes) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move {
                self.gate()?;
                self.keys.lock().unwrap().insert(id, key.expose_secret().to_vec());
                Ok(())
            })
        }
        fn release(&self, id: VaultId) -> BoxFuture<'_, Result<SecretBytes, Error>> {
            Box::pin(async move {
                let key = self.keys.lock().unwrap().get(&id).cloned();
                let key = key.ok_or_else(|| Error::NotEnrolled("no key".into()))?;
                self.gate()?;
                Ok(secret_from_vec(key))
            })
        }
        fn revoke(&self, id: VaultId) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move {
                self.gate()?;
                self.keys.lock().unwrap().remove(&id);
                Ok(())
            })
        }
        fn is_enrolled(&self, id: VaultId) -> BoxFuture<'_, Result<bool, Error>> {
            Box::pin(async move {
                if *self.unavailable.lock().unwrap() {
                    return Err(Error::KeyServiceUnavailable("not installed".into()));
                }
                Ok(self.keys.lock().unwrap().contains_key(&id))
            })
        }
        fn system_disk_encrypted(&self) -> BoxFuture<'_, Result<bool, Error>> {
            Box::pin(async move { Ok(true) })
        }
    }
}
