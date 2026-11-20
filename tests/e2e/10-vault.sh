#!/usr/bin/env bash
# Create, unlock, lock; encryption on disk; busy handling.
source "$(dirname "$0")/lib.sh"
desktop_session >/dev/null
uid=$(tester_uid)
id=$(create_vault E2E-vault)

check "vault created" test -n "$id"
check "wrong passphrase is refused and the vault stays locked" tsh <<'EOF2'
! printf wrong | cloister-ctl unlock E2E-vault --passphrase-stdin
cloister-ctl list | grep E2E-vault | grep -q $'\tlocked\t'
EOF2
mp=$(tsh PASS="$PASS" <<<'printf %s "$PASS" | cloister-ctl unlock E2E-vault --passphrase-stdin')
check "unlocks under the runtime dir" test "$mp" = "/run/user/$uid/cloister/$id"
check "is a FUSE mount, private to tester (0700)" tsh MP="$mp" <<'EOF2'
mountpoint -q "$MP"
[ "$(stat -c %a "$MP")" = 700 ] && [ "$(stat -c %a "$(dirname "$MP")")" = 700 ]
EOF2
check "other users cannot enter it" as_other "! ls $mp"
check "files can be written" tsh MP="$mp" <<<'echo TOP-SECRET-E2E > "$MP/secret-plan.txt"'
check "lock is refused while a file is open, vault stays usable" tsh MP="$mp" <<'EOF2'
exec 3<"$MP/secret-plan.txt"
! cloister-ctl lock E2E-vault
exec 3<&-
grep -q TOP-SECRET-E2E "$MP/secret-plan.txt"
EOF2
check "lock unmounts" tsh MP="$mp" <<<'cloister-ctl lock E2E-vault && ! mountpoint -q "$MP"'
check "nothing readable on disk: no plaintext name or content" tsh ID="$id" <<'EOF2'
dir=$HOME/.local/share/cloister/$ID
test -f "$dir/gocryptfs.conf"
! grep -rq TOP-SECRET-E2E "$dir"
! find "$dir" | grep -q secret-plan
EOF2
check "unlocking again gives the file back" tsh PASS="$PASS" MP="$mp" <<'EOF2'
printf %s "$PASS" | cloister-ctl unlock E2E-vault --passphrase-stdin >/dev/null
grep -q TOP-SECRET-E2E "$MP/secret-plan.txt"
EOF2
check "forced lock works with a file open" tsh MP="$mp" <<'EOF2'
exec 3<"$MP/secret-plan.txt"
cloister-ctl lock --force E2E-vault
EOF2
check "and the mount is gone" wait_until 15 is_locked "$uid" "$id"
finish
