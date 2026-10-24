#!/usr/bin/env bash
# Check out gwz-transport, gwz-sspi and gwz-git beside gwz-core at the commits that
# .github/gwz-transport.commit, .github/gwz-sspi.commit and .github/gwz-git.commit pin. gwz-core's
# Cargo.toml names the first two by path (`../gwz-transport`, `../gwz-sspi`) in
# tables behind gwz_transport_candidate, and gwz-git (`../gwz-git`) always. Cargo reads the manifest of a path
# dependency whatever its condition, so every job that runs cargo on gwz-core,
# or on a crate that depends on it by path, needs them there, ordinary builds
# included, as it needs git2-rs (checkout-git2-rs.sh).
#
# Usage: checkout-siblings.sh [DIR]
# Clones into DIR/gwz-transport, DIR/gwz-sspi and DIR/gwz-git. DIR defaults to the directory
# that holds the gwz-core checkout this script lives in, which is `..` from
# gwz-core's root in either CI layout (gwz-core at the workspace root, or beside
# gwz-cli or gwz-py). A sibling that is already there is left as it is: a job
# that checks one out with actions/checkout, at a commit of its own, keeps it.
# Run it after those steps, never before.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core="$(dirname "$here")"
parent="${1:-$(dirname "$core")}"
for name in gwz-transport gwz-sspi gwz-git; do
  dest="$parent/$name"
  if [ -e "$dest" ]; then
    echo "checkout-siblings: $dest already exists; not touching it"
    continue
  fi
  commit="$(grep -m1 -E '^[0-9a-f]{40}$' "$here/$name.commit")"
  git init -q "$dest"
  git -C "$dest" remote add origin "https://github.com/owebeeone/$name.git"
  git -C "$dest" fetch -q --depth 1 origin "$commit"
  git -C "$dest" checkout -q --detach FETCH_HEAD
  echo "checkout-siblings: $name $commit at $dest"
done
