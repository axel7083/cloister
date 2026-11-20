# Testing Cloister

Three layers. Each one catches bugs the others cannot.

| Layer | Where | What it proves |
|---|---|---|
| Rust tests | `cargo test` (pr-check, every PR) | logic of every crate; real gocryptfs + FUSE mounts; D-Bus contracts over peer-to-peer connections |
| Real-broker tests | `dbus-run-session cargo test -- --ignored` (pr-check) | the same services through a real D-Bus broker (fd passing, names) |
| System tests | `tests/e2e/` in a Fedora VM (e2e, after pr-check; nightly on rawhide) | the **installed RPM** on a real system: systemd, logind, polkit, dbus-broker, GVfs, SELinux enforcing |

## Running locally

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
CLOISTER_REQUIRE_FUSE=1 cargo test --workspace          # FUSE tests fail instead of skipping
dbus-run-session -- cargo test --workspace -- --ignored --skip host_
meson setup _build && meson test -C _build               # desktop file, polkit, D-Bus policy, unit checks

# System tests (needs qemu-kvm; about 10 minutes)
packaging/build-rpm.sh                                   # or: download the `dist` artifact of a CI run
cargo build --release -p cloister-testkit
ci/vm/run-e2e.sh --image "$(ci/vm/fetch-image.sh 45)" \
  --rpms rpmbuild-out/RPMS/x86_64 --testkit target/release
#   --only '20-*'  runs one script, --keep leaves the VM running for debugging
```

`ci/vm/run-e2e.sh` boots a throw-away VM (`qemu -snapshot`), installs the RPMs
and runs every `tests/e2e/[0-9]*.sh` from the host over ssh, as `root`,
`tester` (owns the vaults) and `other` (must not see them). A long-lived
`tester` login plays the desktop session that logind elects as the user's
"Display", so `SetLockedHint` exercises the real screen-lock path.
Logs and the VM journal land in `e2e-artifacts/`.

Stand-ins, because CI has no screen and no finger:
- **polkit rule** `/etc/polkit-1/rules.d/90-cloister-e2e.rules` answers YES/NO
  in place of the fingerprint dialog (`30-keys.sh`);
- **`cloister-fake-prompt`** (`crates/cloister-testkit`, never packaged) owns
  the app's bus name and answers `Prompt1.Unlock` in place of the password
  dialog, so `gio mount -d <vault-id>` goes through the real GIO client, GVfs
  proxy and `cloister-gvfs-monitor` (`40-gvfs.sh`);
- **`cloister-gio-unmount <vault-id>`** unmounts through the volume monitor's
  `GMount`, like Nautilus's eject button. `gio mount -u file://…` would not
  reach our monitor: GIO resolves file paths with the native unix-mount
  monitor only.

## Manual release checklist

Not automatable; check on a real GNOME session before tagging:

- [ ] Sidebar shows the folder with the closed-lock icon; clicking opens the
      password dialog; the folder opens after unlocking; the icon changes.
- [ ] Eject button and right-click → Unmount lock it.
- [ ] Fingerprint: turn it on in the app's settings, then unlock from the
      sidebar with a touch (GNOME's polkit dialog).
- [ ] Super+L locks an unlocked folder; suspend/resume locks it too.
- [ ] The app: create a folder, wrong password message, "Lock Anyway" when a
      file is open, settings dialog, remove from list.
