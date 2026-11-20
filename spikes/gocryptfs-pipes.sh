#!/usr/bin/env bash
# Spike 2: secrets reach gocryptfs only through pipes (never argv/env).
set -euo pipefail

work=$(mktemp -d)
trap 'fusermount3 -u "$work/plain" 2>/dev/null || true; rm -rf "$work"' EXIT
mkdir "$work/cipher" "$work/plain"
pass='correct horse battery staple'

echo "== init with -passfile /dev/stdin"
printf '%s' "$pass" | gocryptfs -q -init -passfile /dev/stdin "$work/cipher"

echo "== dump master key with gocryptfs-xray (password on stdin)"
mk=$(printf '%s\n' "$pass" | gocryptfs-xray -dumpmasterkey "$work/cipher/gocryptfs.conf" 2>/dev/null | tail -n1)
[[ "$mk" =~ ^[0-9a-f-]{64,}$ ]] && echo "ok: got master key (${#mk} chars)" || { echo "FAIL: unexpected output"; exit 1; }

echo "== mount with -masterkey=stdin"
printf '%s\n' "$mk" | gocryptfs -q -masterkey=stdin "$work/cipher" "$work/plain"
echo hello > "$work/plain/test.txt"
fusermount3 -u "$work/plain"

echo "== mount with -passfile /dev/stdin, foreground child (-fg)"
printf '%s' "$pass" | gocryptfs -q -fg -passfile /dev/stdin "$work/cipher" "$work/plain" &
pid=$!
for _ in $(seq 50); do mountpoint -q "$work/plain" && break; sleep 0.1; done
grep -q hello "$work/plain/test.txt" && echo "ok: read back file"
fusermount3 -u "$work/plain"; wait "$pid"
echo "ok: foreground process exited after unmount"
