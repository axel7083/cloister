//! cloister: create encrypted folders and unlock them.

mod application;
mod client;
mod dialogs;
mod runtime;
mod settings;
mod unlock;
mod window;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("CLOISTER_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    application::CloisterApp::new().run()
}
