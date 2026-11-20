//! cloister-keyd: stores vault master keys and releases them to their owner after
//! polkit authentication (fingerprint or login password).
//!
//! Runs on the system bus as the unprivileged `cloister` user, started
//! by D-Bus activation, and exits when idle.

mod activity;
mod disk;
mod gate;
mod service;
mod store;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use cloister_dbus::{KEYS_BUS_NAME, KEYS_OBJECT_PATH};
use tokio::signal::unix::{signal, SignalKind};
use zbus::{connection, Connection};

use crate::activity::Activity;
use crate::gate::BusGate;
use crate::service::KeyService;
use crate::store::KeyStore;

const IDLE_EXIT: Duration = Duration::from_secs(30);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("CLOISTER_LOG").unwrap_or_else(|_| "info".into()),
        )
        .without_time()
        .with_ansi(false)
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

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Set by systemd from StateDirectory=; refuse to guess a location.
    let root = std::env::var_os("STATE_DIRECTORY")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or("STATE_DIRECTORY is not set (run through systemd)")?;
    let disk_encrypted = disk::is_on_encrypted_disk(&root);
    if !disk_encrypted {
        tracing::warn!("key store is not on an encrypted disk");
    }

    let system = Connection::system().await?;
    let activity = Activity::new();
    let service = KeyService::new(
        KeyStore::new(root),
        Arc::new(BusGate::new(system.clone())),
        activity.clone(),
        disk_encrypted,
    );
    let conn = connection::Builder::system()
        .serve_at(KEYS_OBJECT_PATH, service)
        .name(KEYS_BUS_NAME)
        .build()
        .await?;
    tracing::info!("ready");

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = sigterm.recv() => { tracing::info!("SIGTERM, exiting"); break }
            _ = sigint.recv() => { tracing::info!("SIGINT, exiting"); break }
            _ = sighup.recv() => { tracing::info!("SIGHUP, exiting"); break }
            _ = tick.tick() => {
                if activity.idle_for() > IDLE_EXIT {
                    conn.release_name(KEYS_BUS_NAME).await?;
                    tracing::info!("idle, exiting");
                    break;
                }
            }
        }
    }
    Ok(())
}
