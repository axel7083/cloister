#!/usr/bin/env bash
# Boot a throw-away Fedora VM (KVM), install Cloister's RPMs and run the
# end-to-end tests against it. The image is never modified (qemu -snapshot).
#
#   ci/vm/run-e2e.sh --image IMG.qcow2 --rpms DIR --testkit DIR [--keep] [--only GLOB]
#
# Tests (tests/e2e/[0-9]*.sh) run on this host and drive the VM over ssh
# as root, "tester" and "other". Logs land in $E2E_ARTIFACTS (default ./e2e-artifacts).
set -euo pipefail
cd "$(dirname "$0")/../.."
repo=$PWD

image= rpms= testkit= keep=0 only='[0-9]*.sh'
while (($#)); do
  case $1 in
    --image) image=$(realpath "$2"); shift 2 ;;
    --rpms) rpms=$(realpath "$2"); shift 2 ;;
    --testkit) testkit=$(realpath "$2"); shift 2 ;;
    --keep) keep=1; shift ;;
    --only) only=$2; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -f $image && -d $rpms && -x $testkit/cloister-fake-prompt && -x $testkit/cloister-gio-unmount ]] || { echo "need --image, --rpms and --testkit" >&2; exit 2; }
artifacts=$(realpath -m "${E2E_ARTIFACTS:-e2e-artifacts}")
mkdir -p "$artifacts"

work=$(mktemp -d)
qemu_pid=
cleanup() {
  if [[ -n $qemu_pid ]]; then
    if ((keep)); then
      echo "VM kept: ssh -i $work/id -p $port root@127.0.0.1  (kill $qemu_pid when done)"
      return
    fi
    kill "$qemu_pid" 2>/dev/null || true
  fi
  cp -f "$work/console.log" "$artifacts/" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# --- cloud-init seed: root, tester, other, all reachable with one key ------
ssh-keygen -q -t ed25519 -N '' -f "$work/id"
pub=$(cat "$work/id.pub")
cat > "$work/user-data" <<YAML
#cloud-config
disable_root: false
ssh_pwauth: false
users:
  - name: root
    ssh_authorized_keys: ["$pub"]
  - name: tester
    lock_passwd: true
    ssh_authorized_keys: ["$pub"]
  - name: other
    lock_passwd: true
    ssh_authorized_keys: ["$pub"]
YAML
printf 'instance-id: cloister-e2e\nlocal-hostname: cloister-e2e\n' > "$work/meta-data"
if command -v cloud-localds >/dev/null; then
  cloud-localds "$work/seed.iso" "$work/user-data" "$work/meta-data"
elif command -v genisoimage >/dev/null; then
  genisoimage -quiet -output "$work/seed.iso" -volid cidata -joliet -rock "$work/user-data" "$work/meta-data"
else
  xorriso -as mkisofs -quiet -o "$work/seed.iso" -V cidata -J -r "$work/user-data" "$work/meta-data"
fi

# --- boot -----------------------------------------------------------------
port=$(( 20000 + RANDOM % 20000 ))
qemu-system-x86_64 -machine accel=kvm -cpu host -m 4096 -smp 2 \
  -drive file="$image",if=virtio,snapshot=on \
  -drive file="$work/seed.iso",media=cdrom,readonly=on \
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$port-:22 -device virtio-net-pci,netdev=n0 \
  -display none -serial file:"$work/console.log" \
  -daemonize -pidfile "$work/qemu.pid"
qemu_pid=$(cat "$work/qemu.pid")

export E2E_SSH="ssh -q -i $work/id -p $port -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 -o BatchMode=yes"
export E2E_SCP="scp -q -i $work/id -P $port -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes"
export E2E_HOST=127.0.0.1 E2E_ARTIFACTS=$artifacts
vm() { $E2E_SSH root@127.0.0.1 "$@"; }

echo "waiting for the VM (ssh on port $port)…"
for _ in $(seq 120); do vm true 2>/dev/null && break; sleep 3; done
vm true || { echo "VM did not come up" >&2; exit 1; }
vm cloud-init status --wait >/dev/null || true

# --- install ----------------------------------------------------------------
vm mkdir -p /tmp/e2e/rpms
$E2E_SCP "$rpms"/*.rpm root@127.0.0.1:/tmp/e2e/rpms/
$E2E_SCP "$testkit"/cloister-fake-prompt "$testkit"/cloister-gio-unmount root@127.0.0.1:/usr/local/bin/
vm "rm -f /tmp/e2e/rpms/*.src.rpm /tmp/e2e/rpms/*debug*.rpm"
# auditd first, so every SELinux denial from our install on is recorded.
vm "dnf install -y -q gvfs audit >/dev/null && systemctl start auditd"
vm "dnf install -y -q /tmp/e2e/rpms/*.rpm >/dev/null && restorecon /usr/local/bin/cloister-*"
vm "rpm -q cloister gocryptfs gvfs; getenforce"

# --- tests ------------------------------------------------------------------
failed=()
for t in "$repo"/tests/e2e/$only; do
  echo "=== $(basename "$t")"
  if ! timeout --kill-after=30 900 bash "$t" 2>&1 | tee "$artifacts/$(basename "$t" .sh).log"; then
    failed+=("$(basename "$t")")
  fi
done
vm "journalctl -b --no-pager" > "$artifacts/journal.log" 2>/dev/null || true

if ((${#failed[@]})); then
  echo "FAILED: ${failed[*]}" >&2
  exit 1
fi
echo "all end-to-end tests passed"
