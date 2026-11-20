//! Integration tests against the real gocryptfs. Skipped when gocryptfs or
//! /dev/fuse is unavailable, unless CLOISTER_REQUIRE_FUSE=1 (CI).

use std::fs;

use cloister_core::testing::require_fuse;
use cloister_core::{secret_from_vec, Error, SecretBytes, Unlock};

fn secret(s: &str) -> SecretBytes {
    secret_from_vec(s.as_bytes().to_vec())
}

macro_rules! require_gocryptfs {
    () => {
        match require_fuse() {
            Some(g) => g,
            None => return,
        }
    };
}

#[test]
fn init_mount_unmount_with_passphrase_and_master_key() {
    let g = require_gocryptfs!();
    let tmp = tempfile::tempdir().unwrap();
    let cipher = tmp.path().join("cipher");
    let mnt = tmp.path().join("run/mnt");
    let pass = secret("correct horse battery staple");

    g.init(&cipher, &pass).unwrap();
    assert!(cipher.join("gocryptfs.conf").is_file());

    let mut h = g
        .mount(
            &cipher,
            &mnt,
            Unlock::Passphrase(secret("correct horse battery staple")),
        )
        .unwrap();
    assert!(h.is_alive());
    fs::write(mnt.join("note.txt"), "hello").unwrap();
    h.unmount(false).unwrap();
    assert!(!cloister_core::mountinfo::is_mounted(&mnt).unwrap());
    assert!(!mnt.join("note.txt").exists());
    h.unmount(false).unwrap(); // idempotent

    let key = g.dump_master_key(&cipher, &pass).unwrap();
    let h = g.mount(&cipher, &mnt, Unlock::MasterKey(key)).unwrap();
    assert_eq!(fs::read_to_string(mnt.join("note.txt")).unwrap(), "hello");
    drop(h); // Drop unmounts
    assert!(!cloister_core::mountinfo::is_mounted(&mnt).unwrap());
}

#[test]
fn wrong_passphrase_is_reported() {
    let g = require_gocryptfs!();
    let tmp = tempfile::tempdir().unwrap();
    let cipher = tmp.path().join("cipher");
    let mnt = tmp.path().join("mnt");
    g.init(&cipher, &secret("right")).unwrap();

    assert!(matches!(
        g.mount(&cipher, &mnt, Unlock::Passphrase(secret("wrong"))),
        Err(Error::WrongPassphrase)
    ));
    assert!(matches!(
        g.dump_master_key(&cipher, &secret("wrong")),
        Err(Error::WrongPassphrase)
    ));
    assert!(!cloister_core::mountinfo::is_mounted(&mnt).unwrap());
}

#[test]
fn busy_vault_refuses_non_lazy_unmount() {
    let g = require_gocryptfs!();
    let tmp = tempfile::tempdir().unwrap();
    let cipher = tmp.path().join("cipher");
    let mnt = tmp.path().join("mnt");
    g.init(&cipher, &secret("pw")).unwrap();
    let mut h = g
        .mount(&cipher, &mnt, Unlock::Passphrase(secret("pw")))
        .unwrap();

    let open = fs::File::create(mnt.join("open.txt")).unwrap();
    assert!(matches!(h.unmount(false), Err(Error::Busy)));
    assert!(
        cloister_core::mountinfo::is_mounted(&mnt).unwrap(),
        "still mounted after Busy"
    );
    drop(open);
    h.unmount(false).unwrap();
}

#[test]
fn refuses_unsafe_inputs() {
    let g = require_gocryptfs!();
    let tmp = tempfile::tempdir().unwrap();
    let cipher = tmp.path().join("cipher");

    for bad in ["", "two\nlines", "nul\0byte"] {
        assert!(
            matches!(
                g.init(&cipher, &secret(bad)),
                Err(Error::InvalidPassphrase(_))
            ),
            "{bad:?}"
        );
    }
    fs::create_dir(&cipher).unwrap();
    fs::write(cipher.join("existing"), "x").unwrap();
    assert!(matches!(
        g.init(&cipher, &secret("pw")),
        Err(Error::NotEmpty(_))
    ));

    let cipher2 = tmp.path().join("cipher2");
    let mnt = tmp.path().join("mnt");
    g.init(&cipher2, &secret("pw")).unwrap();
    fs::create_dir(&mnt).unwrap();
    fs::write(mnt.join("leftover"), "x").unwrap();
    assert!(matches!(
        g.mount(&cipher2, &mnt, Unlock::Passphrase(secret("pw"))),
        Err(Error::NotEmpty(_))
    ));
}
