#!/usr/bin/env bash
# Automatic locking: screen lock (LockedHint) and end of the session.
source "$(dirname "$0")/lib.sh"
display=$(desktop_session)
uid=$(tester_uid)
id=$(create_vault E2E-lock)
unlock() { tsh PASS="$PASS" <<<'printf %s "$PASS" | cloister-ctl unlock E2E-lock --passphrase-stdin >/dev/null'; }

check "session service runs without NoNewPrivs (fusermount3 needs setuid)" tsh <<'EOF2'
pid=$(systemctl --user show -p MainPID --value cloister-session)
grep -Eq '^NoNewPrivs:\s+0$' /proc/$pid/status
EOF2

check "unlock" unlock
path=$(session_path "$display")
root "busctl call org.freedesktop.login1 $path org.freedesktop.login1.Session SetLockedHint b true"
check "screen lock (LockedHint) locks within 5 seconds" wait_until 5 is_locked "$uid" "$id"
root "busctl call org.freedesktop.login1 $path org.freedesktop.login1.Session SetLockedHint b false"

check "unlock again after the screen is unlocked" unlock
# Keep a second login so the user manager (and cloister-session) survives:
# this exercises the session-removed trigger, not a service shutdown.
$E2E_SSH -f "tester@$E2E_HOST" 'exec sleep infinity' </dev/null >/dev/null 2>&1
root "loginctl terminate-session $display"
check "ending the graphical session locks" wait_until 15 is_locked "$uid" "$id"
check "the session service is still running" tsh <<<'systemctl --user is-active cloister-session'

root "loginctl terminate-user tester" || true
finish
