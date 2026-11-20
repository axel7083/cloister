//! sf-session: mounts vaults and locks them when the session locks.
//!
//! Started on demand by D-Bus activation. Exits after [`IDLE_EXIT`] without
//! calls, but only while every vault is locked: while one is unlocked this
//! process is the auto-lock watcher and must stay.

mod autolock;
mod mounts;
mod passfd;
mod service;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sf_core::{Gocryptfs, Paths, VaultRegistry};
use sf_dbus::{SESSION_BUS_NAME, SESSION_OBJECT_PATH, STATE_LOCKED, STATE_UNLOCKED};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;
use zbus::{connection, Connection};

use crate::autolock::{AutoLock, LockReason};
use crate::mounts::{GocryptfsDriver, MountEvent, MountManager};
use crate::service::{Activity, VaultsService, VaultsServiceSignals};

const IDLE_EXIT: Duration = Duration::from_secs(30);
const IDLE_CHECK: Duration = Duration::from_secs(5);

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SF_LOG").unwrap_or_else(|_| "info".into()),
        )
        .without_time() // the journal adds timestamps
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), BoxError> {
    let paths = Paths::from_env()?;
    let registry = VaultRegistry::load(paths.registry_file())?;
    let gocryptfs = Gocryptfs::default();
    if !gocryptfs.is_available() {
        return Err("gocryptfs is not installed".into());
    }

    let (mount_tx, mut mount_rx) = mpsc::unbounded_channel();
    let mounts = MountManager::spawn(GocryptfsDriver(gocryptfs.clone()), paths.clone(), mount_tx);
    mounts.reap_stale(registry.list().iter().map(|v| *v.id()).collect()).await;

    let autolock_ready = Arc::new(AtomicBool::new(false));
    let activity = Activity::new();
    let service = VaultsService::new(
        registry,
        paths,
        Arc::new(gocryptfs),
        mounts.clone(),
        autolock_ready.clone(),
        activity.clone(),
    );
    let session = connection::Builder::session()
        .serve_at(SESSION_OBJECT_PATH, service)
        .name(SESSION_BUS_NAME)
        .build()
        .await?;
    let iface = session
        .object_server()
        .interface::<_, VaultsService>(SESSION_OBJECT_PATH)
        .await?;

    let (lock_tx, mut lock_rx) = mpsc::channel(8);
    let autolock = start_autolock(lock_tx, autolock_ready.clone()).await;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut idle = tokio::time::interval(IDLE_CHECK);
    let mut sleep_inhibitor = None;
    tracing::info!("ready");

    loop {
        tokio::select! {
            Some(event) = mount_rx.recv() => {
                let (id, state, mp) = match &event {
                    MountEvent::Unlocked(id, p) => (id, STATE_UNLOCKED, p.display().to_string()),
                    MountEvent::Locked(id) => (id, STATE_LOCKED, String::new()),
                };
                if let Err(e) = iface.state_changed(&id.to_string(), state, &mp).await {
                    tracing::warn!("cannot emit StateChanged: {e}");
                }
                let any_mounted = !mounts.mounted().is_empty();
                match (&autolock, any_mounted, sleep_inhibitor.is_some()) {
                    (Some(a), true, false) => match a.inhibit_sleep().await {
                        Ok(fd) => sleep_inhibitor = Some(fd),
                        Err(e) => tracing::warn!("no sleep inhibitor, relying on the screen lock: {e}"),
                    },
                    (_, false, true) => sleep_inhibitor = None, // closing the fd releases it
                    _ => {}
                }
            }
            Some(reason) = lock_rx.recv() => {
                if !mounts.mounted().is_empty() {
                    tracing::info!(?reason, "automatic lock");
                    mounts.unmount_all().await;
                }
                if reason == LockReason::Sleep {
                    sleep_inhibitor = None;
                }
            }
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            _ = idle.tick() => {
                if mounts.mounted().is_empty() && activity.idle_for() > IDLE_EXIT {
                    tracing::info!("idle, exiting");
                    // Release first: calls arriving now re-activate a new instance.
                    session.release_name(SESSION_BUS_NAME).await?;
                    break;
                }
            }
        }
    }
    mounts.unmount_all().await;
    Ok(())
}

/// Starts the logind watcher. Unlocking stays refused until it is armed, and
/// becomes refused again (with every vault locked) if it stops.
async fn start_autolock(tx: mpsc::Sender<LockReason>, ready: Arc<AtomicBool>) -> Option<Arc<AutoLock>> {
    let connect = async { AutoLock::connect(&Connection::system().await?).await };
    let autolock = match connect.await {
        Ok(a) => Arc::new(a),
        Err(e) => {
            tracing::error!("automatic locking unavailable, unlocking disabled: {e}");
            return None;
        }
    };
    let runner = autolock.clone();
    tokio::spawn(async move {
        let result = runner.run(tx.clone(), || ready.store(true, Ordering::SeqCst)).await;
        ready.store(false, Ordering::SeqCst);
        tracing::error!("automatic locking stopped ({result:?}), locking everything");
        let _ = tx.send(LockReason::WatcherLost).await;
    });
    Some(autolock)
}
