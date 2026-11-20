//! Wire format of GVfs' private `org.gtk.Private.RemoteVolumeMonitor`
//! (gvfs/monitor/proxy/dbus-interfaces.xml, gvfsproxyvolumemonitordaemon.c).
//! The struct signatures below are frozen by GVfs and checked by tests.

use std::collections::HashMap;

use serde::Serialize;
use sf_dbus::VaultInfo;
use zbus::{DBusError, OwnedVariant, Type, Variant};

pub const OBJECT_PATH: &str = "/org/gtk/Private/RemoteVolumeMonitor";
/// Must equal `DBusName=` in our `.monitor` file: clients compare it with
/// the first argument of every signal.
pub const BUS_NAME: &str = "io.github.axel7083.SecureFolder.VolumeMonitor";

const ICON_LOCKED: &str = ". GThemedIcon io.github.axel7083.SecureFolder-locked-symbolic changes-prevent-symbolic";
const ICON_UNLOCKED: &str = ". GThemedIcon io.github.axel7083.SecureFolder-unlocked-symbolic changes-allow-symbolic";
const ICON_COLOR: &str = ". GThemedIcon io.github.axel7083.SecureFolder folder";

/// `(ssssbbbbbbbbuasa{ss}sa{sv})`: we expose no drives, only the type.
#[derive(Debug, Serialize, Type)]
pub struct Drive {
    id: String,
    name: String,
    icon: String,
    symbolic_icon: String,
    can_eject: bool,
    can_poll_for_media: bool,
    has_media: bool,
    is_media_removable: bool,
    is_media_check_automatic: bool,
    can_start: bool,
    can_start_degraded: bool,
    can_stop: bool,
    start_stop_type: u32,
    volume_ids: Vec<String>,
    identifiers: HashMap<String, String>,
    sort_key: String,
    expansion: HashMap<String, OwnedVariant>,
}

/// `(ssssssbbssa{ss}sa{sv})`
#[derive(Debug, Serialize, Type)]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub symbolic_icon: String,
    pub uuid: String,
    /// Empty: clients then call our `VolumeMount` instead of mounting a URI.
    pub activation_uri: String,
    pub can_mount: bool,
    pub should_automount: bool,
    /// Empty: no drive, so Nautilus lists the volume on its own in the sidebar.
    pub drive_id: String,
    pub mount_id: String,
    pub identifiers: HashMap<String, String>,
    pub sort_key: String,
    pub expansion: HashMap<String, Variant<'static>>,
}

/// `(ssssssbsassa{sv})`
#[derive(Debug, Serialize, Type)]
pub struct Mount {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub symbolic_icon: String,
    pub uuid: String,
    pub root_uri: String,
    pub can_unmount: bool,
    pub volume_id: String,
    pub x_content_types: Vec<String>,
    pub sort_key: String,
    pub expansion: HashMap<String, Variant<'static>>,
}

pub fn volume_id(vault_id: &str) -> String {
    format!("volume-{vault_id}")
}

pub fn mount_id(vault_id: &str) -> String {
    format!("mount-{vault_id}")
}

/// Vault id from a volume or mount id we generated.
pub fn vault_id_of(object_id: &str) -> Option<&str> {
    object_id.strip_prefix("volume-").or_else(|| object_id.strip_prefix("mount-"))
}

fn sort_key(v: &VaultInfo) -> String {
    format!("secure-folder-{}", v.name.to_lowercase())
}

pub fn volume(v: &VaultInfo) -> Volume {
    let unlocked = v.is_unlocked();
    Volume {
        id: volume_id(&v.id),
        name: v.name.clone(),
        icon: ICON_COLOR.into(),
        symbolic_icon: if unlocked { ICON_UNLOCKED } else { ICON_LOCKED }.into(),
        uuid: v.id.clone(),
        activation_uri: String::new(),
        can_mount: true,
        should_automount: false,
        drive_id: String::new(),
        mount_id: if unlocked { mount_id(&v.id) } else { String::new() },
        identifiers: HashMap::from([("uuid".to_owned(), v.id.clone())]),
        sort_key: sort_key(v),
        expansion: HashMap::from([("always-call-mount".to_owned(), Variant::from(true))]),
    }
}

/// `None` while locked.
pub fn mount(v: &VaultInfo) -> Option<Mount> {
    v.is_unlocked().then(|| Mount {
        id: mount_id(&v.id),
        name: v.name.clone(),
        icon: ICON_COLOR.into(),
        symbolic_icon: ICON_UNLOCKED.into(),
        uuid: v.id.clone(),
        root_uri: file_uri(&v.mount_point),
        can_unmount: true,
        volume_id: volume_id(&v.id),
        x_content_types: Vec::new(),
        sort_key: sort_key(v),
        expansion: HashMap::new(),
    })
}

/// `file://` URI with RFC 3986 percent-encoding of the path.
pub fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// GIO errors as GDBus encodes an unregistered `G_IO_ERROR` domain
/// (`g_dbus_error_encode_gerror`); the client turns them back into `GError`s.
#[derive(Debug, DBusError)]
#[zbus(prefix = "org.gtk.GDBus.UnmappedGError.Quark._g_2dio_2derror_2dquark")]
pub enum GioError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// G_IO_ERROR_FAILED: shown to the user.
    Code0(String),
    /// G_IO_ERROR_NOT_SUPPORTED
    Code15(String),
    /// G_IO_ERROR_BUSY
    Code26(String),
    /// G_IO_ERROR_FAILED_HANDLED: the user already saw a dialog; stay silent.
    Code30(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(state: &str) -> VaultInfo {
        VaultInfo {
            id: "67e55044-10b1-426f-9247-bb680e5fe0c8".into(),
            name: "My Vault".into(),
            state: state.into(),
            mount_point: if state == "unlocked" { "/run/user/1000/secure-folder/a b".into() } else { String::new() },
        }
    }

    #[test]
    fn signatures_match_gvfs() {
        assert_eq!(Drive::SIGNATURE.to_string(), "(ssssbbbbbbbbuasa{ss}sa{sv})");
        assert_eq!(Volume::SIGNATURE.to_string(), "(ssssssbbssa{ss}sa{sv})");
        assert_eq!(Mount::SIGNATURE.to_string(), "(ssssssbsassa{sv})");
    }

    #[test]
    fn gio_error_names() {
        use zbus::DBusError as _;
        assert_eq!(
            GioError::Code30(String::new()).name().as_str(),
            "org.gtk.GDBus.UnmappedGError.Quark._g_2dio_2derror_2dquark.Code30"
        );
    }

    #[test]
    fn locked_volume_has_no_mount() {
        let v = volume(&info("locked"));
        assert_eq!((v.mount_id.as_str(), v.activation_uri.as_str(), v.drive_id.as_str()), ("", "", ""));
        assert!(v.symbolic_icon.contains("locked-symbolic"));
        assert!(mount(&info("locked")).is_none());
    }

    #[test]
    fn unlocked_volume_links_its_mount() {
        let v = volume(&info("unlocked"));
        let m = mount(&info("unlocked")).unwrap();
        assert_eq!(v.mount_id, m.id);
        assert_eq!(m.volume_id, v.id);
        assert_eq!(m.root_uri, "file:///run/user/1000/secure-folder/a%20b");
        assert_eq!(vault_id_of(&m.id), Some("67e55044-10b1-426f-9247-bb680e5fe0c8"));
        assert_eq!(vault_id_of(&v.id), vault_id_of(&m.id));
        assert_eq!(vault_id_of("other"), None);
    }
}
