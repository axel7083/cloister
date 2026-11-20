//! secure-folder: create Secure Folders and unlock them.

mod application;
mod client;
mod dialogs;
mod runtime;
mod unlock;
mod window;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SF_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    application::SecureFolderApp::new().run()
}
