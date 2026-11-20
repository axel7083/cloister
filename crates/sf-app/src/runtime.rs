//! Bridge between the GTK main loop and zbus (tokio).
//!
//! D-Bus work runs on a small tokio runtime; GTK code awaits the result from
//! its own executor (`glib::spawn_future_local`), so widgets never leave the
//! main thread.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("dbus")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Runs `future` on the tokio runtime and awaits its output from any executor.
pub async fn on_tokio<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(future).await.expect("D-Bus task panicked")
}

/// Starts `future` on the tokio runtime without waiting for it.
pub fn spawn_tokio<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    runtime().spawn(future);
}
