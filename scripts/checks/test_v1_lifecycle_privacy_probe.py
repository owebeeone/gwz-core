#!/usr/bin/env python3
"""Compile probes for the sealed v1 lifecycle privacy perimeter.

R4b-G F-2 / inventory W1 / evidence row 2.5b. Battery source:
`GwzM5-8R4bTransitionDesign.md:1478-1479` -- "a privacy/compile test proving
lifecycle code cannot construct `PreparedV1Rewrite`, proof tokens, or call a
raw v1 writer".

Rust visibility already gives the property. Nothing proved it FAILS CLOSED,
which is what the 22 sibling probes in
`test_check_checked_artifact_boundaries.py` do for the checked-artifact
boundary. These probes close that gap in the same temp-copy idiom: mutate a
copy of the tree, run the compiler over it, and assert the outcome.

The direction is inverted relative to the boundary probes, and deliberately.
There the compiler ACCEPTS the mutation and the checker must reject it, so
`run_compiler_probe` asserts returncode 0 first. Here the compiler IS the
enforcement, so the negative probes assert it rejects, and a POSITIVE CONTROL
compiles the byte-identical probe text from inside the perimeter. Without that
control a renamed or deleted item would make every negative probe pass for the
wrong reason; with it, the pair states exactly "this name exists, and it is
not reachable from there".

`finalize_dispatch.rs` is the deliberate outside position: it is the v0
finalizer, a direct child of `workspace_ops::merge`, and therefore the
MOST privileged module outside the seal -- `v1_lifecycle` is private to
`merge`, so a probe that cannot reach in from there cannot reach in from
anywhere.

Three seals, not one perimeter:
  * `PreparedV1Rewrite` -- `pub(super)` in the private `transition` module;
  * proof tokens (`VerifiedParticipants`) -- `pub(super)` in `authority`;
  * the raw writer `store::rewrite::commit` -- `pub(super)` in the private
    `rewrite` module, sealed against the REST OF THE LIFECYCLE too, which the
    fourth negative probe states.

Two paths per seal, not one (R4b-G correctness review finding C-3). Each
canonical path above is sealed by the PRIVACY OF ITS MODULE, so a two-edit
widening -- item to `pub(crate)` PLUS a re-export at the `v1_lifecycle` root --
leaves every canonical probe green (`transition`/`authority`/`store` stay
private, so the canonical path still E0603s) while an outside consumer names
the root binding and compiles. The review demonstrated exactly that. The three
`ROOT_BINDING` probes therefore state the second half of each seal: the name is
not bound at the `v1_lifecycle` module root either, so the re-export channel is
closed for all three. Residual, stated rather than hidden: name resolution is
by name, so a re-export renamed with `as` binds a name no probe can predict --
that channel is held by `PROTECTED_SOURCE_TREE_DIGESTS`, which pins
`v1_lifecycle/mod.rs` where any such re-export must be written.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
TOOLCHAIN = "+1.95.0"

# M5d: the previous outside position, `merge/finalize_dispatch.rs`, was a v0
# engine file deleted by 8603b58, which left this probe pointing at a missing
# path (5 of 8 tests erroring with FileNotFoundError). `merge/plan.rs` is the
# replacement on purpose: it is a SIBLING of `v1_lifecycle/` inside `merge/`,
# which is the strongest outside position available -- same crate, same parent
# module, so a leak of a sealed type would be visible here first. It is the
# shared planner every merge runs through, predates and survives the erasure,
# and is not a deletion candidate.
OUTSIDE = "src/workspace_ops/merge/plan.rs"
INSIDE_LIFECYCLE = "src/workspace_ops/merge/v1_lifecycle/service.rs"
INSIDE_STORE = "src/workspace_ops/merge/v1_lifecycle/store/mod.rs"

SEALED = {
    "prepared_rewrite": "transition::PreparedV1Rewrite",
    "proof_token": "authority::VerifiedParticipants",
    "raw_writer": "store::rewrite::commit",
}

# The same three seals named at the `v1_lifecycle` module root -- the one place
# a `use` re-export that widens them can be written. Nothing binds these names
# there today: `v1_lifecycle/mod.rs` declares modules and one sentinel const and
# carries no `use` at all, so each probe below is an E0432 on the real tree.
ROOT_BINDING = {
    "prepared_rewrite": "PreparedV1Rewrite",
    "proof_token": "VerifiedParticipants",
    "raw_writer": "commit",
}


def probe_text(label: str, path: str) -> str:
    """The byte-identical probe body used inside and outside the seal."""
    return (
        f"\n#[cfg(test)]\nmod r4bg_privacy_probe_{label} {{\n"
        "    #[allow(unused_imports)]\n"
        "    use crate::workspace_ops::merge::v1_lifecycle::"
        f"{path};\n}}\n"
    )


def compile_with_probe(
    relative: str, label: str, path: str
) -> subprocess.CompletedProcess[str]:
    temporary = tempfile.TemporaryDirectory()
    target = Path(temporary.name) / "gwz-core"
    # Same copy set as `run_compiler_probe`: the tree carries `include_str!`
    # seam pins that reach `dev-docs/` and `scripts/`, so a narrower copy
    # fails to build for reasons that have nothing to do with privacy.
    for name in (".github", "dev-docs", "protocol", "scripts", "src", "tests"):
        shutil.copytree(ROOT / name, target / name)
    for name in ("Cargo.toml", "Cargo.lock", "clippy.toml", "rust-toolchain.toml"):
        shutil.copy2(ROOT / name, target / name)
    probed = target / relative
    probed.write_text(
        probed.read_text(encoding="utf-8") + probe_text(label, path), "utf-8"
    )
    env = os.environ.copy()
    env.setdefault(
        "CARGO_TARGET_DIR", str(ROOT.parent / "target" / "v1-privacy-probe")
    )
    try:
        return subprocess.run(
            [
                "cargo",
                TOOLCHAIN,
                "check",
                "--manifest-path",
                str(target / "Cargo.toml"),
                "--all-targets",
            ],
            check=False,
            capture_output=True,
            text=True,
            env=env,
        )
    finally:
        temporary.cleanup()


class V1LifecyclePrivacyProbeTest(unittest.TestCase):
    def assert_sealed(self, relative: str, label: str, private_module: str) -> None:
        result = compile_with_probe(relative, label, SEALED[label])
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("E0603", result.stderr)
        self.assertIn(f"module `{private_module}` is private", result.stderr)

    def assert_unbound_at_the_root(self, relative: str, label: str) -> None:
        """The seal's second half: no re-export binds the name at the root.

        A rejection is asserted by error code AND by the rejected path, so a
        build that fails for an unrelated reason cannot green this probe.
        """
        name = ROOT_BINDING[label]
        result = compile_with_probe(relative, f"{label}_at_root", name)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertTrue(
            "E0432" in result.stderr or "E0603" in result.stderr, result.stderr
        )
        self.assertIn(f"v1_lifecycle::{name}", result.stderr)

    def test_sealed_names_exist_and_compile_inside_the_perimeter(self) -> None:
        for label, relative in (
            ("prepared_rewrite", INSIDE_LIFECYCLE),
            ("proof_token", INSIDE_LIFECYCLE),
            ("raw_writer", INSIDE_STORE),
        ):
            with self.subTest(label=label):
                result = compile_with_probe(relative, label, SEALED[label])
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_prepared_v1_rewrite_is_unnameable_outside_the_perimeter(self) -> None:
        self.assert_sealed(OUTSIDE, "prepared_rewrite", "transition")

    def test_proof_tokens_are_unnameable_outside_the_perimeter(self) -> None:
        self.assert_sealed(OUTSIDE, "proof_token", "authority")

    def test_raw_v1_writer_is_unnameable_outside_the_perimeter(self) -> None:
        self.assert_sealed(OUTSIDE, "raw_writer", "store")

    def test_raw_v1_writer_is_unnameable_from_the_rest_of_the_lifecycle(self) -> None:
        self.assert_sealed(INSIDE_LIFECYCLE, "raw_writer", "rewrite")

    def test_prepared_v1_rewrite_is_unbound_at_the_lifecycle_root(self) -> None:
        self.assert_unbound_at_the_root(OUTSIDE, "prepared_rewrite")

    def test_proof_tokens_are_unbound_at_the_lifecycle_root(self) -> None:
        self.assert_unbound_at_the_root(OUTSIDE, "proof_token")

    def test_raw_v1_writer_is_unbound_at_the_lifecycle_root(self) -> None:
        # Probed from INSIDE the lifecycle, mirroring the seal one line above:
        # the writer is sealed against the rest of the lifecycle too, and a
        # root `use` -- even a private one -- is nameable by every descendant.
        self.assert_unbound_at_the_root(INSIDE_LIFECYCLE, "raw_writer")


if __name__ == "__main__":
    unittest.main()
