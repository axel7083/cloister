# Secure Folder

An encrypted folder for GNOME that lives in your file manager: it shows in the
sidebar with a lock icon, unlocks with your vault password or your fingerprint,
and locks itself when you lock the screen, suspend or log out.

Status: early development (MVP in progress).

## Components

| Component | Runs as | Lifetime | Concern |
|---|---|---|---|
| `sf-session` | you (user unit) | on demand, stays while a vault is unlocked | mounts vaults (gocryptfs), auto-lock |
| `sf-keyd` | `secure-folder` system user (not root) | on demand, exits when idle | stores vault keys, releases them after polkit (fingerprint / login password) |
| `secure-folder` | you | on demand | create vaults, password prompt, settings |
| `sf-gvfs-monitor` | you | GVfs session | sidebar volume for every GIO file manager: click to unlock, eject (or right-click → Unmount) to lock |
| `secure-folder-ctl` | you | per command | scripting, other file managers |

Front-ends only talk to the public D-Bus API (crate `sf-dbus`); they contain no
vault logic:

| Bus | Bus name | Object path | Interface |
|---|---|---|---|
| session | `io.github.axel7083.SecureFolder.Session` | `/io/github/axel7083/SecureFolder/Session` | `…Vaults1` |
| session | `io.github.axel7083.SecureFolder` (the app) | `/io/github/axel7083/SecureFolder` | `…Prompt1` |
| system | `io.github.axel7083.SecureFolder.KeyService` | `/io/github/axel7083/SecureFolder/KeyService` | `…KeyService1` (used by `sf-session` only) |

Passphrases always travel as file descriptors, never inside D-Bus messages.

Automatic locking relies on logind only (works on any desktop that sets
`LockedHint`): screen lock, suspend (a sleep delay inhibitor is held while a vault
is unlocked) and end of the graphical session. If the watcher cannot start,
unlocking is refused.

## Fingerprint unlock

Optional, per vault (app: gear button → *Unlock with Fingerprint*;
CLI: `secure-folder-ctl fingerprint-on VAULT`). After checking the vault
password, `sf-session` hands the vault master key to `sf-keyd`. Each unlock
then goes through polkit (`io.github.axel7083.SecureFolder.unlock`,
`auth_self`): GNOME's dialog accepts your fingerprint or login password.
The key travels only between `sf-keyd` and `sf-session`, as a pipe that the
caller provides; front-ends never see it.

## Installing for development

```sh
meson setup build --prefix=/usr --buildtype=release && meson compile -C build
sudo scripts/dev-install.sh build      # installs, relabels (SELinux), reloads polkit/D-Bus
# remove: sudo ninja -C build uninstall && sudo userdel secure-folder && sudo rm -rf /var/lib/secure-folder
```

## Trying it without installing

```sh
cargo build
target/debug/sf-session &          # normally started by D-Bus activation
target/debug/secure-folder-ctl create "Secure Folder"
target/debug/secure-folder-ctl unlock "Secure Folder"   # prints the mount point
target/debug/secure-folder-ctl lock "Secure Folder"
```

## Threat model

Protects against:
- someone reading the vault while it is locked: after a reboot, a screen lock,
  a suspend or a logout;
- other local user accounts.

Does **not** protect against:
- malware running as your user while a vault is unlocked (it can read the files,
  like any other program you run);
- root.

Fingerprint unlock stores the vault master key on this computer, readable only by
the `secure-folder` system user and root. That is only as safe as your disk:
**enable full-disk encryption** if you turn fingerprint unlock on. The vault
password path does not have this limitation.

## Building

```sh
sudo dnf install rust cargo meson gocryptfs gtk4-devel libadwaita-devel nautilus-python
meson setup build && meson compile -C build && meson test -C build
```

## License

GPL-3.0-or-later
