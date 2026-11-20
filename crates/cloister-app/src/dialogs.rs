//! Dialogs. Each one is a plain async function: present, await the answer.

use adw::prelude::*;
use gtk::glib;
use zeroize::Zeroizing;

use crate::client::Client;

/// Same rule as the session service (`MIN_PASSPHRASE_BYTES`), checked here
/// first so the user gets immediate feedback.
const MIN_PASSWORD_LEN: usize = 8;

/// What a password prompt is for.
pub struct PasswordPrompt<'a> {
    pub heading: String,
    pub body: &'a str,
    pub accept: &'a str,
}

impl PasswordPrompt<'_> {
    pub fn unlock(vault_name: &str) -> Self {
        Self {
            heading: format!("Unlock “{vault_name}”"),
            body: "Enter the password of this encrypted folder.",
            accept: "_Unlock",
        }
    }
}

/// Asks for a vault password. `None` when cancelled.
pub async fn ask_password(
    prompt: PasswordPrompt<'_>,
    error: Option<&str>,
    parent: Option<&gtk::Widget>,
) -> Option<Zeroizing<String>> {
    let dialog = adw::AlertDialog::new(Some(&prompt.heading), Some(error.unwrap_or(prompt.body)));
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .placeholder_text("Password")
        .build();
    if error.is_some() {
        entry.add_css_class("error");
    }
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "_Cancel"), ("unlock", prompt.accept)]);
    dialog.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("unlock"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("unlock", false);
    entry.connect_changed(glib::clone!(
        #[weak]
        dialog,
        move |e| dialog.set_response_enabled("unlock", !e.text().is_empty())
    ));
    dialog.connect_map(glib::clone!(
        #[weak]
        entry,
        move |_| {
            entry.grab_focus();
        }
    ));

    let response = dialog.choose_future(parent).await;
    let password = Zeroizing::new(entry.text().to_string());
    entry.set_text("");
    (response == "unlock").then_some(password)
}

/// Shows an error message and waits until it is dismissed.
pub async fn show_error(heading: &str, message: &str, parent: Option<&gtk::Widget>) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(message));
    dialog.add_response("close", "_Close");
    dialog.choose_future(parent).await;
}

/// "Files are in use" confirmation. `true` to lock anyway.
pub async fn confirm_force_lock(vault_name: &str, parent: Option<&gtk::Widget>) -> bool {
    let dialog = adw::AlertDialog::new(
        Some(&format!("“{vault_name}” Is in Use")),
        Some("Some files are still open. Locking now may lose unsaved changes in the applications using them."),
    );
    dialog.add_responses(&[("cancel", "_Cancel"), ("force", "_Lock Anyway")]);
    dialog.set_response_appearance("force", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.choose_future(parent).await == "force"
}

/// "New Encrypted Folder" dialog. Creates the vault itself; returns its id.
pub async fn create_vault(client: &Client, parent: &gtk::Widget) -> Option<String> {
    let dialog = adw::Dialog::builder()
        .title("New Encrypted Folder")
        .content_width(420)
        .build();

    let cancel = gtk::Button::with_mnemonic("_Cancel");
    let create = gtk::Button::builder()
        .label("_Create")
        .use_underline(true)
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&create);

    let name = adw::EntryRow::builder()
        .title("Name")
        .text("Private Folder")
        .build();
    let password = adw::PasswordEntryRow::builder().title("Password").build();
    let confirm = adw::PasswordEntryRow::builder()
        .title("Confirm Password")
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(
            "There is no way to recover your files if you forget this password. \
             The folder locks itself when you lock the screen.",
        )
        .build();
    group.add(&name);
    group.add(&password);
    group.add(&confirm);

    let status = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .visible(false)
        .build();
    let spinner = adw::Spinner::builder().visible(false).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&group);
    content.append(&status);
    content.append(&spinner);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&create));

    let validate = glib::clone!(
        #[weak]
        name,
        #[weak]
        password,
        #[weak]
        confirm,
        #[weak]
        create,
        #[weak]
        status,
        move || {
            let (p, c) = (password.text(), confirm.text());
            let problem = if p.chars().count() < MIN_PASSWORD_LEN && !p.is_empty() {
                Some(format!(
                    "The password must have at least {MIN_PASSWORD_LEN} characters."
                ))
            } else if !c.is_empty() && p != c {
                Some("The passwords do not match.".to_owned())
            } else {
                None
            };
            status.set_visible(problem.is_some());
            status.set_label(problem.as_deref().unwrap_or(""));
            let ok =
                !name.text().trim().is_empty() && p.chars().count() >= MIN_PASSWORD_LEN && p == c;
            create.set_sensitive(ok);
        }
    );
    for row in [
        name.upcast_ref::<adw::EntryRow>(),
        password.upcast_ref(),
        confirm.upcast_ref(),
    ] {
        let validate = validate.clone();
        row.connect_changed(move |_| validate());
    }

    let (tx, rx) = async_channel::bounded::<Option<String>>(1);
    cancel.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));
    dialog.connect_closed(glib::clone!(
        #[strong]
        tx,
        #[weak]
        password,
        #[weak]
        confirm,
        move |_| {
            password.set_text("");
            confirm.set_text("");
            // try_send: never overwrite an id already sent by "Create".
            let _ = tx.try_send(None);
        }
    ));
    create.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        name,
        #[weak]
        password,
        #[weak]
        status,
        #[weak]
        spinner,
        #[strong]
        client,
        #[strong]
        tx,
        move |button| {
            button.set_sensitive(false);
            spinner.set_visible(true);
            status.set_visible(false);
            let secret = Zeroizing::new(password.text().to_string());
            let vault_name = name.text().to_string();
            let client = client.clone();
            let tx = tx.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let result = client.create(vault_name, secret).await;
                spinner.set_visible(false);
                match result {
                    Ok(id) => {
                        let _ = tx.try_send(Some(id));
                        dialog.close();
                    }
                    Err(e) => {
                        status.set_label(&e.message());
                        status.set_visible(true);
                        button.set_sensitive(true);
                    }
                }
            });
        }
    ));

    dialog.present(Some(parent));
    // The first value wins: Some(id) on success, None on cancel/close.
    rx.recv().await.ok().flatten()
}
