//! `io.github.axel7083.Cloister.KeyService1` implementation.

use std::fs::File;
use std::sync::Arc;
use std::time::Duration;

use cloister_core::secret::read_limited;
use cloister_core::{ExposeSecret, SecretBytes, VaultId};
use cloister_dbus::{Error, MAX_KEY_BYTES};
use zbus::message::Header;
use zbus::names::OwnedUniqueName;
use zbus::{interface, OwnedFd};

use crate::activity::Activity;
use crate::gate::{Action, Gate};
use crate::store::KeyStore;

const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub struct KeyService {
    store: Arc<KeyStore>,
    gate: Arc<dyn Gate>,
    activity: Activity,
    disk_encrypted: bool,
}

impl KeyService {
    pub fn new(
        store: KeyStore,
        gate: Arc<dyn Gate>,
        activity: Activity,
        disk_encrypted: bool,
    ) -> Self {
        Self {
            store: Arc::new(store),
            gate,
            activity,
            disk_encrypted,
        }
    }
}

fn sender(hdr: &Header<'_>) -> Option<OwnedUniqueName> {
    hdr.sender().map(|s| s.to_owned().into())
}

fn parse(id: &str) -> Result<VaultId, Error> {
    id.parse()
        .map_err(|_| Error::InvalidArgument("invalid vault id".into()))
}

fn io_err(e: std::io::Error) -> Error {
    tracing::error!("key store: {e}");
    Error::Failed("key store error".into())
}

async fn read_key(fd: OwnedFd) -> Result<SecretBytes, Error> {
    let file = File::from(std::os::fd::OwnedFd::from(fd));
    let task = tokio::task::spawn_blocking(move || read_limited(file, MAX_KEY_BYTES));
    match tokio::time::timeout(READ_TIMEOUT, task).await {
        Ok(Ok(Ok(k))) if !k.expose_secret().is_empty() => Ok(k),
        Ok(Ok(Ok(_))) => Err(Error::InvalidArgument("empty key".into())),
        Ok(Ok(Err(e))) => Err(Error::InvalidArgument(format!("cannot read key: {e}"))),
        Ok(Err(_)) => Err(Error::Failed("key reader panicked".into())),
        Err(_) => Err(Error::InvalidArgument("key fd was not closed".into())),
    }
}

/// Writes the key into the caller's pipe. Off the async thread, with a
/// timeout: a caller could hand us a full pipe that never drains.
async fn write_key(sink: OwnedFd, key: SecretBytes) -> Result<(), Error> {
    use std::io::Write;
    let mut file = File::from(std::os::fd::OwnedFd::from(sink));
    let task = tokio::task::spawn_blocking(move || file.write_all(key.expose_secret()));
    match tokio::time::timeout(READ_TIMEOUT, task).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(e))) => Err(Error::InvalidArgument(format!("cannot write key: {e}"))),
        Ok(Err(_)) => Err(Error::Failed("key writer panicked".into())),
        Err(_) => Err(Error::InvalidArgument("key sink does not drain".into())),
    }
}

#[interface(name = "io.github.axel7083.Cloister.KeyService1")]
impl KeyService {
    async fn enroll(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        id: String,
        key: OwnedFd,
    ) -> Result<(), Error> {
        self.activity.touch();
        let id = parse(&id)?;
        let uid = self.gate.caller_uid(sender(&hdr)).await?;
        let key = read_key(key).await?;
        self.gate.authorize(sender(&hdr), Action::Enroll).await?;
        self.store.put(uid, &id, &key).map_err(io_err)?;
        tracing::info!(uid, %id, "key enrolled");
        Ok(())
    }

    async fn release(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        id: String,
        sink: OwnedFd,
    ) -> Result<(), Error> {
        self.activity.touch();
        let id = parse(&id)?;
        let uid = self.gate.caller_uid(sender(&hdr)).await?;
        // Check first: never prompt for a key that does not exist.
        if !self.store.contains(uid, &id) {
            return Err(Error::NotEnrolled(
                "fingerprint unlock is not turned on for this vault".into(),
            ));
        }
        self.gate.authorize(sender(&hdr), Action::Unlock).await?;
        let key = self
            .store
            .get(uid, &id)
            .map_err(io_err)?
            .ok_or_else(|| Error::NotEnrolled("key removed meanwhile".into()))?;
        write_key(sink, key).await?;
        self.activity.touch();
        tracing::info!(uid, %id, "key released");
        Ok(())
    }

    async fn revoke(&self, #[zbus(header)] hdr: Header<'_>, id: String) -> Result<(), Error> {
        self.activity.touch();
        let id = parse(&id)?;
        let uid = self.gate.caller_uid(sender(&hdr)).await?;
        self.gate.authorize(sender(&hdr), Action::Revoke).await?;
        if self.store.remove(uid, &id).map_err(io_err)? {
            tracing::info!(uid, %id, "key revoked");
        }
        Ok(())
    }

    async fn is_enrolled(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        id: String,
    ) -> Result<bool, Error> {
        self.activity.touch();
        let id = parse(&id)?;
        let uid = self.gate.caller_uid(sender(&hdr)).await?;
        Ok(self.store.contains(uid, &id))
    }

    async fn system_disk_encrypted(&self) -> bool {
        self.activity.touch();
        self.disk_encrypted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex;

    use cloister_dbus::KeyService1Proxy;
    use futures_util::future::BoxFuture;
    use zbus::{connection, Guid};

    /// Fake gate: fixed uid, records actions, allows unless `deny` is set.
    #[derive(Default)]
    struct FakeGate {
        uid: Mutex<u32>,
        deny: Mutex<bool>,
        asked: Mutex<Vec<Action>>,
    }

    impl Gate for FakeGate {
        fn caller_uid(&self, _: Option<OwnedUniqueName>) -> BoxFuture<'_, Result<u32, Error>> {
            Box::pin(async move { Ok(*self.uid.lock().unwrap()) })
        }
        fn authorize(
            &self,
            _: Option<OwnedUniqueName>,
            action: Action,
        ) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async move {
                self.asked.lock().unwrap().push(action);
                if *self.deny.lock().unwrap() {
                    Err(Error::Denied("no".into()))
                } else {
                    Ok(())
                }
            })
        }
    }

    async fn setup() -> (
        tempfile::TempDir,
        Arc<FakeGate>,
        KeyService1Proxy<'static>,
        (zbus::Connection, zbus::Connection),
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let gate = Arc::new(FakeGate {
            uid: Mutex::new(1000),
            ..Default::default()
        });
        let service = KeyService::new(
            KeyStore::new(tmp.path()),
            gate.clone(),
            Activity::new(),
            true,
        );
        let (a, b) = UnixStream::pair().unwrap();
        let server = connection::Builder::unix_stream(a)
            .server(Guid::generate())
            .p2p()
            .serve_at(cloister_dbus::KEYS_OBJECT_PATH, service);
        let client = connection::Builder::unix_stream(b).p2p();
        let (server, client) = tokio::try_join!(server.build(), client.build()).unwrap();
        let proxy = KeyService1Proxy::new(&client).await.unwrap();
        (tmp, gate, proxy, (server, client))
    }

    fn kind(e: Error) -> String {
        e.kind().unwrap_or("?").to_owned()
    }

    /// Calls Release with a fresh pipe and returns what was written into it.
    async fn release(keys: &KeyService1Proxy<'_>, id: &str) -> Result<Vec<u8>, Error> {
        use std::io::Read;
        let (mut r, w) = std::io::pipe().unwrap();
        keys.release(id, zbus::Fd::from(std::os::fd::OwnedFd::from(w)))
            .await?;
        let mut v = Vec::new();
        r.read_to_end(&mut v).unwrap();
        Ok(v)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enroll_release_revoke() {
        let (_tmp, gate, keys, _c) = setup().await;
        let id = VaultId::generate().to_string();

        assert_eq!(kind(release(&keys, &id).await.unwrap_err()), "NotEnrolled");
        assert!(
            gate.asked.lock().unwrap().is_empty(),
            "no prompt for a missing key"
        );

        keys.enroll(&id, cloister_dbus::secret_fd(b"master-key").unwrap())
            .await
            .unwrap();
        assert!(keys.is_enrolled(&id).await.unwrap());
        assert_eq!(release(&keys, &id).await.unwrap(), b"master-key");

        *gate.uid.lock().unwrap() = 1001;
        assert!(!keys.is_enrolled(&id).await.unwrap(), "keys are per uid");
        assert_eq!(kind(release(&keys, &id).await.unwrap_err()), "NotEnrolled");
        *gate.uid.lock().unwrap() = 1000;

        *gate.deny.lock().unwrap() = true;
        assert_eq!(kind(release(&keys, &id).await.unwrap_err()), "Denied");
        assert_eq!(kind(keys.revoke(&id).await.unwrap_err()), "Denied");
        *gate.deny.lock().unwrap() = false;

        keys.revoke(&id).await.unwrap();
        assert!(!keys.is_enrolled(&id).await.unwrap());
        assert_eq!(
            *gate.asked.lock().unwrap(),
            [
                Action::Enroll,
                Action::Unlock,
                Action::Unlock,
                Action::Revoke,
                Action::Revoke
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_bad_input_before_prompting() {
        let (_tmp, gate, keys, _c) = setup().await;
        assert_eq!(
            kind(
                keys.enroll("../x", cloister_dbus::secret_fd(b"k").unwrap())
                    .await
                    .unwrap_err()
            ),
            "InvalidArgument"
        );
        let id = VaultId::generate().to_string();
        assert_eq!(
            kind(
                keys.enroll(&id, cloister_dbus::secret_fd(b"").unwrap())
                    .await
                    .unwrap_err()
            ),
            "InvalidArgument"
        );
        assert!(gate.asked.lock().unwrap().is_empty());
        assert!(keys.system_disk_encrypted().await.unwrap());
    }
}

#[cfg(test)]
mod bus_tests {
    use super::tests_support::AllowAll;
    use super::*;

    /// Release through a real bus broker (session bus), like production.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs a session bus"]
    async fn release_over_real_bus() {
        let tmp = tempfile::tempdir().unwrap();
        let name = format!("io.github.axel7083.Cloister.Test{}", std::process::id());
        let service = KeyService::new(
            KeyStore::new(tmp.path()),
            Arc::new(AllowAll),
            Activity::new(),
            true,
        );
        let _server = zbus::connection::Builder::session()
            .serve_at(cloister_dbus::KEYS_OBJECT_PATH, service)
            .name(name.as_str())
            .build()
            .await
            .unwrap();
        let client = zbus::Connection::session().await.unwrap();
        let keys = cloister_dbus::KeyService1Proxy::builder(&client)
            .destination(name.as_str())
            .build()
            .await
            .unwrap();
        let id = VaultId::generate().to_string();
        keys.enroll(&id, cloister_dbus::secret_fd(b"k3y").unwrap())
            .await
            .unwrap();
        let (mut r, w) = std::io::pipe().unwrap();
        keys.release(&id, zbus::Fd::from(std::os::fd::OwnedFd::from(w)))
            .await
            .unwrap();
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut r, &mut v).unwrap();
        assert_eq!(v, b"k3y");
    }
}

#[cfg(test)]
mod tests_support {
    use super::*;
    use futures_util::future::BoxFuture;

    pub struct AllowAll;
    impl Gate for AllowAll {
        fn caller_uid(&self, _: Option<OwnedUniqueName>) -> BoxFuture<'_, Result<u32, Error>> {
            Box::pin(async { Ok(1000) })
        }
        fn authorize(
            &self,
            _: Option<OwnedUniqueName>,
            _: Action,
        ) -> BoxFuture<'_, Result<(), Error>> {
            Box::pin(async { Ok(()) })
        }
    }
}
