#!/usr/bin/env bash
# Spike 4: does logind LockedHint follow the screen lock? Run as your user
# (no sudo needed), lock the screen (Super+L), unlock, then Ctrl+C.
# Expect lines "LockedHint=yes" then "LockedHint=no".
set -euo pipefail
user=${SUDO_USER:-$USER}
sid=$(loginctl show-user "$user" -p Display --value)
echo "watching graphical session $sid of $user ($(loginctl show-session "$sid" -p Type --value))"
last=
while :; do
  now=$(loginctl show-session "$sid" -p LockedHint)
  [[ "$now" != "$last" ]] && echo "$(date +%T) $now" && last=$now
  sleep 0.5
done
