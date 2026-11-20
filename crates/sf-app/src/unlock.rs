//! The unlock flow, shared by the main window and `Prompt1.Unlock`.

use sf_dbus::Error;

use crate::client::Client;
use crate::dialogs;

/// Unlocks vault `id`, prompting as needed. `Ok(true)` once unlocked,
/// `Ok(false)` if the user cancelled. Errors were already shown to the user.
pub async fn unlock(client: &Client, id: &str, parent: Option<&gtk::Widget>) -> Result<bool, Error> {
    let vault = client.get(id).await?;
    if vault.is_unlocked() {
        return Ok(true);
    }
    // Milestone 4 inserts the fingerprint path (key service) here.
    let mut retry_message = None;
    loop {
        let Some(password) = dialogs::ask_password(&vault.name, retry_message, parent).await else {
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
