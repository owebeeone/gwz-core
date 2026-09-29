"""Provenance and retained-reader checks for placement candidate regeneration."""

import importlib.util
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent
REGEN_PATH = ROOT / "candidate-regenerator.py"
TAUT_SOURCE = ROOT.parents[2] / "taut" / "src"
CORE = ROOT.parents[1]
CORE_SCHEMA = CORE / "protocol" / "gwz.taut.py"
OWNER_SCHEMA = ROOT.parents[2] / "gwz-transport" / "protocol" / "transport.ir.json"
SPEC = importlib.util.spec_from_file_location("candidate_regen", REGEN_PATH)
assert SPEC is not None and SPEC.loader is not None
REGEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REGEN)


@pytest.mark.parametrize(
    ("cwd", "inherited_toolchain"),
    [(CORE, None), (CORE.parent, None), (CORE.parent, "stable")],
    ids=["from-gwz-core", "from-workspace-root", "with-an-inherited-toolchain"],
)
def test_candidate_regeneration_matches_checked_artifacts(cwd, inherited_toolchain):
    # The proof runs rustfmt under gwz-core's pinned toolchain, so neither the
    # working directory nor an exported RUSTUP_TOOLCHAIN changes its outcome.
    env = {name: value for name, value in os.environ.items() if name != "RUSTUP_TOOLCHAIN"}
    if inherited_toolchain is not None:
        env["RUSTUP_TOOLCHAIN"] = inherited_toolchain
    subprocess.run(
        [
            "python3",
            str(REGEN_PATH),
            "--core-schema",
            str(CORE_SCHEMA),
            "--owner-schema",
            str(OWNER_SCHEMA),
            "--taut-source",
            str(TAUT_SOURCE),
            "--check",
        ],
        check=True,
        cwd=cwd,
        env=env,
    )


def test_dirty_canonical_taut_source_is_rejected(monkeypatch):
    original = REGEN.subprocess.run

    def fake_run(command, **kwargs):
        if "status" in command:
            return type("Result", (), {"stdout": " M src/taut/wire/codec.py\n"})()
        return original(command, **kwargs)

    monkeypatch.setattr(REGEN.subprocess, "run", fake_run)
    with pytest.raises(SystemExit, match="checkout is dirty"):
        REGEN._verify_pin(OWNER_SCHEMA, TAUT_SOURCE)


def test_retained_reader_is_pinned_checked_in_baseline():
    retained = CORE / "tests" / "transport_consumer" / "candidate" / "retained_old_generated.rs"
    metadata = REGEN.json.loads((ROOT / "candidate-generator.json").read_text())
    assert REGEN.hashlib.sha256(retained.read_bytes()).hexdigest() == metadata[
        "retained-old-rust-sha256"
    ]
    assert REGEN.hashlib.sha256(CORE_SCHEMA.read_bytes()).hexdigest() == metadata[
        "retained-old-schema-sha256"
    ]


@pytest.mark.parametrize("artifact", ["CORPUS_GOLDEN", "CORPUS_VECTORS"])
def test_a_stale_candidate_corpus_fails_the_check(artifact, tmp_path):
    # The candidate build's corpus_byte_parity runs this corpus, so --check must
    # refuse a copy that no longer matches what the schema generates.
    checked_in = getattr(REGEN, artifact)
    stale = tmp_path / checked_in.name
    stale.write_text(checked_in.read_text().replace('"', "'", 1))
    script = "\n".join(
        [
            "import importlib.util, pathlib, sys",
            f"spec = importlib.util.spec_from_file_location('regen', {str(REGEN_PATH)!r})",
            "regen = importlib.util.module_from_spec(spec)",
            "spec.loader.exec_module(regen)",
            f"regen.{artifact} = pathlib.Path({str(stale)!r})",
            "sys.argv = ['regen', '--core-schema', sys.argv[1], '--owner-schema', sys.argv[2],"
            " '--taut-source', sys.argv[3], '--check']",
            "regen.main()",
        ]
    )
    result = subprocess.run(
        ["python3", "-c", script, str(CORE_SCHEMA), str(OWNER_SCHEMA), str(TAUT_SOURCE)],
        capture_output=True,
        text=True,
        cwd=CORE,
    )
    assert result.returncode != 0
    assert f"stale candidate generated artifacts: {stale}" in result.stderr
