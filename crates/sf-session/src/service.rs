//! `io.github.axel7083.SecureFolder.Vaults1` implementation.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sf_core::{ExposeSecret, Gocryptfs, Paths, SecretBytes, Unlock, Vault, VaultId, VaultRegistry};
use sf_dbus::{Error, VaultInfo, STATE_LOCKED, STATE_UNLOCKED};
use zbus::object_server::SignalEmitter;
use zbus::{interface, OwnedFd};

use crate::mounts::MountManager;
use crate::passfd::read_passphrase;

/// Baseline policy for new vault passphrases (front-ends may ask for more).
pub const MIN_PASSPHRASE_BYTES: usize = 8;

/// Creates the encrypted structure of a new vault.
pub trait VaultInit: Send + Sync + 'static {
    fn init(&self, cipher_dir: &Path, passphrase: &SecretBytes) -> sf_core::Result<()>;
}

impl VaultInit for Gocryptfs {
    fn init(&self, cipher_dir: &Path, passphrase: &SecretBytes) -> sf_core::Result<()> {
        Gocryptfs::init(self, cipher_dir, passphrase)
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
    init: Arc<dyn VaultInit>,
    mounts: MountManager,
    autolock_ready: Arc<AtomicBool>,
    activity: Activity,
}

impl VaultsService {
    pub fn new(
        registry: VaultRegistry,
        paths: Paths,
        init: Arc<dyn VaultInit>,
        mounts: MountManager,
        autolock_ready: Arc<AtomicBool>,
        activity: Activity,
    ) -> Self {
        Self {
            registry: Arc::new(Mutex::new(registry)),
            paths,
            init,
            mounts,
            autolock_ready,
            activity,
        }
    }

    fn vault(&self, id: &str) -> Result<Vault, Error> {
        let id: VaultId = id.parse().map_err(sf_core::Error::from)?;
        self.registry()
            .get(&id)
            .cloned()
            .ok_or(Error::UnknownVault(format!("no vault {id}")))
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, VaultRegistry> {
        self.registry.lock().expect("registry poisoned")
    }
}

#[interface(name = "io.github.axel7083.SecureFolder.Vaults1")]
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
                    state: if mp.is_some() { STATE_UNLOCKED } else { STATE_LOCKED }.to_owned(),
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

        let init = self.init.clone();
        let dir = cipher_dir.clone();
        tokio::task::spawn_blocking(move || init.init(&dir, &passphrase))
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

    async fn unlock_with_passphrase(&self, id: String, passphrase: OwnedFd) -> Result<String, Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        let passphrase = read_passphrase(passphrase).await?;
        if !self.autolock_ready.load(Ordering::SeqCst) {
            return Err(Error::AutoLockUnavailable(
                "refusing to unlock: automatic locking is not active".into(),
            ));
        }
        let at = self.mounts.mount(vault, Unlock::Passphrase(passphrase)).await?;
        self.activity.touch();
        Ok(at.display().to_string())
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

    async fn remove(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>, id: String) -> Result<(), Error> {
        self.activity.touch();
        let vault = self.vault(&id)?;
        if self.mounts.mounted().contains_key(vault.id()) {
            return Err(Error::Busy("lock the vault before removing it".into()));
        }
        self.registry().remove(vault.id())?;
        tracing::info!(id = %vault.id(), cipher_dir = %vault.cipher_dir().display(), "vault forgotten, files kept");
        emitter.vaults_changed().await?;
        Ok(())
    }

    #[zbus(signal)]
    pub async fn state_changed(emitter: &SignalEmitter<'_>, id: &str, state: &str, mount_point: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn vaults_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    use sf_dbus::Vaults1Proxy;
    use tokio::sync::mpsc;
    use zbus::{connection, Fd, Guid};

    use crate::mounts::fake::Fake;

    struct FakeInit;
    impl VaultInit for FakeInit {
        fn init(&self, dir: &Path, _: &SecretBytes) -> sf_core::Result<()> {
            Ok(std::fs::create_dir_all(dir)?)
        }
    }

    struct Harness {
        _tmp: tempfile::TempDir,
        _server: zbus::Connection,
        _client: zbus::Connection,
        proxy: Vaults1Proxy<'static>,
        ready: Arc<AtomicBool>,
    }

    async fn harness() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let p = |s: &str| tmp.path().join(s);
        let paths = Paths::new(p("config"), p("data"), p("run"));
        let registry = VaultRegistry::load(paths.registry_file()).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mounts = MountManager::spawn(Fake::default(), paths.clone(), tx);
        let ready = Arc::new(AtomicBool::new(true));
        let service =
            VaultsService::new(registry, paths, Arc::new(FakeInit), mounts, ready.clone(), Activity::new());

        let (a, b) = UnixStream::pair().unwrap();
        let guid = Guid::generate();
        let server = connection::Builder::unix_stream(a)
            .server(guid)
            .p2p()
            .serve_at(sf_dbus::SESSION_OBJECT_PATH, service);
        let client = connection::Builder::unix_stream(b).p2p();
        let (server, client) = tokio::try_join!(server.build(), client.build()).unwrap();
        let proxy = Vaults1Proxy::new(&client).await.unwrap();
        Harness { _tmp: tmp, _server: server, _client: client, proxy, ready }
    }

    fn fd(secret: &str) -> Fd<'static> {
        let (r, mut w) = std::io::pipe().unwrap();
        w.write_all(secret.as_bytes()).unwrap();
        Fd::from(std::os::fd::OwnedFd::from(r))
    }

    fn name(e: sf_dbus::Error) -> String {
        use zbus::DBusError as _;
        e.name().to_string()
    }

    const PREFIX: &str = "io.github.axel7083.SecureFolder.Error.";

    #[tokio::test(flavor = "multi_thread")]
    async fn create_unlock_lock_remove() {
        let h = harness().await;
        let id = h.proxy.create("My Vault", fd("good-enough\n")).await.unwrap();
        let list = h.proxy.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id.as_str(), list[0].name.as_str(), list[0].state.as_str()), (id.as_str(), "My Vault", "locked"));

        let wrong = h.proxy.unlock_with_passphrase(&id, fd("nope")).await.unwrap_err();
        assert_eq!(name(wrong), format!("{PREFIX}WrongPassphrase"));

        let mp = h.proxy.unlock_with_passphrase(&id, fd("good")).await.unwrap();
        assert!(mp.ends_with(&format!("run/secure-folder/{id}")));
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
        let e = h.proxy.unlock_with_passphrase(&id, fd("good")).await.unwrap_err();
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
        let unknown = h.proxy.lock(&VaultId::generate().to_string(), false).await.unwrap_err();
        assert_eq!(name(unknown), format!("{PREFIX}UnknownVault"));
        assert!(h.proxy.list().await.unwrap().is_empty(), "nothing created by failed calls");
    }
}
