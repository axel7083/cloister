use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Last time a client called us; drives the idle exit.
#[derive(Clone)]
pub struct Activity(Arc<Mutex<Instant>>);

impl Activity {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    pub fn touch(&self) {
        *self.0.lock().expect("activity poisoned") = Instant::now();
    }
    pub fn idle_for(&self) -> Duration {
        self.0.lock().expect("activity poisoned").elapsed()
    }
}
