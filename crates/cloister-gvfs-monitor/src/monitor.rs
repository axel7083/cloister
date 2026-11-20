//! `org.gtk.Private.RemoteVolumeMonitor`: one volume per vault, a mount while
//! it is unlocked. Mounting asks the app to prompt (`Prompt1.Unlock`),
//! unmounting locks (`Vaults1.Lock`). No vault logic lives here.

use cloister_dbus::{Prompt1Proxy, VaultInfo, Vaults1Proxy};
use tokio::sync::Mutex;
use zbus::interface;
use zbus::object_server::SignalEmitter;

use crate::state::{self, Change, Snapshot};
use crate::wire::{self, Drive, GioError, Mount, Volume, BUS_NAME};

/// `G_MOUNT_UNMOUNT_FORCE`
const UNMOUNT_FORCE: u32 = 1;

pub struct Monitor {
    /// Held across signal emission so events go out in order.
    state: Mutex<Snapshot>,
    vaults: Vaults1Proxy<'static>,
    prompt: Prompt1Proxy<'static>,
}

impl Monitor {
    pub fn new(vaults: Vaults1Proxy<'static>, prompt: Prompt1Proxy<'static>) -> Self {
        Self {
            state: Mutex::new(Snapshot::new()),
            vaults,
            prompt,
        }
    }

    /// Re-reads the vault list and announces the differences.
    pub async fn refresh(&self, emitter: &SignalEmitter<'_>) {
        match self.vaults.list().await {
            Ok(list) => self.apply(emitter, |_| state::snapshot(list)).await,
            Err(e) => tracing::warn!("cannot list vaults: {}", e.message()),
        }
    }

    /// The session service exited (it only does when everything is locked).
    pub async fn session_gone(&self, emitter: &SignalEmitter<'_>) {
        self.apply(emitter, state::all_locked).await;
    }

    async fn apply(&self, emitter: &SignalEmitter<'_>, next: impl FnOnce(&Snapshot) -> Snapshot) {
        let mut current = self.state.lock().await;
        let new = next(&current);
        for change in state::diff(&current, &new) {
            if let Err(e) = emit(emitter, &change).await {
                tracing::warn!("cannot emit {change:?}: {e}");
            }
        }
        *current = new;
    }

    async fn vault(&self, object_id: &str) -> Result<VaultInfo, GioError> {
        let id =
            wire::vault_id_of(object_id).ok_or_else(|| GioError::Code0("unknown volume".into()))?;
        self.state
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| GioError::Code0("unknown volume".into()))
    }
}

async fn emit(e: &SignalEmitter<'_>, change: &Change) -> zbus::Result<()> {
    match change {
        Change::VolumeAdded(v) => {
            Monitor::volume_added(e, BUS_NAME, &wire::volume_id(&v.id), &wire::volume(v)).await
        }
        Change::VolumeChanged(v) => {
            Monitor::volume_changed(e, BUS_NAME, &wire::volume_id(&v.id), &wire::volume(v)).await
        }
        Change::VolumeRemoved(v) => {
            Monitor::volume_removed(e, BUS_NAME, &wire::volume_id(&v.id), &wire::volume(v)).await
        }
        Change::MountAdded(v) | Change::MountChanged(v) | Change::MountRemoved(v) => {
            // A removed mount is described as it was while unlocked.
            let Some(m) = wire::mount(v) else {
                return Ok(());
            };
            let id = wire::mount_id(&v.id);
            match change {
                Change::MountAdded(_) => Monitor::mount_added(e, BUS_NAME, &id, &m).await,
                Change::MountChanged(_) => Monitor::mount_changed(e, BUS_NAME, &id, &m).await,
                _ => Monitor::mount_removed(e, BUS_NAME, &id, &m).await,
            }
        }
    }
}

#[interface(name = "org.gtk.Private.RemoteVolumeMonitor")]
impl Monitor {
    fn is_supported(&self) -> bool {
        true
    }

    #[zbus(out_args("drives", "volumes", "mounts"))]
    async fn list(&self) -> (Vec<Drive>, Vec<Volume>, Vec<Mount>) {
        let s = self.state.lock().await;
        (
            Vec::new(),
            s.values().map(wire::volume).collect(),
            s.values().filter_map(wire::mount).collect(),
        )
    }

    fn cancel_operation(&self, _cancellation_id: String) -> bool {
        false
    }

    /// Clicking the locked volume: the app prompts (fingerprint or password).
    async fn volume_mount(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        id: String,
        _cancellation_id: String,
        _mount_flags: u32,
        _mount_op_id: String,
    ) -> Result<(), GioError> {
        let vault = self.vault(&id).await?;
        tracing::info!(vault = %vault.id, "VolumeMount");
        if vault.is_unlocked() {
            return Ok(());
        }
        match self.prompt.unlock(&vault.id).await {
            Ok(true) => {
                // Announce the mount before replying, so the file manager
                // finds it and opens it as soon as the call returns.
                self.refresh(&emitter).await;
                Ok(())
            }
            Ok(false) => Err(GioError::Code30("cancelled".into())),
            // The app already showed its own error dialog.
            Err(e) if e.kind().is_some() => Err(GioError::Code30(e.message())),
            Err(e) => Err(GioError::Code0(format!(
                "Cloister is not available: {}",
                e.message()
            ))),
        }
    }

    /// The eject button: lock.
    async fn mount_unmount(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        id: String,
        _cancellation_id: String,
        unmount_flags: u32,
        _mount_op_id: String,
    ) -> Result<(), GioError> {
        let vault = self.vault(&id).await?;
        tracing::info!(vault = %vault.id, flags = unmount_flags, "MountUnmount");
        let result = self
            .vaults
            .lock(&vault.id, unmount_flags & UNMOUNT_FORCE != 0)
            .await;
        self.refresh(&emitter).await;
        match result {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == Some("Busy") => Err(GioError::Code26(format!(
                "Files in “{}” are in use. Close them, or lock it from the Cloister app.",
                vault.name
            ))),
            Err(e) => Err(GioError::Code0(e.message())),
        }
    }

    fn drive_eject(&self, _id: String, _c: String, _f: u32, _m: String) -> Result<(), GioError> {
        Err(GioError::Code15("no drives".into()))
    }

    fn drive_poll_for_media(&self, _id: String, _c: String) -> Result<(), GioError> {
        Err(GioError::Code15("no drives".into()))
    }

    fn drive_start(&self, _id: String, _c: String, _f: u32, _m: String) -> Result<(), GioError> {
        Err(GioError::Code15("no drives".into()))
    }

    fn drive_stop(&self, _id: String, _c: String, _f: u32, _m: String) -> Result<(), GioError> {
        Err(GioError::Code15("no drives".into()))
    }

    /// We never ask questions through the mount operation.
    #[allow(clippy::too_many_arguments)]
    fn mount_op_reply(
        &self,
        _op: String,
        _r: i32,
        _u: String,
        _d: String,
        _p: String,
        _s: i32,
        _c: i32,
        _a: bool,
    ) {
    }

    #[allow(clippy::too_many_arguments)]
    fn mount_op_reply2(
        &self,
        _op: String,
        _r: i32,
        _u: String,
        _d: String,
        _p: String,
        _s: i32,
        _c: i32,
        _a: bool,
        _expansion: std::collections::HashMap<String, zbus::OwnedVariant>,
    ) {
    }

    #[zbus(signal)]
    async fn volume_added(
        e: &SignalEmitter<'_>,
        dbus_name: &str,
        id: &str,
        volume: &Volume,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn volume_changed(
        e: &SignalEmitter<'_>,
        dbus_name: &str,
        id: &str,
        volume: &Volume,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn volume_removed(
        e: &SignalEmitter<'_>,
        dbus_name: &str,
        id: &str,
        volume: &Volume,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn mount_added(
        e: &SignalEmitter<'_>,
        dbus_name: &str,
        id: &str,
        mount: &Mount,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn mount_changed(
        e: &SignalEmitter<'_>,
        dbus_name: &str,
        id: &str,
        mount: &Mount,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn mount_removed(
        e: &SignalEmitter<'_>,
        dbus_name: &str,
        id: &str,
        mount: &Mount,
    ) -> zbus::Result<()>;
}
