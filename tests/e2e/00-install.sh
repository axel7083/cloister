#!/usr/bin/env bash
# The RPM installs a working system part: user, polkit, D-Bus, SELinux.
source "$(dirname "$0")/lib.sh"

check "SELinux is enforcing" root '[ "$(getenforce)" = Enforcing ]'
check "system user cloister exists without a login shell" root 'getent passwd cloister | grep -q nologin'
check "polkit knows the unlock, enroll and revoke actions" root \
  'for a in unlock enroll revoke; do pkaction --action-id io.github.axel7083.Cloister.$a >/dev/null || exit 1; done'
check "installed files carry their SELinux labels" root \
  'ls -Z /usr/share/polkit-1/actions/io.github.axel7083.Cloister.policy /usr/share/dbus-1/system.d/io.github.axel7083.Cloister.KeyService.conf | grep -vq user_home_t'
check "key service starts on demand, as user cloister (not root)" root \
  'busctl --system call io.github.axel7083.Cloister.KeyService /io/github/axel7083/Cloister/KeyService io.github.axel7083.Cloister.KeyService1 SystemDiskEncrypted >/dev/null && [ "$(ps -o user= -C cloister-keyd)" = cloister ]'
check "GVfs knows our volume monitor" root 'test -f /usr/share/gvfs/remote-volume-monitors/cloister.monitor'
check "no SELinux denial so far" no_avc
finish
