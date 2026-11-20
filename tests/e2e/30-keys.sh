#!/usr/bin/env bash
# Fingerprint unlock path: key service + polkit. A test polkit rule plays
# the role of the user touching the sensor.
source "$(dirname "$0")/lib.sh"
desktop_session >/dev/null
uid=$(tester_uid)
id=$(create_vault E2E-keys)
rule=/etc/polkit-1/rules.d/90-cloister-e2e.rules
polkit() { # YES or NO for tester on our actions
  root "cat > $rule" <<EOF2
polkit.addRule(function (action, subject) {
  if (action.id.indexOf("io.github.axel7083.Cloister.") === 0 && subject.user === "tester")
    return polkit.Result.$1;
});
EOF2
  sleep 1 # polkitd reloads rules on change
}

polkit YES
check "turn fingerprint unlock on" tsh PASS="$PASS" <<<'printf %s "$PASS" | cloister-ctl fingerprint-on E2E-keys --passphrase-stdin'
check "list shows it" tsh <<<'cloister-ctl list | grep E2E-keys | grep -q fingerprint'
check "key stored 0600, owned by cloister, store not world-readable" root \
  "[ \"\$(stat -c '%U %a' /var/lib/cloister/$uid/$id.key)\" = 'cloister 600' ] && [ \"\$(stat -c %a /var/lib/cloister)\" = 700 ]"
check "unlock with the stored key" tsh <<<'cloister-ctl unlock E2E-keys --fingerprint >/dev/null'
check "lock" tsh <<<'cloister-ctl lock E2E-keys'

polkit NO
check "authentication refused: Denied, vault stays locked" tsh <<'EOF2'
! cloister-ctl unlock E2E-keys --fingerprint 2>/tmp/err
grep -qi 'authentication failed' /tmp/err
cloister-ctl list | grep E2E-keys | grep -q $'\tlocked\t'
EOF2
check "another user cannot see or release tester's key" as_other \
  "busctl --system call io.github.axel7083.Cloister.KeyService /io/github/axel7083/Cloister/KeyService io.github.axel7083.Cloister.KeyService1 IsEnrolled s $id | grep -qx 'b false'"

polkit YES
check "turn fingerprint unlock off deletes the key" tsh <<<'cloister-ctl fingerprint-off E2E-keys'
check "key file is gone" root "! test -e /var/lib/cloister/$uid/$id.key"
root "rm -f $rule"
finish
