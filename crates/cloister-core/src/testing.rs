//! Helpers for tests that need the real gocryptfs and FUSE. Not part of the
//! public API.

use crate::Gocryptfs;

/// Environment variable that turns a "FUSE unavailable" skip into a
/// failure. CI sets it so integration tests can never silently not run.
pub const REQUIRE_FUSE_ENV: &str = "CLOISTER_REQUIRE_FUSE";

/// The real driver when gocryptfs and `/dev/fuse` are usable, else `None`
/// (the caller returns early). Panics instead when [`REQUIRE_FUSE_ENV`] is `1`.
pub fn require_fuse() -> Option<Gocryptfs> {
    let g = Gocryptfs::default();
    let usable = g.is_available() && std::path::Path::new("/dev/fuse").exists();
    if usable {
        return Some(g);
    }
    if std::env::var(REQUIRE_FUSE_ENV).as_deref() == Ok("1") {
        panic!("{REQUIRE_FUSE_ENV}=1 but gocryptfs or /dev/fuse is not available");
    }
    eprintln!(
        "skipped: gocryptfs or /dev/fuse not available (set {REQUIRE_FUSE_ENV}=1 to fail instead)"
    );
    None
}
