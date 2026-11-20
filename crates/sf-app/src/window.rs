//! Main window: the list of vaults.
//!
//! ```text
//! ┌ [+]          Secure Folder              ┐
//! │ ┌─────────────────────────────────────┐ │
//! │ │ 🔒 Secure Folder        [Unlock]     │ │
//! │ │    Locked                            │ │
//! │ ├─────────────────────────────────────┤ │
//! │ │ 🔓 Taxes          [Open] [Lock]      │ │
//! │ │    Unlocked                          │ │
//! │ └─────────────────────────────────────┘ │
//! └─────────────────────────────────────────┘
//! ```

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use sf_dbus::VaultInfo;

use crate::client::Client;
use crate::{dialogs, settings, unlock};

pub struct VaultsWindow {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    list: gtk::ListBox,
    error_page: adw::StatusPage,
    client: Client,
}

impl VaultsWindow {
    pub fn new(app: &adw::Application, client: Client) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Secure Folder")
            .default_width(560)
            .default_height(480)
            .width_request(360)
            .height_request(294)
            .build();

        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New Secure Folder")
            .build();
        let header = adw::HeaderBar::new();
        header.pack_start(&add);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .valign(gtk::Align::Start)
            .build();
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&list)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();

        let create_button = gtk::Button::builder()
            .label("_Create Secure Folder")
            .use_underline(true)
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("changes-prevent-symbolic")
            .title("No Secure Folders")
            .description("Keep private files in an encrypted folder that locks itself with your screen.")
            .child(&create_button)
            .build();
        let error_page = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title("Secure Folder Service Unavailable")
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&error_page, Some("error"));

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&stack));
        window.set_content(Some(&view));

        let this = Rc::new(Self { window, stack, list, error_page, client });

        for button in [&add, &create_button] {
            button.connect_clicked(glib::clone!(
                #[weak]
                this,
                move |_| this.create()
            ));
        }
        let changes = this.client.changes();
        glib::spawn_future_local(glib::clone!(
            #[weak]
            this,
            async move {
                this.refresh().await;
                while changes.recv().await.is_ok() {
                    this.refresh().await;
                }
            }
        ));
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Called once when the user closes the window.
    pub fn connect_close(&self, f: impl Fn() + 'static) {
        self.window.connect_close_request(move |_| {
            f();
            glib::Propagation::Proceed
        });
    }

    fn widget(&self) -> gtk::Widget {
        self.window.clone().upcast()
    }

    async fn refresh(self: &Rc<Self>) {
        match self.client.list().await {
            Ok(vaults) if vaults.is_empty() => self.stack.set_visible_child_name("empty"),
            Ok(vaults) => {
                self.list.remove_all();
                for v in vaults {
                    self.list.append(&self.row(v));
                }
                self.stack.set_visible_child_name("list");
            }
            Err(e) => {
                self.error_page.set_description(Some(&glib::markup_escape_text(&e.message())));
                self.stack.set_visible_child_name("error");
            }
        }
    }

    fn row(self: &Rc<Self>, vault: VaultInfo) -> adw::ActionRow {
        let unlocked = vault.is_unlocked();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&vault.name))
            .subtitle(if unlocked { "Unlocked" } else { "Locked" })
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(if unlocked {
            "changes-allow-symbolic"
        } else {
            "changes-prevent-symbolic"
        }));
        let button = |label: &str, css: Option<&str>| {
            let b = gtk::Button::builder()
                .label(label)
                .use_underline(true)
                .valign(gtk::Align::Center)
                .build();
            if let Some(c) = css {
                b.add_css_class(c);
            }
            b
        };
        let vault = Rc::new(vault);
        let gear = gtk::Button::builder()
            .icon_name("emblem-system-symbolic")
            .tooltip_text("Settings")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        gear.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[strong]
            vault,
            move |_| {
                let (client, vault, parent) = (this.client.clone(), (*vault).clone(), this.widget());
                glib::spawn_future_local(settings::show(client, vault, parent));
            }
        ));
        if unlocked {
            let open = button("_Open", None);
            open.connect_clicked(glib::clone!(
                #[weak(rename_to = this)]
                self,
                #[strong]
                vault,
                move |_| this.open(&vault.mount_point)
            ));
            row.set_activatable_widget(Some(&open));
            row.add_suffix(&open);

            let lock = button("_Lock", None);
            lock.connect_clicked(glib::clone!(
                #[weak(rename_to = this)]
                self,
                #[strong]
                vault,
                move |_| {
                    let vault = vault.clone();
                    glib::spawn_future_local(async move {
                        unlock::lock(&this.client, &vault.id, &vault.name, Some(&this.widget())).await;
                    });
                }
            ));
            row.add_suffix(&lock);
        } else {
            let unlock_button = button("_Unlock", Some("suggested-action"));
            unlock_button.connect_clicked(glib::clone!(
                #[weak(rename_to = this)]
                self,
                #[strong]
                vault,
                move |_| {
                    let vault = vault.clone();
                    glib::spawn_future_local(async move {
                        if let Ok(true) = unlock::unlock(&this.client, &vault.id, Some(&this.widget())).await {
                            this.refresh().await;
                        }
                    });
                }
            ));
            row.set_activatable_widget(Some(&unlock_button));
            row.add_suffix(&unlock_button);
        }
        row.add_suffix(&gear);
        row
    }

    fn create(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            if dialogs::create_vault(&this.client, &this.widget()).await.is_some() {
                this.refresh().await;
            }
        });
    }

    /// Opens the unlocked vault in the default file manager.
    fn open(&self, mount_point: &str) {
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(mount_point)));
        let window = self.window.clone();
        launcher.launch(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let Err(e) = r {
                let message = e.to_string();
                glib::spawn_future_local(async move {
                    dialogs::show_error("Cannot Open Folder", &message, Some(window.upcast_ref())).await;
                });
            }
        });
    }
}
