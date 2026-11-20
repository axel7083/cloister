//! cloister-fake-prompt: stands in for the GTK app in end-to-end tests.
//!
//! Owns `io.github.axel7083.Cloister` on the session bus and answers
//! `Prompt1.Unlock(id)` without a window:
//! - `CLOISTER_TEST_PASSPHRASE_FILE` set: unlocks with that passphrase
//!   through `Vaults1.UnlockWithPassphrase`, returns `true`;
//! - `CLOISTER_TEST_CANCEL=1`: returns `false`, like a user pressing Cancel.
//!
//! Test-only: never installed by the package.

use std::process::ExitCode;

use cloister_dbus::{secret_fd, Error, Vaults1Proxy, APP_BUS_NAME, APP_OBJECT_PATH};
use zbus::{connection, interface};

struct FakePrompt {
    vaults: Vaults1Proxy<'static>,
}

#[interface(name = "io.github.axel7083.Cloister.Prompt1")]
impl FakePrompt {
    async fn unlock(&self, id: String) -> Result<bool, Error> {
        eprintln!("fake-prompt: Unlock({id})");
        if std::env::var("CLOISTER_TEST_CANCEL").as_deref() == Ok("1") {
            return Ok(false);
        }
        let path = std::env::var("CLOISTER_TEST_PASSPHRASE_FILE")
            .map_err(|_| Error::Failed("CLOISTER_TEST_PASSPHRASE_FILE is not set".into()))?;
        let passphrase = tokio::fs::read(&path)
            .await
            .map_err(|e| Error::Failed(e.to_string()))?;
        let fd =
            secret_fd(passphrase.trim_ascii_end()).map_err(|e| Error::Failed(e.to_string()))?;
        self.vaults.unlock_with_passphrase(&id, fd).await?;
        Ok(true)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let run = async {
        let client = zbus::Connection::session().await?;
        let vaults = Vaults1Proxy::new(&client).await?;
        let _conn = connection::Builder::session()
            .serve_at(APP_OBJECT_PATH, FakePrompt { vaults })
            .name(APP_BUS_NAME)
            .build()
            .await?;
        eprintln!("fake-prompt: ready");
        std::future::pending::<()>().await;
        Ok::<_, zbus::Error>(())
    };
    match run.await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fake-prompt: {e}");
            ExitCode::FAILURE
        }
    }
}
