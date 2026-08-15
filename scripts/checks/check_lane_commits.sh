#!/usr/bin/env bash
# Per-commit lane gate: run each commit's own boundary checker against that
# commit's exact tree, so a red intermediate commit cannot hide behind a
# green branch head. Motivated by the two recorded deviations where the
# mandatory gate was red at an intermediate commit and healed later
# (95d292f, b923109 — see CurrentProgramCheckpoint.md deviation record and
# ReviewCode-3/-4 finding P3-3).
#
# LANE_GATE_FLOOR excludes commits that predate this mechanism: the two
# historical red commits are permanent ancestors of the floor and are
# already recorded as deviations; re-flagging them on every future push
# would block the branch for history that cannot change. Commits after the
# floor get no such grace.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: check_lane_commits.sh <base> <head>" >&2
  exit 2
fi
base="$1"
head="$2"
python="${PYTHON:-python3}"
floor="${LANE_GATE_FLOOR:-ca520e46ec2b89f61ab81b6cdfe8c946bf220228}"

commits=$(git rev-list --reverse "${head}" --not "${base}" "${floor}")
if [ -z "${commits}" ]; then
  echo "lane gate: no post-floor commits in ${base}..${head}"
  exit 0
fi
status=0
for sha in ${commits}; do
  tmp=$(mktemp -d)
  git archive "${sha}" | tar -x -C "${tmp}"
  if "${python}" "${tmp}/scripts/checks/check_checked_artifact_boundaries.py" \
      --source "${tmp}/src" > "${tmp}/gate.out" 2>&1; then
    echo "lane gate: ok at ${sha}"
  else
    echo "lane gate: boundary checker RED at ${sha}" >&2
    cat "${tmp}/gate.out" >&2
    status=1
  fi
  rm -rf "${tmp}"
done
exit "${status}"
