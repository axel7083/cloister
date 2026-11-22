# Helpers for the end-to-end tests. Sourced by tests/e2e/[0-9]*.sh, which
# ci/vm/run-e2e.sh runs on the host against the VM.
set -uo pipefail
: "${E2E_SSH:?run the tests through ci/vm/run-e2e.sh}"

failures=0
PASS=correct-horse-e2e

# Run a command (string) in the VM as root / tester / other.
root()     { $E2E_SSH "root@$E2E_HOST" "$@"; }
as_other() { $E2E_SSH "other@$E2E_HOST" "$@"; }

# Run a script read from stdin as tester; NAME=value arguments are exported
# first. Each call is a separate ssh login (its own logind session).
tsh() {
  { for kv in "$@"; do printf 'export %q\n' "$kv"; done; echo 'set -euo pipefail'; cat; } |
    $E2E_SSH "tester@$E2E_HOST" bash -s
}

# check DESCRIPTION COMMAND... : TAP line, keeps going on failure.
check() {
  local desc=$1 out
  shift
  if out=$("$@" 2>&1); then
    echo "ok - $desc"
  else
    echo "not ok - $desc"
    sed 's/^/#   /' <<<"$out"
    failures=$((failures + 1))
  fi
}

# wait_until SECONDS COMMAND...
wait_until() {
  local deadline=$((SECONDS + $1))
  shift
  until "$@" >/dev/null 2>&1; do
    ((SECONDS < deadline)) || return 1
    sleep 0.2
  done
}

# A long-lived tester login standing in for the desktop session: logind
# elects it as the user's "Display", which auto-lock watches.
desktop_session() {
  if [[ -z $(root "loginctl show-user tester -p Display --value 2>/dev/null") ]]; then
    $E2E_SSH -f "tester@$E2E_HOST" 'exec sleep infinity' </dev/null >/dev/null 2>&1
    wait_until 10 root '[ -n "$(loginctl show-user tester -p Display --value)" ]'
  fi
  root "loginctl show-user tester -p Display --value"
}

session_path() { root "busctl call org.freedesktop.login1 /org/freedesktop/login1 org.freedesktop.login1.Manager GetSession s $1" | awk '{print $2}' | tr -d '"'; }

tester_uid() { root "id -u tester"; }

# is_mounted UID VAULT-ID (checked as root, independent of tester's session)
is_mounted() { root "grep -q ' /run/user/$1/cloister/$2 ' /proc/self/mountinfo"; }
is_locked() { ! is_mounted "$@"; }

# monitor_logged TEXT: cloister-gvfs-monitor (tester's) logged TEXT
monitor_logged() { tsh <<<'journalctl --user -u cloister-gvfs-monitor -q --no-pager -o cat' | grep -qF "$1"; }

# SELinux denials that involve Cloister or what it drives.
# Denials involving our components. Match after "avc:" only: journal lines
# start with the hostname (cloister-e2e), which would match everything.
no_avc() {
  local avcs
  avcs=$(root "{ journalctl -b -q --no-pager -g 'avc: +denied' 2>/dev/null; ausearch -m AVC,USER_AVC,SELINUX_ERR -ts boot 2>/dev/null; } | grep -Ei '(avc:|SELINUX_ERR).*(cloister|gocryptfs|fusermount|polkit|dbus-broker|gvfs)' || true")
  [[ -z $avcs ]] || { echo "$avcs"; return 1; }
}

# create_vault NAME -> prints its id (tester's vault, passphrase $PASS)
create_vault() { tsh PASS="$PASS" N="$1" <<<'printf %s "$PASS" | cloister-ctl create "$N" --passphrase-stdin'; }

finish() {
  if ((failures)); then
    echo "# $failures check(s) failed"
    exit 1
  fi
}
