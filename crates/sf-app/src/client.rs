//! Typed access to the session service, usable from GTK code.

use futures_util::StreamExt;
use sf_dbus::{secret_fd, Error, VaultInfo, Vaults1Proxy};
use zbus::Connection;
use zeroize::Zeroizing;

use crate::runtime::{on_tokio, spawn_tokio};

/// Cheap to clone; every method may be awaited from the GTK main loop.
#[derive(Clone)]
pub struct Client {
    proxy: Vaults1Proxy<'static>,
}

impl Client {
    pub async fn connect() -> Result<Self, Error> {
        on_tokio(async {
            let conn = Connection::session().await?;
            Ok(Self { proxy: Vaults1Proxy::new(&conn).await? })
        })
        .await
    }

    pub async fn list(&self) -> Result<Vec<VaultInfo>, Error> {
        let p = self.proxy.clone();
        on_tokio(async move { p.list().await }).await
    }

    pub async fn get(&self, id: &str) -> Result<VaultInfo, Error> {
        self.list()
            .await?
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| Error::UnknownVault(format!("no vault {id}")))
    }

    pub async fn create(&self, name: String, passphrase: Zeroizing<String>) -> Result<String, Error> {
        let p = self.proxy.clone();
        on_tokio(async move {
            let fd = secret_fd(passphrase.as_bytes()).map_err(|e| Error::Failed(e.to_string()))?;
            drop(passphrase);
            p.create(&name, fd).await
        })
        .await
    }

    pub async fn unlock(&self, id: String, passphrase: Zeroizing<String>) -> Result<String, Error> {
        let p = self.proxy.clone();
        on_tokio(async move {
            let fd = secret_fd(passphrase.as_bytes()).map_err(|e| Error::Failed(e.to_string()))?;
            drop(passphrase);
            p.unlock_with_passphrase(&id, fd).await
        })
        .await
    }

    pub async fn lock(&self, id: String, force: bool) -> Result<(), Error> {
        let p = self.proxy.clone();
        on_tokio(async move { p.lock(&id, force).await }).await
    }

    /// Fires whenever a vault is created, removed, locked or unlocked.
    pub fn changes(&self) -> async_channel::Receiver<()> {
        let (tx, rx) = async_channel::bounded(1);
        let p = self.proxy.clone();
        spawn_tokio(async move {
            let (Ok(mut states), Ok(mut vaults)) =
                (p.receive_state_changed().await, p.receive_vaults_changed().await)
            else {
                tracing::warn!("cannot subscribe to vault changes");
                return;
            };
            loop {
                tokio::select! {
                    Some(_) = states.next() => {}
                    Some(_) = vaults.next() => {}
                    else => return,
                }
                // A full channel already means "refresh pending".
                if tx.force_send(()).is_err() {
                    return; // window gone
                }
            }
        });
        rx
    }
}
