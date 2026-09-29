#!/usr/bin/env bash
# Check out the git2-rs fork beside gwz-core at the commit .github/git2-rs.commit
# pins, with its vendored libgit2 submodule. gwz-core depends on it by path
# (`../git2-rs`, package `gwz-git2`), so every job that runs cargo on gwz-core,
# or on a crate that depends on it by path, needs it there.
#
# Usage: checkout-git2-rs.sh [DIR]
# Clones into DIR/git2-rs. DIR defaults to the directory that holds the gwz-core
# checkout this script lives in, which is `..` from gwz-core's root in either CI
# layout (gwz-core at the workspace root, or beside gwz-cli or gwz-py). Refuses
# to touch an existing git2-rs, so a local run never clobbers a real checkout.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core="$(dirname "$here")"
parent="${1:-$(dirname "$core")}"
dest="$parent/git2-rs"
commit="$(grep -m1 -E '^[0-9a-f]{40}$' "$here/git2-rs.commit")"
if [ -e "$dest" ]; then
  echo "checkout-git2-rs: $dest already exists; not touching it" >&2
  exit 1
fi
git init -q "$dest"
git -C "$dest" remote add origin https://github.com/owebeeone/git2-rs.git
git -C "$dest" fetch -q --depth 1 origin "$commit"
git -C "$dest" checkout -q --detach FETCH_HEAD
git -C "$dest" submodule update -q --init --depth 1
echo "checkout-git2-rs: git2-rs $commit at $dest"
