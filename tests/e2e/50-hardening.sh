#!/usr/bin/env bash
# Runs last: sandbox score, no denial during the whole run, clean removal.
source "$(dirname "$0")/lib.sh"

check "key service sandbox exposure at most 1.0" root 'systemd-analyze security --no-pager --threshold=10 cloister-keyd.service >/dev/null'
check "no SELinux denial during the whole run" no_avc
check "dnf remove leaves no installed file behind" root '
dnf remove -y -q cloister >/dev/null
! rpm -q cloister
for f in /usr/bin/cloister /usr/bin/cloister-ctl /usr/libexec/cloister-keyd /usr/libexec/cloister-session \
         /usr/libexec/cloister-gvfs-monitor /usr/share/polkit-1/actions/io.github.axel7083.Cloister.policy \
         /usr/share/gvfs/remote-volume-monitors/cloister.monitor; do
  [ ! -e "$f" ] || { echo "left behind: $f"; exit 1; }
done'
finish
