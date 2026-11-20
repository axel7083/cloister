#!/usr/bin/env bash
# The sidebar volume through the real GIO client: gio mount -l/-d/-u go
# through GVfs' proxy to cloister-gvfs-monitor. cloister-fake-prompt
# stands in for the app's password dialog.
source "$(dirname "$0")/lib.sh"
desktop_session >/dev/null
uid=$(tester_uid)
id=$(create_vault E2E-gvfs)
mp=/run/user/$uid/cloister/$id
fake_prompt() { # extra systemd-run --setenv arguments
  tsh PASS="$PASS" ARGS="$*" <<'EOF2'
printf %s "$PASS" > ~/.e2e-pass
systemctl --user stop fake-prompt 2>/dev/null || true
systemd-run --user -q --unit=fake-prompt --setenv=CLOISTER_TEST_PASSPHRASE_FILE=$HOME/.e2e-pass $ARGS /usr/local/bin/cloister-fake-prompt
sleep 1
EOF2
}

check "gio lists the vault, from our volume monitor" tsh <<'EOF2'
gio mount -li | grep -A12 "Volume(.*): E2E-gvfs" | grep -q GProxyVolumeMonitorCloister
EOF2
check "locked icon" tsh <<<'gio mount -li | grep -A12 "Volume(.*): E2E-gvfs" | grep -q Cloister-locked-symbolic'

fake_prompt --setenv=CLOISTER_TEST_CANCEL=1
tsh ID="$id" <<<'gio mount -d "$ID" || true'
check "cancelled prompt leaves it locked" is_locked "$uid" "$id"

fake_prompt
check "gio mount -d <uuid> unlocks through Prompt1" tsh ID="$id" MP="$mp" <<<'gio mount -d "$ID" && mountpoint -q "$MP"'
check "volume now has a mount with the open-lock icon" tsh <<<'gio mount -li | grep -A20 "Volume(.*): E2E-gvfs" | grep -q Cloister-unlocked-symbolic'
check "the monitor handled VolumeMount" monitor_logged VolumeMount

tsh MP="$mp" <<<'echo data > "$MP/f"'
check "unmount is refused while a file is open" tsh ID="$id" MP="$mp" <<'EOF2'
exec 3<"$MP/f"
rc=0; cloister-gio-unmount "$ID" || rc=$?
[ "$rc" = 1 ]  # a GIO error from our monitor, not 2 = mount not found
mountpoint -q "$MP"
EOF2
check "unmount through the volume monitor (Nautilus eject) locks" tsh ID="$id" MP="$mp" <<<'cloister-gio-unmount "$ID" && ! mountpoint -q "$MP"'
check "through our MountUnmount (not a raw FUSE unmount)" monitor_logged MountUnmount

tsh <<<'systemctl --user stop fake-prompt; rm -f ~/.e2e-pass'
finish
