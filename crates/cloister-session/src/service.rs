//! `io.github.axel7083.Cloister.Vaults1` implementation.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cloister_core::{
    ExposeSecret, Gocryptfs, Paths, SecretBytes, Unlock, Vault, VaultId, VaultRegistry,
};
use cloister_dbus::{Error, FingerprintStatus, VaultInfo, STATE_LOCKED, STATE_UNLOCKED};
use zbus::object_server::SignalEmitter;
use zbus::{interface, OwnedFd};

use crate::keys::KeyBackend;
use crate::mounts::MountManager;
use crate::passfd::read_passphrase;

/// Baseline policy for new vault passphrases (front-ends may ask for more).
pub const MIN_PASSPHRASE_BYTES: usize = 8;

/// Vault cryptography that is not mounting: creation and master key access.
pub trait VaultCrypto: Send + Sync + 'static {
    fn init(&self, cipher_dir: &Path, passphrase: &SecretBytes) -> cloister_core::Result<()>;
    /// Fails with `WrongPassphrase` when the passphrase is wrong.
    fn dump_master_key(
        &self,
        cipher_dir: &Path,
        passphrase: &SecretBytes,
    ) -> cloister_core::Result<SecretBytes>;
}

impl VaultCrypto for Gocryptfs {
    fn init(&self, cipher_dir: &Path, passphrase: &SecretBytes) -> cloister_core::Result<()> {
        Gocryptfs::init(self, cipher_dir, passphrase)
    }
    fn dump_master_key(
        &self,
        cipher_dir: &Path,
        passphrase: &SecretBytes,
    ) -> cloister_core::Result<SecretBytes> {
        Gocryptfs::dump_master_key(self, cipher_dir, passphrase)
    }
}

/// Last time a client called us; drives the idle exit.
#[derive(Clone)]
pub struct Activity(Arc<Mutex<Instant>>);

impl Activity {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    pub fn touch(&self) {
        *self.0.lock().expect("activity poisoned") = Instant::now();
    }
    pub fn idle_for(&self) -> Duration {
        self.0.lock().expect("activity poisoned").elapsed()
    }
}

pub struct VaultsService {
    registry: Arc<Mutex<VaultRegistry>>,
    paths: Paths,
    crypto: Arc<dyn VaultCrypto>,
    keys: Arc<dyn KeyBackend>,
    mounts: MountManager,
    autolock_ready: Arc<AtomicBool>,
    activity: Activity,
}

impl VaultsService {
    pub fn new(
        registry: VaultRegistry,
        paths: Paths,
        crypto: Arc<dyn VaultCrypto>,
        keys: Arc<dyn KeyBackend>,
        mounts: MountManager,
        autolock_ready: Arc<AtomicBool>,
        activity: Activity,
    ) -> Self {
        Self {
            registry: Arc::new(Mutex::new(registry)),
            paths,
            crypto,
            keys,
            mounts,
            autolock_ready,
            activity,
        }
    }

    fn vault(&self, id: &str) -> Result<Vault, Error> {
        let id: VaultId = id.parse().map_err(cloister_core::Error::from)?;
        self.registry()
            .get(&id)
            .cloned()
            .ok_or(Error::UnknownVault(format!("no vault {id}")))
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, VaultRegistry> {
        self.registry.lock().expect("registry poisoned")
    }

    fn require_autolock(&self) -> Result<(), Error> {
        if self.autolock_ready.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::AutoLockUnavailable(
                "refusing to unlock: automatic locking is not active".into(),
            ))
        }
    }
}

#[interface(name = "io.github.axel7083.Cloister.Vaults1")]
impl VaultsService {
    async fn list(&self) -> Result<Vec<VaultInfo>, Error> {
        self.activity.touch();
        let mounted = self.mounts.mounted();
        Ok(self
            .registry()
            .list()
            .iter()
            .map(|v| {
                let mp = mounted.get(v.id());
                VaultInfo {
                    id: v.id().to_string(),
                    name: v.name().to_owned(),
                    state: if mp.is_some() {
                        STATE_UNLOCKED
                    } else {
                        STATE_LOCKED
                    }
                    .to_owned(),
                    mount_point: mp.map(|p| p.display().to_string()).unwrap_or_default(),
                }
            })
            .collect())
    }

    async fn create(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        name: String,
        passphrase: OwnedFd,
    ) -> Result<String, Error> {
        self.activity.touch();
        let passphrase = read_passphrase(passphrase).await?;
        if passphrase.expose_secret().len() < MIN_PASSPHRASE_BYTES {
            return Err(Error::InvalidArgument(format!(
                "passphrase must be at least {MIN_PASSPHRASE_BYTES} characters"
            )));
        }
        let id = VaultId::generate();
        let cipher_dir = self.paths.default_cipher_dir(&id);
        let vault = Vault::new(id, &name, cipher_dir.clone())?;

        let crypto = self.crypto.clone();
        let dir = cipher_dir.clone();
        tokio::task::spawn_blocking(move || crypto.init(&dir, &passphrase))
            .await
            .map_err(|_| Error::Failed("vault creation panicked".into()))??;

        if let Err(e) = self.registry().add(vault) {
            // Only remove what we just created ourselves.
            let _ = std::fs::remove_dir_all(&cipher_dir);
            return Err(e.into());
        }
        tracing::info!(%id, "vault created");
        emitter.vaults_changed().await?;
        Ok(id.to_string())
    }

    async fn unlock_with_passphrase(
        &self,
        id: String,
        passphrase: OwnedFd,
    ) -> Result<String, Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        let passphrase = read_passphrase(passphrase).await?;
        self.require_autolock()?;
        let at = self
            .mounts
            .mount(vault, Unlock::Passphrase(passphrase))
            .await?;
        self.activity.touch();
        Ok(at.display().to_string())
    }

    async fn unlock_with_key_service(&self, id: String) -> Result<String, Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        self.require_autolock()?;
        if let Some(at) = self.mounts.mounted().get(vault.id()) {
            return Ok(at.display().to_string()); // no prompt when already unlocked
        }
        let key = self.keys.release(*vault.id()).await?;
        let at = self.mounts.mount(vault, Unlock::MasterKey(key)).await?;
        self.activity.touch();
        Ok(at.display().to_string())
    }

    async fn enroll(&self, id: String, passphrase: OwnedFd) -> Result<(), Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        let passphrase = read_passphrase(passphrase).await?;
        let crypto = self.crypto.clone();
        let dir = vault.cipher_dir().to_owned();
        let key = tokio::task::spawn_blocking(move || crypto.dump_master_key(&dir, &passphrase))
            .await
            .map_err(|_| Error::Failed("key extraction panicked".into()))??;
        self.keys.enroll(*vault.id(), key).await?;
        self.activity.touch();
        tracing::info!(id = %vault.id(), "fingerprint unlock turned on");
        Ok(())
    }

    async fn unenroll(&self, id: String) -> Result<(), Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        self.keys.revoke(*vault.id()).await?;
        tracing::info!(id = %vault.id(), "fingerprint unlock turned off");
        Ok(())
    }

    async fn get_fingerprint_status(&self, id: String) -> Result<FingerprintStatus, Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        match self.keys.is_enrolled(*vault.id()).await {
            Ok(enrolled) => Ok(FingerprintStatus {
                available: true,
                enrolled,
                system_disk_encrypted: self.keys.system_disk_encrypted().await.unwrap_or(false),
            }),
            Err(Error::KeyServiceUnavailable(_)) => Ok(FingerprintStatus {
                available: false,
                enrolled: false,
                system_disk_encrypted: false,
            }),
            Err(e) => Err(e),
        }
    }

    async fn lock(&self, id: String, force: bool) -> Result<(), Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        Ok(self.mounts.unmount(*vault.id(), force).await?)
    }

    async fn lock_all(&self) -> Result<(), Error> {
        self.activity.touch();
        self.mounts.unmount_all().await;
        Ok(())
    }

    async fn remove(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        id: String,
    ) -> Result<(), Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        if self.mounts.mounted().contains_key(vault.id()) {
            return Err(Error::Busy("lock the vault before removing it".into()));
        }
        self.registry().remove(vault.id())?;
        // Best effort: a stale key is useless without the registry entry,
        // but should not linger.
        if let Ok(true) = self.keys.is_enrolled(*vault.id()).await {
            if let Err(e) = self.keys.revoke(*vault.id()).await {
                tracing::warn!(id = %vault.id(), "stored key not deleted: {}", e.message());
            }
        }
        tracing::info!(id = %vault.id(), cipher_dir = %vault.cipher_dir().display(), "vault forgotten, files kept");
        emitter.vaults_changed().await?;
        Ok(())
    }

    #[zbus(signal)]
    pub async fn state_changed(
        emitter: &SignalEmitter<'_>,
        id: &str,
        state: &str,
        mount_point: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn vaults_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    use cloister_dbus::Vaults1Proxy;
    use tokio::sync::mpsc;
    use zbus::{connection, Fd, Guid};

    use crate::keys::fake::FakeKeys;
    use crate::mounts::fake::Fake;

    struct FakeCrypto;
    impl VaultCrypto for FakeCrypto {
        fn init(&self, dir: &Path, _: &SecretBytes) -> cloister_core::Result<()> {
            Ok(std::fs::create_dir_all(dir)?)
        }
        fn dump_master_key(&self, _: &Path, p: &SecretBytes) -> cloister_core::Result<SecretBytes> {
            match p.expose_secret() {
                b"good" => Ok(cloister_core::secret_from_vec(b"MASTER".to_vec())),
                _ => Err(cloister_core::Error::WrongPassphrase),
            }
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        _server: zbus::Connection,
        _client: zbus::Connection,
        proxy: Vaults1Proxy<'static>,
        ready: Arc<AtomicBool>,
        keys: Arc<FakeKeys>,
    }

    async fn harness() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let p = |s: &str| tmp.path().join(s);
        let paths = Paths::new(p("config"), p("data"), p("run"));
        let registry = VaultRegistry::load(paths.registry_file()).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mounts = MountManager::spawn(Fake::default(), paths.clone(), tx);
        let ready = Arc::new(AtomicBool::new(true));
        let keys = Arc::new(FakeKeys::default());
        let service = VaultsService::new(
            registry,
            paths,
            Arc::new(FakeCrypto),
            keys.clone(),
            mounts,
            ready.clone(),
            Activity::new(),
        );

        let (a, b) = UnixStream::pair().unwrap();
        let guid = Guid::generate();
        let server = connection::Builder::unix_stream(a)
            .server(guid)
            .p2p()
            .serve_at(cloister_dbus::SESSION_OBJECT_PATH, service);
        let client = connection::Builder::unix_stream(b).p2p();
        let (server, client) = tokio::try_join!(server.build(), client.build()).unwrap();
        let proxy = Vaults1Proxy::new(&client).await.unwrap();
        Harness {
            _tmp: tmp,
            _server: server,
            _client: client,
            proxy,
            ready,
            keys,
        }
    }

    fn fd(secret: &str) -> Fd<'static> {
        let (r, mut w) = std::io::pipe().unwrap();
        w.write_all(secret.as_bytes()).unwrap();
        Fd::from(std::os::fd::OwnedFd::from(r))
    }

    fn name(e: cloister_dbus::Error) -> String {
        use zbus::DBusError as _;
        e.name().to_string()
    }

    const PREFIX: &str = "io.github.axel7083.Cloister.Error.";

    #[tokio::test(flavor = "multi_thread")]
    async fn create_unlock_lock_remove() {
        let h = harness().await;
        let id = h
            .proxy
            .create("My Vault", fd("good-enough\n"))
            .await
            .unwrap();
        let list = h.proxy.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            (
                list[0].id.as_str(),
                list[0].name.as_str(),
                list[0].state.as_str()
            ),
            (id.as_str(), "My Vault", "locked")
        );

        let wrong = h
            .proxy
            .unlock_with_passphrase(&id, fd("nope"))
            .await
            .unwrap_err();
        assert_eq!(name(wrong), format!("{PREFIX}WrongPassphrase"));

        let mp = h
            .proxy
            .unlock_with_passphrase(&id, fd("good"))
            .await
            .unwrap();
        assert!(mp.ends_with(&format!("run/cloister/{id}")));
        let list = h.proxy.list().await.unwrap();
        assert!(list[0].is_unlocked());
        assert_eq!(list[0].mount_point, mp);

        let busy = h.proxy.remove(&id).await.unwrap_err();
        assert_eq!(name(busy), format!("{PREFIX}Busy"));
        h.proxy.lock(&id, false).await.unwrap();
        h.proxy.remove(&id).await.unwrap();
        assert!(h.proxy.list().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refuses_unlock_without_autolock() {
        let h = harness().await;
        let id = h.proxy.create("V", fd("good-enough")).await.unwrap();
        h.ready.store(false, Ordering::SeqCst);
        let e = h
            .proxy
            .unlock_with_passphrase(&id, fd("good"))
            .await
            .unwrap_err();
        assert_eq!(name(e), format!("{PREFIX}AutoLockUnavailable"));
        assert!(!h.proxy.list().await.unwrap()[0].is_unlocked());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn validates_input() {
        let h = harness().await;
        let short = h.proxy.create("V", fd("short")).await.unwrap_err();
        assert_eq!(name(short), format!("{PREFIX}InvalidArgument"));
        let bad_name = h.proxy.create(" \n ", fd("good-enough")).await.unwrap_err();
        assert_eq!(name(bad_name), format!("{PREFIX}InvalidArgument"));
        let bad_id = h.proxy.lock("../../etc", false).await.unwrap_err();
        assert_eq!(name(bad_id), format!("{PREFIX}InvalidArgument"));
        let unknown = h
            .proxy
            .lock(&VaultId::generate().to_string(), false)
            .await
            .unwrap_err();
        assert_eq!(name(unknown), format!("{PREFIX}UnknownVault"));
        assert!(
            h.proxy.list().await.unwrap().is_empty(),
            "nothing created by failed calls"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fingerprint_enroll_unlock_unenroll() {
        let h = harness().await;
        let id = h.proxy.create("V", fd("good-enough")).await.unwrap();

        let st = h.proxy.get_fingerprint_status(&id).await.unwrap();
        assert!(st.available && !st.enrolled);
        let e = h.proxy.unlock_with_key_service(&id).await.unwrap_err();
        assert_eq!(name(e), format!("{PREFIX}NotEnrolled"));

        let e = h.proxy.enroll(&id, fd("wrong")).await.unwrap_err();
        assert_eq!(name(e), format!("{PREFIX}WrongPassphrase"));
        assert!(
            h.keys.keys.lock().unwrap().is_empty(),
            "nothing stored on a wrong passphrase"
        );

        h.proxy.enroll(&id, fd("good")).await.unwrap();
        assert!(h.proxy.get_fingerprint_status(&id).await.unwrap().enrolled);
        assert_eq!(
            h.keys.keys.lock().unwrap().values().next().unwrap(),
            b"MASTER"
        );

        *h.keys.deny.lock().unwrap() = true;
        let e = h.proxy.unlock_with_key_service(&id).await.unwrap_err();
        assert_eq!(name(e), format!("{PREFIX}Denied"));
        assert!(!h.proxy.list().await.unwrap()[0].is_unlocked());
        *h.keys.deny.lock().unwrap() = false;

        h.ready.store(false, Ordering::SeqCst);
        let e = h.proxy.unlock_with_key_service(&id).await.unwrap_err();
        assert_eq!(name(e), format!("{PREFIX}AutoLockUnavailable"));
        h.ready.store(true, Ordering::SeqCst);

        let mp = h.proxy.unlock_with_key_service(&id).await.unwrap();
        assert!(h.proxy.list().await.unwrap()[0].is_unlocked());
        assert_eq!(
            h.proxy.unlock_with_key_service(&id).await.unwrap(),
            mp,
            "idempotent, no prompt"
        );

        h.proxy.lock(&id, false).await.unwrap();
        h.proxy.unenroll(&id).await.unwrap();
        assert!(!h.proxy.get_fingerprint_status(&id).await.unwrap().enrolled);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn key_service_missing_is_reported_not_fatal() {
        let h = harness().await;
        let id = h.proxy.create("V", fd("good-enough")).await.unwrap();
        *h.keys.unavailable.lock().unwrap() = true;
        let st = h.proxy.get_fingerprint_status(&id).await.unwrap();
        assert!(!st.available && !st.enrolled);
        h.proxy.remove(&id).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_deletes_stored_key() {
        let h = harness().await;
        let id = h.proxy.create("V", fd("good-enough")).await.unwrap();
        h.proxy.enroll(&id, fd("good")).await.unwrap();
        h.proxy.remove(&id).await.unwrap();
        assert!(h.keys.keys.lock().unwrap().is_empty());
    }
}
