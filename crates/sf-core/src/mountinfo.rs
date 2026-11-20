//! Minimal reader for `/proc/self/mountinfo`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Whether `path` is currently a mount point (exact match, no symlink resolution).
pub fn is_mounted(path: &Path) -> io::Result<bool> {
    Ok(mount_points(&fs::read_to_string("/proc/self/mountinfo")?).any(|p| p == path))
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
    fn root_is_mounted() {
        assert!(is_mounted(Path::new("/")).unwrap());
    }
}
