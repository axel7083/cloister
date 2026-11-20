//! Who is calling, and may they? Identity comes from the bus daemon (never
//! from arguments); authorization from polkit, with user interaction so the
//! desktop's agent asks for the fingerprint or the login password.

use std::collections::HashMap;

use futures_util::future::BoxFuture;
use sf_dbus::{Error, ACTION_ENROLL, ACTION_REVOKE, ACTION_UNLOCK};
use zbus::names::OwnedUniqueName;
use zbus::{proxy, Connection, Variant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Unlock,
    Enroll,
    Revoke,
}

impl Action {
    pub fn id(self) -> &'static str {
        match self {
            Self::Unlock => ACTION_UNLOCK,
            Self::Enroll => ACTION_ENROLL,
            Self::Revoke => ACTION_REVOKE,
        }
    }
}

pub trait Gate: Send + Sync + 'static {
    fn caller_uid(&self, sender: Option<OwnedUniqueName>) -> BoxFuture<'_, Result<u32, Error>>;
    fn authorize(&self, sender: Option<OwnedUniqueName>, action: Action) -> BoxFuture<'_, Result<(), Error>>;
}

#[proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    #[allow(clippy::type_complexity)]
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, Variant<'_>>),
        action_id: &str,
        details: HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

const ALLOW_USER_INTERACTION: u32 = 1;

/// Production gate: the system bus daemon and polkit.
pub struct BusGate {
    conn: Connection,
}

impl BusGate {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

fn require(sender: Option<OwnedUniqueName>) -> Result<OwnedUniqueName, Error> {
    sender.ok_or_else(|| Error::Denied("caller has no bus name".into()))
}

impl Gate for BusGate {
    fn caller_uid(&self, sender: Option<OwnedUniqueName>) -> BoxFuture<'_, Result<u32, Error>> {
        Box::pin(async move {
            let sender = require(sender)?;
            let dbus = zbus::fdo::DBusProxy::new(&self.conn).await?;
            dbus.get_connection_unix_user(sender.as_ref().into())
                .await
                .map_err(|e| Error::Denied(format!("cannot identify caller: {e}")))
        })
    }

    fn authorize(&self, sender: Option<OwnedUniqueName>, action: Action) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let sender = require(sender)?;
            let authority = AuthorityProxy::new(&self.conn).await?;
            let subject = ("system-bus-name", HashMap::from([("name", Variant::from(sender.as_str()))]));
            let (authorized, _challenge, _details) = authority
                .check_authorization(&subject, action.id(), HashMap::new(), ALLOW_USER_INTERACTION, "")
                .await
                .map_err(|e| Error::Failed(format!("polkit: {e}")))?;
            if authorized {
                Ok(())
            } else {
                Err(Error::Denied("authentication failed or cancelled".into()))
            }
        })
    }
}
