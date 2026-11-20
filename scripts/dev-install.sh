#!/usr/bin/env bash
# Install a meson build under /usr for development, then make the system
# services pick it up. Run as root:  sudo scripts/dev-install.sh [builddir]
#
# Why the extra steps: `meson install` copies files from your home directory
# with their home SELinux label and only relabels them at the end. polkitd and
# dbus-broker notice the new files in between, are denied reading them, and
# drop them until restarted/reloaded.
set -euo pipefail
build=${1:-build}
[[ $EUID -eq 0 ]] || { echo "run as root" >&2; exit 1; }

meson install -C "$build" --no-rebuild
if command -v restorecon >/dev/null; then
  restorecon -R /usr/share/polkit-1/actions /usr/share/dbus-1 /usr/lib/systemd /usr/lib/sysusers.d /usr/libexec /usr/bin
fi
systemd-sysusers
systemctl daemon-reload
systemctl restart polkit
systemctl reload dbus-broker
pkaction --action-id io.github.axel7083.SecureFolder.unlock >/dev/null && echo "polkit actions: ok"
echo "installed; log out and back in (or 'systemctl --user daemon-reload') for user units"
