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
| `sf-gvfs-monitor` | you | GVfs session | sidebar volume for every GIO file manager |
| Nautilus extension | inside Nautilus | — | "Lock" menu item |
| `secure-folder-ctl` | you | per command | scripting, other file managers |

Front-ends only talk to the public D-Bus API (`io.github.axel7083.SecureFolder.Vaults1`
and `.Prompt1`); they contain no vault logic.

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
