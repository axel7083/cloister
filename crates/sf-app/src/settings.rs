//! Per-vault settings: fingerprint unlock, removal.
//!
//! ```text
//! ┌        “Taxes”                         ┐
//! │ Unlocking                              │
//! │ ┌────────────────────────────────────┐ │
//! │ │ Unlock with Fingerprint       [⏽]  │ │
//! │ │ Or your login password             │ │
//! │ └────────────────────────────────────┘ │
//! │ ⚠ Your disk is not encrypted…          │
//! │ ┌────────────────────────────────────┐ │
//! │ │        Remove from List            │ │
//! │ └────────────────────────────────────┘ │
//! └────────────────────────────────────────┘
//! ```

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use sf_dbus::VaultInfo;

use crate::client::Client;
use crate::dialogs::{self, PasswordPrompt};

const NOT_ENCRYPTED_WARNING: &str = "Your system disk is not encrypted. With fingerprint unlock \
     turned on, someone with access to the disk could open this folder without its password.";

pub async fn show(client: Client, vault: VaultInfo, parent: gtk::Widget) {
    let status = client.fingerprint_status(vault.id.clone()).await;

    let dialog = adw::PreferencesDialog::builder()
        .title(format!("“{}”", vault.name))
        .search_enabled(false)
        .content_width(460)
        .build();
    let page = adw::PreferencesPage::new();

    let unlocking = adw::PreferencesGroup::builder().title("Unlocking").build();
    let fingerprint = adw::SwitchRow::builder()
        .title("Unlock with Fingerprint")
        .subtitle("Or with your login password. The folder password always works too.")
        .build();
    unlocking.add(&fingerprint);
    match &status {
        Ok(s) if s.available => {
            fingerprint.set_active(s.enrolled);
            if !s.system_disk_encrypted {
                unlocking.set_description(Some(NOT_ENCRYPTED_WARNING));
            }
        }
        Ok(_) => {
            fingerprint.set_sensitive(false);
            fingerprint.set_subtitle("Not available: the Secure Folder key service is not installed.");
        }
        Err(e) => {
            fingerprint.set_sensitive(false);
            fingerprint.set_subtitle(&glib::markup_escape_text(&e.message()));
        }
    }
    page.add(&unlocking);

    let danger = adw::PreferencesGroup::new();
    let remove = adw::ButtonRow::builder().title("_Remove from List").use_underline(true).build();
    remove.add_css_class("destructive-action");
    danger.add(&remove);
    page.add(&danger);
    dialog.add(&page);

    // Reverting the switch after a failure must not re-trigger the handler.
    let reverting = Rc::new(Cell::new(false));
    fingerprint.connect_active_notify(glib::clone!(
        #[strong]
        client,
        #[strong]
        vault,
        #[strong]
        reverting,
        #[weak]
        dialog,
        move |row| {
            if reverting.get() {
                return;
            }
            let (client, vault, reverting, row, dialog) =
                (client.clone(), vault.clone(), reverting.clone(), row.clone(), dialog.clone());
            glib::spawn_future_local(async move {
                row.set_sensitive(false);
                let want = row.is_active();
                let ok = if want {
                    enroll(&client, &vault, dialog.upcast_ref()).await
                } else {
                    match client.unenroll(vault.id.clone()).await {
                        Ok(()) => true,
                        Err(e) => {
                            dialogs::show_error("Cannot Turn Off Fingerprint Unlock", &e.message(), Some(dialog.upcast_ref()))
                                .await;
                            false
                        }
                    }
                };
                if !ok {
                    reverting.set(true);
                    row.set_active(!want);
                    reverting.set(false);
                }
                row.set_sensitive(true);
            });
        }
    ));

    remove.connect_activated(glib::clone!(
        #[strong]
        client,
        #[strong]
        vault,
        #[weak]
        dialog,
        move |_| {
            let (client, vault, dialog) = (client.clone(), vault.clone(), dialog.clone());
            glib::spawn_future_local(async move {
                if confirm_remove(&vault.name, dialog.upcast_ref()).await {
                    match client.remove(vault.id.clone()).await {
                        Ok(()) => {
                            dialog.close();
                        }
                        Err(e) => {
                            dialogs::show_error("Cannot Remove", &e.message(), Some(dialog.upcast_ref())).await
                        }
                    }
                }
            });
        }
    ));

    dialog.present(Some(&parent));
}

/// Asks the folder password (proves knowledge of it), then the session
/// service extracts the key and the system asks for authentication.
async fn enroll(client: &Client, vault: &VaultInfo, parent: &gtk::Widget) -> bool {
    let mut error = None;
    loop {
        let prompt = PasswordPrompt {
            heading: "Turn On Fingerprint Unlock".into(),
            body: "Enter the password of this Secure Folder.",
            accept: "_Continue",
        };
        let Some(password) = dialogs::ask_password(prompt, error, Some(parent)).await else {
            return false;
        };
        match client.enroll(vault.id.clone(), password).await {
            Ok(()) => return true,
            Err(e) if e.kind() == Some("WrongPassphrase") => error = Some("Wrong password. Try again."),
            Err(e) if e.kind() == Some("Denied") => return false, // cancelled the system dialog
            Err(e) => {
                dialogs::show_error("Cannot Turn On Fingerprint Unlock", &e.message(), Some(parent)).await;
                return false;
            }
        }
    }
}

async fn confirm_remove(name: &str, parent: &gtk::Widget) -> bool {
    let dialog = adw::AlertDialog::new(
        Some(&format!("Remove “{name}”?")),
        Some("The folder disappears from Secure Folder. Its encrypted files stay on disk and are not deleted."),
    );
    dialog.add_responses(&[("cancel", "_Cancel"), ("remove", "_Remove")]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.choose_future(Some(parent)).await == "remove"
}
