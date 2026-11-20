//! Key storage: `<root>/<uid>/<vault-id>.key`, files 0600, directories 0700,
//! owned by the service user. Keys are protected at rest by file
//! permissions only, hence the full-disk encryption requirement.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use sf_core::paths::create_private_dir_all;
use sf_core::secret::read_limited;
use sf_core::{ExposeSecret, SecretBytes, VaultId};
use sf_dbus::MAX_KEY_BYTES;

pub struct KeyStore {
    root: PathBuf,
}

impl KeyStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Stores (or replaces) a key atomically.
    pub fn put(&self, uid: u32, id: &VaultId, key: &SecretBytes) -> io::Result<()> {
        let dir = self.root.join(uid.to_string());
        create_private_dir_all(&dir)?;
        let tmp = dir.join(format!(".{id}.tmp"));
        match fs::remove_file(&tmp) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        f.write_all(key.expose_secret())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, self.path(uid, id))?;
        File::open(&dir)?.sync_all()
    }

    pub fn get(&self, uid: u32, id: &VaultId) -> io::Result<Option<SecretBytes>> {
        match File::open(self.path(uid, id)) {
            Ok(f) => read_limited(f, MAX_KEY_BYTES).map(Some),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn contains(&self, uid: u32, id: &VaultId) -> bool {
        self.path(uid, id).is_file()
    }

    /// Deletes a key; `false` if there was none.
    pub fn remove(&self, uid: u32, id: &VaultId) -> io::Result<bool> {
        match fs::remove_file(self.path(uid, id)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `VaultId` only renders as a canonical UUID: no path traversal possible.
    fn path(&self, uid: u32, id: &VaultId) -> PathBuf {
        self.root.join(uid.to_string()).join(format!("{id}.key"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::secret_from_vec;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn put_get_remove_scoped_by_uid() {
        let tmp = tempfile::tempdir().unwrap();
        let store = KeyStore::new(tmp.path());
        let id = VaultId::generate();
        store.put(1000, &id, &secret_from_vec(b"k1".to_vec())).unwrap();

        assert_eq!(store.get(1000, &id).unwrap().unwrap().expose_secret(), b"k1");
        assert!(store.get(1001, &id).unwrap().is_none(), "other uid sees nothing");
        assert!(!store.contains(1001, &id));

        store.put(1000, &id, &secret_from_vec(b"k2".to_vec())).unwrap();
        assert_eq!(store.get(1000, &id).unwrap().unwrap().expose_secret(), b"k2");

        let file = tmp.path().join("1000").join(format!("{id}.key"));
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);

        assert!(store.remove(1000, &id).unwrap());
        assert!(!store.remove(1000, &id).unwrap());
        assert!(store.get(1000, &id).unwrap().is_none());
    }
}
