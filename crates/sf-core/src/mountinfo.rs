//! Minimal reader for `/proc/self/mountinfo`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Whether `path` is currently a mount point (exact match, no symlink resolution).
pub fn is_mounted(path: &Path) -> io::Result<bool> {
    Ok(mount_points(&fs::read_to_string("/proc/self/mountinfo")?).any(|p| p == path))
}

/// One line of mountinfo, reduced to what we use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub mount_point: PathBuf,
    pub fs_type: String,
    /// Mount source, e.g. `/dev/mapper/luks-…` or `tmpfs`.
    pub source: String,
}

/// The mount holding `path` (longest mount point prefix; `path` should be
/// canonical).
pub fn find_mount(path: &Path) -> io::Result<Option<MountEntry>> {
    Ok(longest_prefix(entries(&fs::read_to_string("/proc/self/mountinfo")?), path))
}

fn longest_prefix(entries: impl Iterator<Item = MountEntry>, path: &Path) -> Option<MountEntry> {
    entries
        .filter(|e| path.starts_with(&e.mount_point))
        // The last of equally long matches is the one on top (mounted later).
        .fold(None, |best: Option<MountEntry>, e| match best {
            Some(b) if b.mount_point.as_os_str().len() > e.mount_point.as_os_str().len() => Some(b),
            _ => Some(e),
        })
}

fn entries(mountinfo: &str) -> impl Iterator<Item = MountEntry> + '_ {
    mountinfo.lines().filter_map(|l| {
        let (left, right) = l.split_once(" - ")?;
        let mount_point = unescape(left.split(' ').nth(4)?);
        let mut r = right.split(' ');
        let fs_type = r.next()?.to_owned();
        let source = unescape(r.next()?).to_string_lossy().into_owned();
        Some(MountEntry { mount_point, fs_type, source })
    })
}

/// Mount points (5th field), with the kernel's octal escapes (`\040`…) decoded.
fn mount_points(mountinfo: &str) -> impl Iterator<Item = PathBuf> + '_ {
    mountinfo
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .map(unescape)
}

fn unescape(field: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = (b[i] == b'\\' && i + 3 < b.len())
            .then(|| &b[i + 1..i + 4])
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)))
            .and_then(|d| u8::try_from(d.iter().fold(0u32, |acc, c| acc * 8 + u32::from(c - b'0'))).ok());
        if let Some(byte) = octal {
            out.push(byte);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_escapes() {
        let info = "36 35 98:0 / /mnt/My\\040Vault rw,noatime master:1 - fuse.gocryptfs /c rw\n\
                    37 35 98:0 / /run/user/1000 rw - tmpfs tmpfs rw\n";
        let points: Vec<_> = mount_points(info).collect();
        assert_eq!(points, [PathBuf::from("/mnt/My Vault"), PathBuf::from("/run/user/1000")]);
    }

    #[test]
    fn keeps_invalid_escapes_verbatim() {
        assert_eq!(unescape("/a\\777b\\04"), PathBuf::from("/a\\777b\\04"));
    }

    #[test]
    fn finds_longest_prefix() {
        let info = "1 0 0:1 / / rw - btrfs /dev/mapper/luks-x rw\n\
                    2 1 0:2 / /var rw - ext4 /dev/sda2 rw\n\
                    3 1 0:3 / /var/lib/x rw - tmpfs tmpfs rw\n";
        let m = longest_prefix(entries(info), Path::new("/var/lib/secure-folder")).unwrap();
        assert_eq!((m.mount_point.to_str().unwrap(), m.source.as_str()), ("/var", "/dev/sda2"));
        let m = longest_prefix(entries(info), Path::new("/home/me")).unwrap();
        assert_eq!((m.fs_type.as_str(), m.source.as_str()), ("btrfs", "/dev/mapper/luks-x"));
    }

    #[test]
    fn root_is_mounted() {
        assert!(is_mounted(Path::new("/")).unwrap());
    }
}
