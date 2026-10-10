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
# The Windows-parity inventory a commit is compared with, whichever form it has: the directory
# scripts/checks/windows_parity/, or the single windows_parity_inventory.json that preceded it. A commit is
# compared with the base as of its own fork point (git merge-base), not with the base's current tip: a lane cloned
# from an older main lowers the count against where it started, and the base may have moved on and lowered it
# further meanwhile. A merge of the base brings the fork point up to that base.
inventory_at() {
  local ref="$1" dir="$2"
  if git cat-file -e "${ref}:scripts/checks/windows_parity/meta.json" 2>/dev/null; then
    git archive "${ref}" scripts/checks/windows_parity | tar -x -C "${dir}"
    echo "${dir}/scripts/checks/windows_parity"
  elif git cat-file -e "${ref}:scripts/checks/windows_parity_inventory.json" 2>/dev/null; then
    git show "${ref}:scripts/checks/windows_parity_inventory.json" > "${dir}/windows_parity_inventory.json"
    echo "${dir}/windows_parity_inventory.json"
  fi
}
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
    base_tmp=$(mktemp -d)
    base_inventory=$(inventory_at "$(git merge-base "${base}" "${sha}")" "${base_tmp}")
    "${python}" "${tmp}/scripts/checks/check_windows_parity.py" --root "${tmp}" > "${tmp}/parity.out" 2>&1 || parity_ok=0
    # A commit made before the per-step inventory (scripts/checks/windows_parity/) carries a checker
    # that reads only the single-file form, so it cannot compare against a base in the directory form.
    # Its count is compared once it is merged: the merge commit has the per-step inventory.
    if [ -n "${base_inventory}" ] && [ -d "${base_inventory}" ] && [ ! -d "${tmp}/scripts/checks/windows_parity" ]; then
      echo "lane gate: ${sha}: older inventory form than the base; shrink comparison left to the merge" \
        >> "${tmp}/parity.out"
    elif [ -n "${base_inventory}" ]; then
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
    rm -rf "${base_tmp}"
  fi
  rm -rf "${tmp}"
done
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
