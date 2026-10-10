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
# The base's Windows-parity inventory, whichever form the base has: the directory scripts/checks/windows_parity/,
# or the single windows_parity_inventory.json that preceded it (origin/main has only the file until the split lands).
base_inventory=""
base_tmp=$(mktemp -d)
if git cat-file -e "${base}:scripts/checks/windows_parity/meta.json" 2>/dev/null; then
  git archive "${base}" scripts/checks/windows_parity | tar -x -C "${base_tmp}"
  base_inventory="${base_tmp}/scripts/checks/windows_parity"
elif git cat-file -e "${base}:scripts/checks/windows_parity_inventory.json" 2>/dev/null; then
  git show "${base}:scripts/checks/windows_parity_inventory.json" > "${base_tmp}/windows_parity_inventory.json"
  base_inventory="${base_tmp}/windows_parity_inventory.json"
fi
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
  # The Windows-parity inventory (GwzTransportWindowsParityPlan.md, step 0.3):
  # this commit's tree against its own inventory, and the inventory against the
  # base's, so the unported count only shrinks. A commit that predates the
  # checker has none to run.
  if [ -f "${tmp}/scripts/checks/check_windows_parity.py" ]; then
    parity_ok=1
    "${python}" "${tmp}/scripts/checks/check_windows_parity.py" --root "${tmp}" > "${tmp}/parity.out" 2>&1 || parity_ok=0
    if [ -n "${base_inventory}" ]; then
      "${python}" "${tmp}/scripts/checks/check_windows_parity.py" --shrink-from "${base_inventory}" \
        >> "${tmp}/parity.out" 2>&1 || parity_ok=0
    fi
    if [ "${parity_ok}" = 1 ]; then
      echo "lane gate: Windows parity ok at ${sha}"
    else
      echo "lane gate: Windows-parity check RED at ${sha}" >&2
      cat "${tmp}/parity.out" >&2
      status=1
    fi
  fi
  rm -rf "${tmp}"
done
rm -rf "${base_tmp}"
# The Windows compile gate (GwzTransportWindowsParityPlan.md, step 0.4; skim finding P3-3): a commit that changes a
# trigger path needs a `Windows-receipt: <label>` line in its message or in a later commit of the range, or
# `Windows-receipt: ci-only <reason>`. The range is judged as a whole, with the head's own script and inventory, so
# it runs once. A head that predates the script has no rule to apply.
head_tmp=$(mktemp -d)
git archive "${head}" | tar -x -C "${head_tmp}"
if [ -f "${head_tmp}/scripts/checks/check_windows_receipts.py" ]; then
  if "${python}" "${head_tmp}/scripts/checks/check_windows_receipts.py" "${base}" "${head}" --floor "${floor}"; then
    :
  else
    echo "lane gate: Windows-receipt check RED in ${base}..${head}" >&2
    status=1
  fi
fi
rm -rf "${head_tmp}"
exit "${status}"
