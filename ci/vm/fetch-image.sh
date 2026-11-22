#!/usr/bin/env bash
# Download and verify a Fedora Cloud Base image; prints its path.
#   ci/vm/fetch-image.sh 45|rawhide [cache-dir]
# 45 is pinned by SHA-256; rawhide resolves today's compose and checks it
# against that compose's CHECKSUM file.
set -euo pipefail
release=${1:?usage: fetch-image.sh 45|rawhide [cache-dir]}
cache=${2:-${XDG_CACHE_HOME:-$HOME/.cache}/cloister-e2e}
mirror=https://download.fedoraproject.org/pub/fedora/linux
mkdir -p "$cache"

case "$release" in
  45)
    name=Fedora-Cloud-Base-Generic-45-1.4.x86_64.qcow2
    url=$mirror/releases/45/Cloud/x86_64/images/$name
    sha=50bdc5e70f6a24f540932d15411c1c6ae9fcfa40fb990d876d83c5cc823b2ad0
    ;;
  rawhide)
    # download.fedoraproject.org redirects every request to a random mirror,
    # and mirrors sync at different times: resolve one mirror with the
    # listing and fetch the CHECKSUM and the image from that same mirror.
    dir=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$mirror/development/rawhide/Cloud/x86_64/images/")
    dir=${dir%/}
    listing=$(curl -fsSL "$dir/")
    name=$(grep -oE 'Fedora-Cloud-Base-Generic-Rawhide-[0-9.n]+\.x86_64\.qcow2' <<<"$listing" | sort -u | tail -1)
    sums=$(grep -oE 'Fedora-Cloud-images-Rawhide-x86_64-[0-9.n]+-CHECKSUM' <<<"$listing" | sort -u | tail -1)
    url=$dir/$name
    sha=$(curl -fsSL "$dir/$sums" | sed -n "s/^SHA256 ($name) = //p")
    ;;
  *) echo "unknown release: $release" >&2; exit 2 ;;
esac
[[ -n "$name" && ${#sha} -eq 64 ]] || { echo "cannot resolve image for $release" >&2; exit 1; }

image=$cache/$name
if [[ ! -f "$image" ]] || ! echo "$sha  $image" | sha256sum -c --status; then
  curl -fsSL --retry 3 -o "$image.part" "$url"
  echo "$sha  $image.part" | sha256sum -c --status || { echo "checksum mismatch for $name" >&2; exit 1; }
  mv "$image.part" "$image"
fi
echo "$image"
