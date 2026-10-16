#!/usr/bin/env bash
# Build the two sources of openvxe.spec from the current checkout and, when
# rpmbuild is available, the RPMs:
#   openvxe-<version>.tar.gz          git archive of HEAD
#   openvxe-<version>-vendor.tar.xz   crates vendored with cargo vendor
# Usage: packaging/fedora/make-sources.sh [--srpm-only]
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
version="$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)"
rev="$(git -C "$root" rev-parse --short=7 HEAD)"
name="openvxe-$version"

git -C "$root" archive --format=tar.gz --prefix="$name/" -o "$here/$name.tar.gz" HEAD

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xzf "$here/$name.tar.gz" -C "$work"
(cd "$work/$name" && cargo vendor --quiet --locked --versioned-dirs vendor >/dev/null)
tar -cJf "$here/$name-vendor.tar.xz" -C "$work/$name" vendor
echo "Sources written to $here"

if ! command -v rpmbuild >/dev/null; then
    echo "rpmbuild not found: install rpm-build, then rerun this script."
    exit 0
fi

mode=-ba
[[ "${1:-}" == "--srpm-only" ]] && mode=-bs
rpmbuild "$mode" \
    --define "_sourcedir $here" \
    --define "_topdir $here/rpmbuild" \
    --define "gitrev $rev" \
    "$here/openvxe.spec"
echo "Packages written to $here/rpmbuild"
