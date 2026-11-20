//! Mount lifecycle: one dedicated thread owns every mounted vault and runs
//! mount / unmount operations one at a time (no two operations can race on
//! the same vault). The async side talks to it through [`MountManager`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use cloister_core::{Gocryptfs, MountHandle, Paths, Result, Unlock, Vault, VaultId};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

/// How often mounted vaults are checked for an unexpected exit of gocryptfs.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(2);

/// Mount backend. Implemented by gocryptfs in production, by a fake in tests.
pub trait MountDriver: Send + 'static {
    type Handle: Mounted;
    fn mount(&self, vault: &Vault, at: &Path, unlock: Unlock) -> Result<Self::Handle>;
    fn is_mounted(&self, at: &Path) -> bool;
    /// Lazily unmounts a mount point we hold no handle for.
    fn unmount_stale(&self, at: &Path) -> Result<()>;
}

pub trait Mounted: Send + 'static {
    fn unmount(&mut self, lazy: bool) -> Result<()>;
    fn is_alive(&mut self) -> bool;
}

/// Something observable happened to a vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountEvent {
    Unlocked(VaultId, PathBuf),
    Locked(VaultId),
}

enum Cmd {
    Mount(Vault, Unlock, oneshot::Sender<Result<PathBuf>>),
    Unmount(VaultId, bool, oneshot::Sender<Result<()>>),
    UnmountAll(oneshot::Sender<()>),
    ReapStale(Vec<VaultId>, oneshot::Sender<()>),
}

/// Handle to the mount thread. Dropping the last handle unmounts everything.
#[derive(Clone)]
pub struct MountManager {
    tx: mpsc::Sender<Cmd>,
    mounted: Arc<Mutex<HashMap<VaultId, PathBuf>>>,
}

impl MountManager {
    pub fn spawn<D: MountDriver>(driver: D, paths: Paths, events: tokio_mpsc::UnboundedSender<MountEvent>) -> Self {
        let (tx, rx) = mpsc::channel();
        let mounted = Arc::new(Mutex::new(HashMap::new()));
        let worker = Worker {
            driver,
            paths,
            events,
            handles: HashMap::new(),
            mounted: mounted.clone(),
        };
        thread::Builder::new()
            .name("mounts".into())
            .spawn(move || worker.run(rx))
            .expect("spawn mount thread");
        Self { tx, mounted }
    }

    /// Mounts `vault`; returns its mount point. Already mounted: returns it.
    pub async fn mount(&self, vault: Vault, unlock: Unlock) -> Result<PathBuf> {
        self.call(|r| Cmd::Mount(vault, unlock, r)).await
    }

    /// Unmounts; with `force`, open files do not prevent locking.
    pub async fn unmount(&self, id: VaultId, force: bool) -> Result<()> {
        self.call(|r| Cmd::Unmount(id, force, r)).await
    }

    /// Force-unmounts every vault.
    pub async fn unmount_all(&self) {
        self.call(Cmd::UnmountAll).await;
    }

    /// Lazily unmounts mount points of `ids` left over by a previous instance.
    pub async fn reap_stale(&self, ids: Vec<VaultId>) {
        self.call(|r| Cmd::ReapStale(ids, r)).await;
    }

    /// Snapshot of mounted vaults.
    pub fn mounted(&self) -> HashMap<VaultId, PathBuf> {
        self.mounted.lock().expect("mount state poisoned").clone()
    }

    async fn call<T>(&self, cmd: impl FnOnce(oneshot::Sender<T>) -> Cmd) -> T {
        let (reply, rx) = oneshot::channel();
        self.tx.send(cmd(reply)).expect("mount thread alive");
        rx.await.expect("mount thread replied")
    }
}

struct Worker<D: MountDriver> {
    driver: D,
    paths: Paths,
    events: tokio_mpsc::UnboundedSender<MountEvent>,
    handles: HashMap<VaultId, D::Handle>,
    mounted: Arc<Mutex<HashMap<VaultId, PathBuf>>>,
}

impl<D: MountDriver> Worker<D> {
    fn run(mut self, rx: mpsc::Receiver<Cmd>) {
        loop {
            let cmd = if self.handles.is_empty() {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            } else {
                rx.recv_timeout(LIVENESS_INTERVAL)
            };
            match cmd {
                Ok(Cmd::Mount(v, u, r)) => drop(r.send(self.mount(v, u))),
                Ok(Cmd::Unmount(id, force, r)) => drop(r.send(self.unmount(&id, force))),
                Ok(Cmd::UnmountAll(r)) => drop(r.send(self.unmount_all())),
                Ok(Cmd::ReapStale(ids, r)) => drop(r.send(self.reap_stale(&ids))),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.check_liveness();
        }
        self.unmount_all();
    }

    fn mount(&mut self, vault: Vault, unlock: Unlock) -> Result<PathBuf> {
        let id = *vault.id();
        let at = self.paths.mount_point(&id);
        if self.handles.contains_key(&id) {
            return Ok(at);
        }
        if self.driver.is_mounted(&at) {
            tracing::warn!(%id, "unmounting a stale mount before unlocking");
            self.driver.unmount_stale(&at)?;
        }
        let handle = self.driver.mount(&vault, &at, unlock)?;
        self.handles.insert(id, handle);
        self.set_mounted(id, Some(at.clone()));
        tracing::info!(%id, "unlocked");
        Ok(at)
    }

    fn unmount(&mut self, id: &VaultId, force: bool) -> Result<()> {
        let Some(handle) = self.handles.get_mut(id) else { return Ok(()) };
        handle.unmount(force)?;
        self.handles.remove(id);
        self.set_mounted(*id, None);
        tracing::info!(%id, force, "locked");
        Ok(())
    }

    fn unmount_all(&mut self) {
        let ids: Vec<_> = self.handles.keys().copied().collect();
        for id in ids {
            if let Err(e) = self.unmount(&id, true) {
                tracing::error!(%id, "forced lock failed: {e}");
            }
        }
    }

    fn reap_stale(&mut self, ids: &[VaultId]) {
        for id in ids.iter().filter(|id| !self.handles.contains_key(id)) {
            let at = self.paths.mount_point(id);
            if self.driver.is_mounted(&at) {
                tracing::warn!(%id, "locking vault left mounted by a previous instance");
                if let Err(e) = self.driver.unmount_stale(&at) {
                    tracing::error!(%id, "could not unmount stale mount: {e}");
                }
            }
        }
    }

    /// gocryptfs exited on its own (crash, external `fusermount -u`): the
    /// vault is locked; make sure nothing is left mounted.
    fn check_liveness(&mut self) {
        let dead: Vec<_> = self
            .handles
            .iter_mut()
            .filter_map(|(id, h)| (!h.is_alive()).then_some(*id))
            .collect();
        for id in dead {
            tracing::warn!(%id, "gocryptfs exited unexpectedly");
            let mut handle = self.handles.remove(&id).expect("present");
            let _ = handle.unmount(true);
            let at = self.paths.mount_point(&id);
            if self.driver.is_mounted(&at) {
                let _ = self.driver.unmount_stale(&at);
            }
            self.set_mounted(id, None);
        }
    }

    fn set_mounted(&self, id: VaultId, at: Option<PathBuf>) {
        let mut m = self.mounted.lock().expect("mount state poisoned");
        let event = match at {
            Some(p) => {
                m.insert(id, p.clone());
                MountEvent::Unlocked(id, p)
            }
            None => {
                m.remove(&id);
                MountEvent::Locked(id)
            }
        };
        drop(m);
        let _ = self.events.send(event);
    }
}

/// Production driver.
pub struct GocryptfsDriver(pub Gocryptfs);

impl MountDriver for GocryptfsDriver {
    type Handle = MountHandle;

    fn mount(&self, vault: &Vault, at: &Path, unlock: Unlock) -> Result<MountHandle> {
        self.0.mount(vault.cipher_dir(), at, unlock)
    }

    fn is_mounted(&self, at: &Path) -> bool {
        cloister_core::mountinfo::is_mounted(at).unwrap_or(false)
    }

    fn unmount_stale(&self, at: &Path) -> Result<()> {
        self.0.unmount_path(at, true)
    }
}

impl Mounted for MountHandle {
    fn unmount(&mut self, lazy: bool) -> Result<()> {
        MountHandle::unmount(self, lazy)
    }

    fn is_alive(&mut self) -> bool {
        MountHandle::is_alive(self)
    }
}

/// In-memory backend for tests: the passphrase "good" (or any master key) unlocks.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cloister_core::{Error, ExposeSecret};

    #[derive(Default)]
    pub struct FakeState {
        pub mounted: HashSet<PathBuf>,
        pub busy: bool,
        pub alive: Vec<Arc<AtomicBool>>,
    }

    #[derive(Clone, Default)]
    pub struct Fake(pub Arc<Mutex<FakeState>>);

    pub struct FakeHandle {
        fake: Fake,
        at: PathBuf,
        alive: Arc<AtomicBool>,
    }

    impl MountDriver for Fake {
        type Handle = FakeHandle;
        fn mount(&self, _: &Vault, at: &Path, unlock: Unlock) -> Result<FakeHandle> {
            match unlock {
                Unlock::Passphrase(p) if p.expose_secret() == b"good" => {}
                Unlock::MasterKey(_) => {}
                Unlock::Passphrase(_) => return Err(Error::WrongPassphrase),
            }
            let alive = Arc::new(AtomicBool::new(true));
            let mut s = self.0.lock().unwrap();
            s.mounted.insert(at.to_owned());
            s.alive.push(alive.clone());
            Ok(FakeHandle { fake: self.clone(), at: at.to_owned(), alive })
        }
        fn is_mounted(&self, at: &Path) -> bool {
            self.0.lock().unwrap().mounted.contains(at)
        }
        fn unmount_stale(&self, at: &Path) -> Result<()> {
            self.0.lock().unwrap().mounted.remove(at);
            Ok(())
        }
    }

    impl Mounted for FakeHandle {
        fn unmount(&mut self, lazy: bool) -> Result<()> {
            let mut s = self.fake.0.lock().unwrap();
            if s.busy && !lazy {
                return Err(Error::Busy);
            }
            s.mounted.remove(&self.at);
            self.alive.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn is_alive(&mut self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
    }

}

#[cfg(test)]
mod tests {
    use super::fake::Fake;
    use super::*;
    use std::sync::atomic::Ordering;

    use cloister_core::{secret_from_vec, Error};

    fn setup() -> (Fake, MountManager, tokio_mpsc::UnboundedReceiver<MountEvent>) {
        let fake = Fake::default();
        let (tx, rx) = tokio_mpsc::unbounded_channel();
        let paths = Paths::new("/c".into(), "/d".into(), "/r".into());
        (fake.clone(), MountManager::spawn(fake, paths, tx), rx)
    }

    fn vault() -> Vault {
        Vault::new(VaultId::generate(), "V", "/cipher".into()).unwrap()
    }

    fn pass(p: &str) -> Unlock {
        Unlock::Passphrase(secret_from_vec(p.as_bytes().to_vec()))
    }

    #[tokio::test]
    async fn mount_unmount_emits_events() {
        let (_, m, mut ev) = setup();
        let v = vault();
        let id = *v.id();
        let at = m.mount(v.clone(), pass("good")).await.unwrap();
        assert_eq!(at, Path::new("/r/cloister").join(id.to_string()));
        assert_eq!(ev.recv().await, Some(MountEvent::Unlocked(id, at.clone())));
        assert_eq!(m.mount(v, pass("good")).await.unwrap(), at, "idempotent");
        assert_eq!(m.mounted().get(&id), Some(&at));

        m.unmount(id, false).await.unwrap();
        assert_eq!(ev.recv().await, Some(MountEvent::Locked(id)));
        assert!(m.mounted().is_empty());
        m.unmount(id, false).await.unwrap(); // already locked: no-op
    }

    #[tokio::test]
    async fn wrong_passphrase_leaves_vault_locked() {
        let (_, m, _ev) = setup();
        assert!(matches!(m.mount(vault(), pass("bad")).await, Err(Error::WrongPassphrase)));
        assert!(m.mounted().is_empty());
    }

    #[tokio::test]
    async fn busy_needs_force_and_unmount_all_forces() {
        let (fake, m, _ev) = setup();
        let (a, b) = (vault(), vault());
        m.mount(a.clone(), pass("good")).await.unwrap();
        m.mount(b.clone(), pass("good")).await.unwrap();
        fake.0.lock().unwrap().busy = true;

        assert!(matches!(m.unmount(*a.id(), false).await, Err(Error::Busy)));
        assert_eq!(m.mounted().len(), 2, "still mounted after Busy");
        m.unmount_all().await;
        assert!(m.mounted().is_empty());
        assert!(fake.0.lock().unwrap().mounted.is_empty());
    }

    #[tokio::test]
    async fn stale_mounts_are_reaped() {
        let (fake, m, _ev) = setup();
        let v = vault();
        let at = Path::new("/r/cloister").join(v.id().to_string());
        fake.0.lock().unwrap().mounted.insert(at.clone());
        m.reap_stale(vec![*v.id()]).await;
        assert!(!fake.is_mounted(&at));
    }

    #[tokio::test]
    async fn crashed_gocryptfs_is_reported_as_locked() {
        let (fake, m, mut ev) = setup();
        let v = vault();
        m.mount(v.clone(), pass("good")).await.unwrap();
        let _ = ev.recv().await;
        // gocryptfs dies but its mount is left behind.
        fake.0.lock().unwrap().alive[0].store(false, Ordering::SeqCst);
        let locked = tokio::time::timeout(LIVENESS_INTERVAL * 3, ev.recv()).await.unwrap();
        assert_eq!(locked, Some(MountEvent::Locked(*v.id())));
        assert!(m.mounted().is_empty());
        assert!(fake.0.lock().unwrap().mounted.is_empty(), "leftover mount cleaned");
    }
}

#[cfg(test)]
mod real_tests {
    use super::*;
    use cloister_core::secret_from_vec;

    /// Real gocryptfs through the mount thread, unlocking with a master key
    /// (the fingerprint path). Skipped without gocryptfs/FUSE.
    #[tokio::test(flavor = "multi_thread")]
    async fn master_key_mount_through_manager() {
        let g = Gocryptfs::default();
        if !g.is_available() || !Path::new("/dev/fuse").exists() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("c"), tmp.path().join("d"), tmp.path().join("r"));
        let id = VaultId::generate();
        let cipher = paths.default_cipher_dir(&id);
        let pass = secret_from_vec(b"pw-pw-pw".to_vec());
        g.init(&cipher, &pass).unwrap();
        let key = g.dump_master_key(&cipher, &pass).unwrap();

        let (tx, _rx) = tokio_mpsc::unbounded_channel();
        let m = MountManager::spawn(GocryptfsDriver(g), paths, tx);
        let vault = Vault::new(id, "V", cipher).unwrap();
        let at = m.mount(vault, Unlock::MasterKey(key)).await.unwrap();
        assert!(cloister_core::mountinfo::is_mounted(&at).unwrap());
        m.unmount(id, false).await.unwrap();
    }
}
