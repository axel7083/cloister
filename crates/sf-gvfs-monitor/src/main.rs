//! sf-gvfs-monitor: shows Secure Folder vaults in the sidebar of every GIO
//! file manager (Nautilus, Nemo, Caja, Thunar…) through GVfs' remote volume
//! monitor protocol. A thin front-end over the public D-Bus contract.
//!
//! GVfs clients start it (D-Bus activation from our `.monitor` file) and
//! expect it to live for the whole session.

mod monitor;
mod state;
mod wire;

use std::process::ExitCode;

use futures_util::StreamExt;
use sf_dbus::{Prompt1Proxy, Vaults1Proxy};
use tokio::signal::unix::{signal, SignalKind};
use zbus::connection;

use crate::monitor::Monitor;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SF_LOG").unwrap_or_else(|_| "info".into()),
        )
        .without_time()
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

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Clients (proxies) on their own connection; the monitor object on the
    // named one.
    let client = zbus::Connection::session().await?;
    let vaults = Vaults1Proxy::new(&client).await?;
    let prompt = Prompt1Proxy::new(&client).await?;

    let conn = connection::Builder::session()
        .serve_at(wire::OBJECT_PATH, Monitor::new(vaults.clone(), prompt))
        .build()
        .await?;
    let iface = conn.object_server().interface::<_, Monitor>(wire::OBJECT_PATH).await?;
    let emitter = iface.signal_emitter().clone();
    iface.get().await.refresh(&emitter).await;
    // Take the name once the initial state is loaded: clients List() right away.
    conn.request_name(wire::BUS_NAME).await?;
    tracing::info!("ready");

    let mut states = vaults.receive_state_changed().await?;
    let mut changed = vaults.receive_vaults_changed().await?;
    let mut owner = vaults.inner().receive_owner_changed().await?;
    let mut sigterm = signal(SignalKind::terminate())?;
    loop {
        tokio::select! {
            Some(_) = states.next() => iface.get().await.refresh(&emitter).await,
            Some(_) = changed.next() => iface.get().await.refresh(&emitter).await,
            Some(new_owner) = owner.next() => {
                // Do not List() when it left: that would start it again.
                if new_owner.is_none() {
                    iface.get().await.session_gone(&emitter).await;
                } else {
                    iface.get().await.refresh(&emitter).await;
                }
            }
            _ = sigterm.recv() => break,
        }
    }
    Ok(())
}
