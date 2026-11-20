//! The GtkApplication. Besides the window, it serves
//! `io.github.axel7083.Cloister.Prompt1` on its own bus name, so file
//! manager integrations can ask for an unlock without opening the window
//! (the app is D-Bus activatable and quits when idle).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use cloister_dbus::{Error, APP_BUS_NAME, ERROR_PREFIX, PROMPT_INTERFACE};
use gtk::{gio, glib};

use crate::client::Client;
use crate::unlock;
use crate::window::VaultsWindow;

const PROMPT_XML: &str = r#"
<node>
  <interface name="io.github.axel7083.Cloister.Prompt1">
    <method name="Unlock">
      <arg type="s" name="id" direction="in"/>
      <arg type="b" name="unlocked" direction="out"/>
    </method>
  </interface>
</node>"#;

/// Quit this long after the last window or prompt closed (service mode).
const INACTIVITY_TIMEOUT_MS: u32 = 10_000;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct CloisterApp {
        pub client: RefCell<Option<Client>>,
        pub window: RefCell<Option<Rc<VaultsWindow>>>,
        /// Unlock prompts in progress, with every caller waiting on each.
        pub pending: RefCell<HashMap<String, Vec<gio::DBusMethodInvocation>>>,
        pub registration: RefCell<Option<gio::RegistrationId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CloisterApp {
        const NAME: &'static str = "CloisterApplication";
        type Type = super::CloisterApp;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for CloisterApp {}

    impl ApplicationImpl for CloisterApp {
        fn activate(&self) {
            self.parent_activate();
            let app = self.obj().clone();
            glib::spawn_future_local(async move { app.show_window().await });
        }

        fn dbus_register(
            &self,
            connection: &gio::DBusConnection,
            object_path: &str,
        ) -> Result<(), glib::Error> {
            self.parent_dbus_register(connection, object_path)?;
            let info = gio::DBusNodeInfo::for_xml(PROMPT_XML)?
                .lookup_interface(PROMPT_INTERFACE)
                .expect("interface in XML");
            let app = self.obj().downgrade();
            let id = connection
                .register_object(object_path, &info)
                .method_call(move |_, _, _, _, method, params, invocation| {
                    let Some(app) = app.upgrade() else { return };
                    match (method, params.get::<(String,)>()) {
                        ("Unlock", Some((id,))) => app.prompt_unlock(id, invocation),
                        _ => invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.InvalidArgs",
                            "bad call",
                        ),
                    }
                })
                .build()?;
            self.registration.replace(Some(id));
            Ok(())
        }

        fn dbus_unregister(&self, connection: &gio::DBusConnection, object_path: &str) {
            if let Some(id) = self.registration.take() {
                let _ = connection.unregister_object(id);
            }
            self.parent_dbus_unregister(connection, object_path);
        }
    }

    impl GtkApplicationImpl for CloisterApp {}
    impl AdwApplicationImpl for CloisterApp {}
}

glib::wrapper! {
    pub struct CloisterApp(ObjectSubclass<imp::CloisterApp>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl CloisterApp {
    pub fn new() -> Self {
        let app: Self = glib::Object::builder()
            .property("application-id", APP_BUS_NAME)
            .property("flags", gio::ApplicationFlags::empty())
            .build();
        app.set_inactivity_timeout(INACTIVITY_TIMEOUT_MS);
        app
    }

    /// Connects to the session service once, then reuses the connection.
    async fn client(&self) -> Result<Client, Error> {
        if let Some(c) = self.imp().client.borrow().clone() {
            return Ok(c);
        }
        let c = Client::connect().await?;
        self.imp().client.replace(Some(c.clone()));
        Ok(c)
    }

    async fn show_window(&self) {
        if let Some(w) = self.imp().window.borrow().as_ref() {
            w.present();
            return;
        }
        match self.client().await {
            Ok(client) => {
                let window = VaultsWindow::new(self.upcast_ref(), client);
                let app = self.downgrade();
                window.connect_close(move || {
                    if let Some(app) = app.upgrade() {
                        app.imp().window.take();
                    }
                });
                window.present();
                self.imp().window.replace(Some(window));
            }
            Err(e) => {
                let _hold = self.hold();
                crate::dialogs::show_error("Cloister Service Unavailable", &e.message(), None)
                    .await;
            }
        }
    }

    /// `Prompt1.Unlock`: concurrent calls for one vault share one prompt.
    fn prompt_unlock(&self, id: String, invocation: gio::DBusMethodInvocation) {
        {
            let mut pending = self.imp().pending.borrow_mut();
            if let Some(waiters) = pending.get_mut(&id) {
                waiters.push(invocation);
                return;
            }
            pending.insert(id.clone(), vec![invocation]);
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            let _hold = app.hold(); // keep running while the prompt is open
            let parent = app.active_window().map(|w| w.upcast::<gtk::Widget>());
            let result = match app.client().await {
                Ok(client) => unlock::unlock(&client, &id, parent.as_ref()).await,
                Err(e) => Err(e),
            };
            let waiters = app
                .imp()
                .pending
                .borrow_mut()
                .remove(&id)
                .unwrap_or_default();
            for invocation in waiters {
                match &result {
                    Ok(unlocked) => invocation.return_value(Some(&(*unlocked,).to_variant())),
                    Err(e) => {
                        let name = format!("{ERROR_PREFIX}.{}", e.kind().unwrap_or("Failed"));
                        invocation.return_dbus_error(&name, &e.message());
                    }
                }
            }
        });
    }
}
