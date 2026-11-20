#!/usr/bin/env bash
# Build the source tarball, the vendored-crates tarball, the SRPM and the RPM.
#   packaging/build-rpm.sh [outdir]      (default: ./rpmbuild-out)
#   packaging/build-rpm.sh --sources DIR  only the two tarballs, into DIR;
#                                         prints the source tarball path last
#                                         (Packit's create-archive action)
# Packs the working tree as git sees it (tracked + new files, .gitignore
# respected), so a local run builds what you see. Needs: git, cargo,
# rpm-build, cargo-rpm-macros.
set -euo pipefail
cd "$(dirname "$0")/.."
sources_only=0
if [[ ${1:-} == --sources ]]; then
  sources_only=1
  sources=$(realpath -m "${2:?--sources needs a directory}")
else
  out=$(realpath -m "${1:-rpmbuild-out}")
  sources=$out/SOURCES
fi
name=cloister
version=$(sed -n 's/^Version:[[:space:]]*//p' packaging/$name.spec)
cargo_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
meson_version=$(sed -n "s/^  version: '\(.*\)',/\1/p" meson.build)
if [[ "$version" != "$cargo_version" || "$version" != "$meson_version" ]]; then
  echo "version mismatch: spec=$version Cargo.toml=$cargo_version meson.build=$meson_version" >&2
  exit 1
fi

if ((sources_only)); then
  mkdir -p "$sources"
else
  rm -rf "$out" && mkdir -p "$out"/{SOURCES,SPECS,SRPMS,RPMS,BUILD}
fi
index=$(mktemp)
trap 'rm -f "$index"' EXIT
cp "$(git rev-parse --git-dir)/index" "$index"
GIT_INDEX_FILE=$index git add -A
tree=$(GIT_INDEX_FILE=$index git write-tree)
git archive --format=tar.gz --prefix="$name-$version/" -o "$sources/$name-$version.tar.gz" "$tree"

# Vendor exactly what Cargo.lock pins (--locked), into vendor/ at the root
# of the second tarball (unpacked inside the source tree by %autosetup -a1).
vend=$(mktemp -d)
trap 'rm -f "$index"; rm -rf "$vend"' EXIT
cargo vendor --locked --versioned-dirs --quiet "$vend/vendor" >/dev/null
tar -C "$vend" -cJf "$sources/$name-$version-vendor.tar.xz" vendor
if ((sources_only)); then
  echo "$sources/$name-$version.tar.gz"
  exit 0
fi

cp packaging/$name.spec "$out/SPECS/"
rpmbuild --define "_topdir $out" -ba "$out/SPECS/$name.spec"
find "$out/SRPMS" "$out/RPMS" -name '*.rpm' -printf '%p\n'
