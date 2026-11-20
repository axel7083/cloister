#!/usr/bin/env bash
# sf-session.service must not set options that imply NoNewPrivileges or a
# private mount namespace (see the comment in the unit).
set -euo pipefail
unit=$1
forbidden='^(NoNewPrivileges|SystemCallFilter|SystemCallArchitectures|SystemCallLog|LockPersonality|RestrictRealtime|RestrictNamespaces|RestrictAddressFamilies|RestrictSUIDSGID|MemoryDenyWriteExecute|ProtectSystem|ProtectHome|PrivateTmp|PrivateDevices|PrivateUsers|PrivateMounts|ProtectKernel[A-Za-z]*|ProtectControlGroups|ProtectClock|ProtectHostname|ProtectProc|ReadOnlyPaths|ReadWritePaths|InaccessiblePaths|TemporaryFileSystem|BindPaths|BindReadOnlyPaths)='
if grep -En "$forbidden" "$unit"; then
  echo "error: $unit sets an option that breaks fusermount3 or hides mounts" >&2
  exit 1
fi
