//! Is a path stored on an encrypted (dm-crypt / LUKS) block device?
//! Reads only /proc and /sys, so it works with `PrivateDevices=yes`.

use std::fs;
use std::path::Path;

/// Best effort: `false` when unknown.
pub fn is_on_encrypted_disk(path: &Path) -> bool {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let Ok(Some(mount)) = cloister_core::mountinfo::find_mount(&canonical) else {
        return false;
    };
    match block_name(&mount.source) {
        Some(name) => is_crypt(&name, 0),
        None => {
            tracing::warn!(source = %mount.source, "cannot tell whether the key store is encrypted");
            false
        }
    }
}

/// `/dev/mapper/NAME` → `dm-N`; `/dev/NAME` → `NAME`.
fn block_name(source: &str) -> Option<String> {
    if let Some(mapped) = source.strip_prefix("/dev/mapper/") {
        return fs::read_dir("/sys/class/block")
            .ok()?
            .flatten()
            .find_map(|e| {
                let name = fs::read_to_string(e.path().join("dm/name")).ok()?;
                (name.trim() == mapped).then(|| e.file_name().to_string_lossy().into_owned())
            });
    }
    source.strip_prefix("/dev/").map(|n| n.replace('/', "!"))
}

/// A dm-crypt device, or a device stacked on one (LVM on LUKS…).
fn is_crypt(name: &str, depth: u8) -> bool {
    if depth > 8 {
        return false;
    }
    let dev = Path::new("/sys/class/block").join(name);
    if let Ok(uuid) = fs::read_to_string(dev.join("dm/uuid")) {
        if uuid.starts_with("CRYPT-") {
            return true;
        }
    }
    let Ok(slaves) = fs::read_dir(dev.join("slaves")) else {
        return false;
    };
    slaves
        .flatten()
        .any(|s| is_crypt(&s.file_name().to_string_lossy(), depth + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_sources_are_not_encrypted() {
        assert_eq!(block_name("tmpfs"), None);
        assert_eq!(block_name("/dev/nvme0n1p2").as_deref(), Some("nvme0n1p2"));
        assert!(!is_crypt("definitely-not-a-device", 0));
    }
}

#[cfg(test)]
mod host_tests {
    /// Prints what the host reports; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "depends on the host disk layout"]
    fn host_var_lib() {
        println!(
            "/var/lib encrypted: {}",
            super::is_on_encrypted_disk(std::path::Path::new("/var/lib"))
        );
        println!(
            "/boot encrypted: {}",
            super::is_on_encrypted_disk(std::path::Path::new("/boot"))
        );
    }
}
