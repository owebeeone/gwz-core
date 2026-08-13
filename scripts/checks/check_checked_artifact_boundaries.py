#!/usr/bin/env python3
"""Pin the R2 checked-versus-ordinary source boundary.

This is deliberately a small source inventory, not a Rust call-graph guesser.
It fails closed when an ordinary command imports the checked-artifact module,
when checked acquisition moves outside the enumerated merge/Git facade files,
or when the ordinary shared boundary writer starts using checked authority.
"""

from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "src"

ORDINARY_COMMANDS = (
    "workspace_ops/handle_branch.rs",
    "workspace_ops/handle_commit.rs",
    "workspace_ops/handle_create_repo.rs",
    "workspace_ops/handle_init_from_sources.rs",
    "workspace_ops/handle_materialize.rs",
    "workspace_ops/handle_repo_lifecycle.rs",
    "workspace_ops/handle_stage.rs",
    "workspace_ops/handle_stash.rs",
    "workspace_ops/handle_tag.rs",
    "workspace_ops/pull_head_member_preflight.rs",
    "workspace_ops/sync_workspace_boundary.rs",
)

LEGACY_CHECKED_ENTRY_OWNERS = frozenset(
    {
        "git/gitbackend/preservation_root/files.rs",
        "workspace_ops/merge/preserve/artifacts.rs",
        "workspace_ops/merge/root/artifact_facts.rs",
        "workspace_ops/merge/store/mod.rs",
    }
)

CHECKED_TOKENS = (
    "crate::checked_artifact",
    "CheckedArtifact::acquire",
    "CheckedArtifactPolicy::",
    "PreCatalogOwnerV1",
    "ManagedParentBootstrapOwnerV1",
)

LEGACY_ENTRY_TOKENS = ("CheckedArtifact::acquire", "CheckedArtifact::prepare_parent")


def production_rust_files() -> list[Path]:
    return sorted(
        path
        for path in SRC.rglob("*.rs")
        if "tests" not in path.parts
        and "interface_tests" not in path.parts
        and not path.name.startswith("tests")
    )


def main() -> int:
    findings: list[str] = []
    for relative in ORDINARY_COMMANDS:
        path = SRC / relative
        text = path.read_text(encoding="utf-8")
        for token in CHECKED_TOKENS:
            if token in text:
                findings.append(f"ordinary command reaches checked boundary: {relative}: {token}")

    owners: set[str] = set()
    for path in production_rust_files():
        text = path.read_text(encoding="utf-8")
        if any(token in text for token in LEGACY_ENTRY_TOKENS):
            owners.add(path.relative_to(SRC).as_posix())
    if owners != LEGACY_CHECKED_ENTRY_OWNERS:
        findings.append(
            "legacy checked entry inventory changed: "
            f"expected={sorted(LEGACY_CHECKED_ENTRY_OWNERS)} actual={sorted(owners)}"
        )

    checked_mod = (SRC / "checked_artifact/mod.rs").read_text(encoding="utf-8")
    if "mod coordinator;" not in checked_mod:
        findings.append("checked coordinator is not owned by checked_artifact/mod.rs")
    if "pub(crate) use coordinator" in checked_mod:
        findings.append("raw coordinator types escaped checked_artifact")

    if findings:
        print("checked-artifact boundary: failed", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print(
        "checked-artifact boundary: ok "
        f"({len(ORDINARY_COMMANDS)} ordinary entries, {len(owners)} checked owners)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
