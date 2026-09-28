#!/usr/bin/env python3

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
RELEASE_PATH = ROOT / "scripts" / "release.py"
SPEC = importlib.util.spec_from_file_location("gwz_core_release", RELEASE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load release script")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseBoundaryTest(unittest.TestCase):
    def test_release_checks_do_not_launch_mutation_suites(self) -> None:
        with mock.patch.object(release, "run") as run, mock.patch.object(release, "cargo_env", return_value={}):
            release.run_checked_boundary_gates(cargo_root=release.REPO)
        commands = [str(call.args[0]) for call in run.call_args_list]
        self.assertFalse(any("test_check_checked_artifact_boundaries" in command or
                             "test_v1_lifecycle_privacy_probe" in command
                             for command in commands), commands)

    def test_release_help_exposes_no_compiler_skip(self) -> None:
        result = subprocess.run(
            [sys.executable, str(RELEASE_PATH), "--help"],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertNotIn("--no-clippy", result.stdout)

    def test_exact_release_gate_reacquires_sha_before_boundary(self) -> None:
        cargo_root = Path("/tmp/exact-release-tree")
        expected = "a" * 40
        calls: list[tuple[str, ...]] = []

        def fake_run(command, **_kwargs):
            calls.append(tuple(str(item) for item in command))
            if command[:3] == ["git", "rev-parse", "HEAD"]:
                return SimpleNamespace(stdout=f"{expected}\n", returncode=0)
            if command[:3] == ["git", "status", "--porcelain"]:
                return SimpleNamespace(stdout="", returncode=0)
            return SimpleNamespace(stdout="", returncode=0)

        with (
            mock.patch.object(release, "run", side_effect=fake_run),
            mock.patch.object(release, "run_checked_boundary_gates") as boundary,
        ):
            release.gate_exact_release_commit(
                cargo_root=cargo_root, expected_head=expected
            )

        self.assertEqual(calls[0], ("git", "reset", "--hard", expected))
        self.assertEqual(calls[1], ("git", "rev-parse", "HEAD"))
        self.assertEqual(calls[2], ("git", "status", "--porcelain"))
        self.assertEqual(calls[3], ("git", "rev-parse", "HEAD"))
        self.assertEqual(calls[4], ("git", "status", "--porcelain"))
        boundary.assert_called_once_with(cargo_root=cargo_root)

    def test_exact_release_gate_rejects_wrong_sha_before_boundary(self) -> None:
        with (
            mock.patch.object(
                release,
                "run",
                return_value=SimpleNamespace(stdout=f"{'b' * 40}\n", returncode=0),
            ),
            mock.patch.object(release, "run_checked_boundary_gates") as boundary,
            self.assertRaises(SystemExit),
        ):
            release.gate_exact_release_commit(
                cargo_root=release.REPO, expected_head="a" * 40
            )
        boundary.assert_not_called()

    def test_exact_release_gate_rejects_sha_drift_after_boundary(self) -> None:
        expected = "a" * 40
        observed = iter((expected, "b" * 40))

        def fake_run(command, **_kwargs):
            if command[:3] == ["git", "rev-parse", "HEAD"]:
                return SimpleNamespace(stdout=f"{next(observed)}\n", returncode=0)
            if command[:3] == ["git", "status", "--porcelain"]:
                return SimpleNamespace(stdout="", returncode=0)
            return SimpleNamespace(stdout="", returncode=0)

        with (
            mock.patch.object(release, "run", side_effect=fake_run),
            mock.patch.object(release, "run_checked_boundary_gates") as boundary,
            self.assertRaises(SystemExit),
        ):
            release.gate_exact_release_commit(
                cargo_root=release.REPO, expected_head=expected
            )
        boundary.assert_called_once_with(cargo_root=release.REPO)

    def test_new_tag_finalizer_always_gates_exact_target_before_tag_and_push(self) -> None:
        calls: list[str] = []
        with (
            mock.patch.object(
                release,
                "gate_exact_release_commit",
                side_effect=lambda **_kwargs: calls.append("gate"),
            ),
            mock.patch.object(
                release,
                "gate_release_publication",
                side_effect=lambda **_kwargs: calls.append("publication"),
            ),
            mock.patch.object(
                release,
                "ensure_tag",
                side_effect=lambda *_args: calls.append("tag"),
            ),
            mock.patch.object(
                release,
                "push_release",
                side_effect=lambda *_args, **_kwargs: calls.append("push"),
            ),
        ):
            release.finalize_new_release(
                cargo_root=Path("/tmp/exact-release-tree"),
                expected_head="a" * 40,
                branch="main",
                tag="v1.2.3",
                push=True,
            )

        # The crates.io lockstep check and the packaging pass (plan S1.5) are
        # gates on the exact target too: both run before the tag exists and
        # before anything is pushed.
        self.assertEqual(calls, ["gate", "publication", "tag", "push"])

    def test_atomic_push_sources_branch_and_tag_from_the_gated_sha(self) -> None:
        expected = "a" * 40
        with mock.patch.object(
            release,
            "run",
            return_value=SimpleNamespace(stdout="", stderr="", returncode=0),
        ) as run:
            release.push_release("main", "v1.2.3", expected_head=expected)

        self.assertEqual(
            run.call_args.args[0],
            [
                "git",
                "-C",
                release.REPO,
                "push",
                "--atomic",
                "origin",
                f"{expected}:refs/heads/main",
                f"{expected}:refs/tags/v1.2.3",
            ],
        )

    def test_release_tests_name_the_gwz_transport_beside_this_checkout(self) -> None:
        # run_tests.py fails closed without a gwz-transport checkout (B11,
        # GwzCoreSessionDesign §5.7), and the release gates may run in a /tmp
        # worktree that has none beside it. The release therefore names the
        # one beside this checkout, unless the caller already named one.
        worktree = Path("/tmp/gwz-core-v1.2.3-1/gwz-core")
        sibling = str(release.REPO.parent / "gwz-transport")
        for environment, named in (({}, sibling), ({"GWZ_TRANSPORT_CHECKOUT": "/elsewhere"}, "/elsewhere")):
            with (
                mock.patch.object(release, "run") as run,
                mock.patch.object(release, "cargo_env", return_value=dict(environment)),
                mock.patch.object(release, "regen_python"),
                mock.patch.object(release, "run_fmt_check"),
                mock.patch.object(release, "assert_lock_current"),
                mock.patch.object(release, "run_checked_boundary_gates"),
            ):
                release.run_gates(cargo_root=worktree, skip_regen=True, no_test=False)
            suites = [call for call in run.call_args_list
                      if any(str(part).endswith("run_tests.py") for part in call.args[0])]
            self.assertEqual(len(suites), 1)
            self.assertEqual(suites[0].kwargs["cwd"], worktree)
            self.assertEqual(suites[0].kwargs["env"]["GWZ_TRANSPORT_CHECKOUT"], named)

    def test_release_tests_skip_the_cfg_siblings_the_worktree_lacks(self) -> None:
        # Nor are gwz-cli and gwz-py beside that worktree, and the release
        # gates gwz-core, so the conditional-compilation check covers gwz-core
        # alone and prints SKIPPED GATE for the two (GwzCoreSessionPlan CS1.7).
        with mock.patch.object(release, "run") as run, mock.patch.object(release, "cargo_env", return_value={}):
            release.run_test_suite(cargo_root=Path("/tmp/gwz-core-v1.2.3-1/gwz-core"))
        command = [str(part) for part in run.call_args.args[0]]
        self.assertTrue(command[1].endswith("run_tests.py"), command)
        self.assertEqual(command[2:], ["--skip-cfg-siblings"])

    def test_release_runs_the_test_suite_only_through_that_helper(self) -> None:
        # Both release test runs, before the version bump and after it, go
        # through run_test_suite, so neither loses the checkout it names.
        self.assertEqual(RELEASE_PATH.read_text(encoding="utf-8").count('"run_tests.py"'), 1)


if __name__ == "__main__":
    unittest.main()
