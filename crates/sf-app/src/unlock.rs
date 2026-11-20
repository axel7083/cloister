//! The unlock flow, shared by the main window and `Prompt1.Unlock`.

use sf_dbus::Error;

use crate::client::Client;
use crate::dialogs::{self, PasswordPrompt};

/// Unlocks vault `id`, prompting as needed. `Ok(true)` once unlocked,
/// `Ok(false)` if the user cancelled. Errors were already shown to the user.
pub async fn unlock(client: &Client, id: &str, parent: Option<&gtk::Widget>) -> Result<bool, Error> {
    let vault = client.get(id).await?;
    if vault.is_unlocked() {
        return Ok(true);
    }
    // Fingerprint first when turned on. Cancelling the system dialog falls
    // back to the vault password.
    if client.fingerprint_status(vault.id.clone()).await.is_ok_and(|s| s.enrolled) {
        match client.unlock_with_key_service(vault.id.clone()).await {
            Ok(_) => return Ok(true),
            Err(e) if matches!(e.kind(), Some("Denied" | "NotEnrolled" | "KeyServiceUnavailable")) => {
                tracing::info!("fingerprint unlock not used: {}", e.message());
            }
            Err(e) => {
                dialogs::show_error(&format!("Cannot Unlock “{}”", vault.name), &e.message(), parent).await;
                return Err(e);
            }
        }
    }
    let mut retry_message = None;
    loop {
        let prompt = PasswordPrompt::unlock(&vault.name);
        let Some(password) = dialogs::ask_password(prompt, retry_message, parent).await else {
            return Ok(false);
        };
        match client.unlock(vault.id.clone(), password).await {
            Ok(_) => return Ok(true),
            Err(e) if e.kind() == Some("WrongPassphrase") => {
                retry_message = Some("Wrong password. Try again.");
            }
            Err(e) => {
                dialogs::show_error(&format!("Cannot Unlock “{}”", vault.name), &e.message(), parent).await;
                return Err(e);
            }
        }
    }
}

/// Locks vault `id`, asking before forcing when files are in use.
pub async fn lock(client: &Client, id: &str, name: &str, parent: Option<&gtk::Widget>) {
    match client.lock(id.to_owned(), false).await {
        Ok(()) => {}
        Err(e) if e.kind() == Some("Busy") => {
            if dialogs::confirm_force_lock(name, parent).await {
                if let Err(e) = client.lock(id.to_owned(), true).await {
                    dialogs::show_error(&format!("Cannot Lock “{name}”"), &e.message(), parent).await;
                }
            }
        }
        Err(e) => dialogs::show_error(&format!("Cannot Lock “{name}”"), &e.message(), parent).await,
    }
}
