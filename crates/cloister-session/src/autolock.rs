//! Automatic locking policy, desktop-neutral: it only relies on logind.
//!
//! - the graphical session's `LockedHint` turns true (screen locked);
//! - `PrepareForSleep(true)` (suspend / hibernate), backed by a "delay"
//!   inhibitor held while a vault is unlocked so we unmount before sleeping;
//! - the graphical session is removed (logout while other sessions remain).

use futures_util::StreamExt;
use zbus::{proxy, Connection, OwnedFd, OwnedObjectPath};

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    #[zbus(name = "GetUserByPID")]
    async fn get_user_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    async fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
    #[zbus(signal)]
    async fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn session_removed(
        &self,
        session_id: &str,
        object_path: OwnedObjectPath,
    ) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.User",
    default_service = "org.freedesktop.login1"
)]
trait User {
    #[zbus(property)]
    fn display(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    ScreenLocked,
    Sleep,
    SessionEnded,
    /// The logind watcher stopped: fail closed.
    WatcherLost,
}

/// A logind observation, decoupled from D-Bus so the policy is testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogindEvent {
    LockedHint(bool),
    PrepareForSleep(bool),
    SessionRemoved(String),
}

/// The policy: which logind events lock every vault.
pub fn lock_reason(event: &LogindEvent, display_session: Option<&str>) -> Option<LockReason> {
    match event {
        LogindEvent::LockedHint(true) => Some(LockReason::ScreenLocked),
        LogindEvent::PrepareForSleep(true) => Some(LockReason::Sleep),
        LogindEvent::SessionRemoved(id) if Some(id.as_str()) == display_session => {
            Some(LockReason::SessionEnded)
        }
        _ => None,
    }
}

pub struct AutoLock {
    manager: ManagerProxy<'static>,
    display_id: Option<String>,
    display: Option<SessionProxy<'static>>,
}

impl AutoLock {
    /// Resolves our user's graphical session. Fails if logind is unreachable.
    pub async fn connect(system: &Connection) -> zbus::Result<Self> {
        let manager = ManagerProxy::new(system).await?;
        let user_path = manager.get_user_by_pid(std::process::id()).await?;
        let user = UserProxy::builder(system).path(user_path).build().await?;
        let (id, path) = user.display().await?;
        let (display_id, display) = if id.is_empty() {
            tracing::warn!("no graphical session: screen-lock trigger disabled");
            (None, None)
        } else {
            let session = SessionProxy::builder(system).path(path).build().await?;
            (Some(id), Some(session))
        };
        Ok(Self {
            manager,
            display_id,
            display,
        })
    }

    /// Takes a sleep "delay" inhibitor; sleep waits (up to logind's
    /// `InhibitDelayMaxSec`) until the returned fd is closed.
    pub async fn inhibit_sleep(&self) -> zbus::Result<OwnedFd> {
        self.manager
            .inhibit(
                "sleep",
                "Cloister",
                "Lock encrypted folders before sleeping",
                "delay",
            )
            .await
    }

    /// Streams lock reasons until a logind stream ends (returns an error then:
    /// the caller must treat auto-lock as unavailable).
    /// `on_armed` is called once every subscription is in place.
    pub async fn run(
        &self,
        tx: tokio::sync::mpsc::Sender<LockReason>,
        on_armed: impl FnOnce(),
    ) -> zbus::Result<()> {
        let mut sleep = self.manager.receive_prepare_for_sleep().await?;
        let mut removed = self.manager.receive_session_removed().await?;
        let mut locked = match &self.display {
            Some(s) => Some(s.receive_locked_hint_changed().await),
            None => None,
        };
        on_armed();
        loop {
            let event = tokio::select! {
                Some(s) = sleep.next() => LogindEvent::PrepareForSleep(s.args()?.start),
                Some(r) = removed.next() => LogindEvent::SessionRemoved(r.args()?.session_id.to_owned()),
                Some(l) = next_locked(&mut locked) => LogindEvent::LockedHint(l.get().await?),
                else => return Err(zbus::Error::Failure("logind signal stream ended".into())),
            };
            tracing::debug!(?event, "logind");
            if let Some(reason) = lock_reason(&event, self.display_id.as_deref()) {
                if tx.send(reason).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

async fn next_locked<S: futures_util::Stream + Unpin>(s: &mut Option<S>) -> Option<S::Item> {
    match s {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        let d = Some("2");
        assert_eq!(
            lock_reason(&LogindEvent::LockedHint(true), d),
            Some(LockReason::ScreenLocked)
        );
        assert_eq!(lock_reason(&LogindEvent::LockedHint(false), d), None);
        assert_eq!(
            lock_reason(&LogindEvent::PrepareForSleep(true), d),
            Some(LockReason::Sleep)
        );
        assert_eq!(lock_reason(&LogindEvent::PrepareForSleep(false), d), None);
        assert_eq!(
            lock_reason(&LogindEvent::SessionRemoved("2".into()), d),
            Some(LockReason::SessionEnded)
        );
        assert_eq!(
            lock_reason(&LogindEvent::SessionRemoved("7".into()), d),
            None
        );
        assert_eq!(
            lock_reason(&LogindEvent::SessionRemoved("2".into()), None),
            None
        );
    }
}
