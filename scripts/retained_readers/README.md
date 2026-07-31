# Retained-reader foundation

This directory owns the non-production R0 inventory and offline-first harness
for actual released GWZ readers. It does not emulate an old decoder with
current code.

`manifest.json` enumerates both distributed reader surfaces (`gwz` and
`gwz-py`), the v0.9.2 pre-record downgrade generation, the v0.10.0
additive-field-unknown Rust generation, and the v0.10.2
field-known/recovery-dormant generation. It also records the two behavioral
lanes, release-platform smoke lanes, Python runtime identity, and explicit
unsupported tuples—including the undistributed `gwz-py` v0.10.0 release.
`manifest.schema.json` is the tooling schema. The stdlib validator implements
every schema keyword used here and rejects unknown document fields and unknown
schema keywords. The harness additionally freezes the R0 reader/platform
cross-product, support classifications, referential integrity, uniqueness,
the reviewed GitHub/PyPI URL providers, and fail-not-skip rules.

The inventory intentionally distinguishes these states:

- `verified`: exact immutable artifact URL, name, and lowercase SHA-256 are
  present; acquisition and execution may proceed.
- `pending-acquisition`: the tuple is required and inventoried, but is not a
  release-gate pass. `gate-ready` fails until exact release evidence is added.
- `unsupported`: the historical release did not distribute that tuple; a
  reason and substitute evidence are mandatory.

Run the offline unit suite and inventory check with:

```sh
python3 -m unittest -v scripts/retained_readers/test_retained_reader_harness.py
python3 scripts/retained_readers/retained_reader_harness.py \
  validate scripts/retained_readers/manifest.json
```

Before a record-changing gate, require complete pins:

```sh
python3 scripts/retained_readers/retained_reader_harness.py \
  gate-ready scripts/retained_readers/manifest.json
```

Artifact acquisition is explicit. The default is offline and fails on a cache
miss; only `--allow-network` permits a download. Cache objects live at
`objects/sha256/<digest>`, are verified before reuse, and are never addressed
by a mutable release name.

```sh
python3 scripts/retained_readers/retained_reader_harness.py \
  acquire scripts/retained_readers/manifest.json \
  --cache /path/to/retained-reader-cache
```

`retained_reader_matrix.py` safely extracts Rust archives and creates Python
environments from the verified GWZ wheel and pinned runtime companion wheels.
Derived trees and virtual environments are fresh per run; only raw
content-addressed artifacts are cached. Python runtime keys include the actual
CPython version, architecture, pointer width, and executable digest. Each
fixture is copied to a temporary workspace and snapshotted before and after.
Every machine-output case parses a typed JSON contract. Mutations are either
exact or use narrowly bounded dynamic object/marker path classes. Their
semantic invariant retains the actual normalized content of exact paths and
the observed count plus content digest of every dynamic class. Lock/member
rows, root publication and index state, publication markers, and complete
archived records have independent semantic postconditions. A missing or
unparseable outcome, artifact, runtime, command, fixture identity, expectation,
or required command case fails. A historically undistributed tuple is reported
as `declared-unsupported` with its manifest evidence; it is never a silent skip.

Behavioral cases use `cases.schema.json`. Each runnable reader must be named by
at least one case (or selected by `"*"`), and every case has an explicit
mutation policy:

```json
{
  "schema": "gwz.retained-reader-cases/v1",
  "cases": [{
    "id": "idle-status-json",
    "readers": ["rust-cli-v0.10.0", "rust-cli-v0.10.2", "gwz-py-v0.10.2"],
    "command": "merge-status",
    "args": ["--json", "merge", "--status"],
    "fixture": "custom-message-pending",
    "fixture_sha256": "d272a637a8d49b5926e4e7456c713941358be6b9890dc5639358c1ba4451a23c",
    "expected": {
      "exit_codes": [1],
      "stdout": {"mode": "json-contract", "value": {
        "shape": "merge", "outcomes": ["Halted"],
        "merge_id": "merge_retained", "member_id": "mem_member",
        "member_outcomes": ["Planned"]
      }},
      "mutation": {"mode": "none"}
    }
  }]
}
```

The workspaces themselves are not captured from a developer checkout. Generate
them from fixed identities, timestamps, source files, branches, and v0 YAML:

```sh
python3 scripts/retained_readers/generate_retained_reader_fixtures.py \
  /path/to/generated-fixtures
```

Generation requires Git with `merge-tree --write-tree` and explicit SHA-1
object-format support. It uses a minimal Git environment, canonicalizes config
and indexes, is deterministic across repeated runs, and does not write an
absolute source or destination path into a fixture. `fixture-contract.json`
freezes portable logical identities for all six fixtures: pending custom
message, the exact already-created commit, its wrong-message negative twin,
fast-forwardable `mode: no_ff`, an archived record, and the pre-record view.

Run one platform lane from an already populated cache:

```sh
python3 scripts/retained_readers/retained_reader_matrix.py \
  scripts/retained_readers/manifest.json scripts/retained_readers/cases.json \
  --platform linux-x86_64 --fixtures /path/to/generated-fixtures \
  --cache /path/to/retained-reader-cache \
  --evidence-out /path/to/evidence-linux-x86_64.json
```

The matrix emits one JSON summary and exits nonzero if any required result is
not `passed`. `--allow-network` affects artifact acquisition only; retained
reader invocation itself is noninteractive and uses the isolated fixture.
`--evidence-out` refuses to write evidence for a failing run and strips command
streams, generated operation IDs, changed-path lists, and host/cache paths. It
retains canonical fixture/generator/evaluator identities, Git version and
object format, actual platform, exact Python runtime, pre-run snapshot identity,
and the semantic post-run invariant identity.

Commit and workflow-run IDs cannot live in the byte-stable checked evidence:
the commit would be self-referential and a run ID is deliberately ephemeral.
CI therefore writes a separate attestation bound to the evidence digest:

```sh
python3 scripts/retained_readers/retained_reader_matrix.py \
  scripts/retained_readers/manifest.json scripts/retained_readers/cases.json \
  --platform macos-aarch64 --fixtures /path/to/generated-fixtures \
  --cache /path/to/retained-reader-cache \
  --evidence-out /path/to/evidence-macos-aarch64.json \
  --attestation-out /path/to/execution-macos-aarch64.json
```

`--attestation-out` requires non-null `GITHUB_SHA` and `GITHUB_RUN_ID` and is
valid only with `--evidence-out`.

`evidence-macos-aarch64.json` is the checked result of the full behavioral
case set against the actual macOS arm64 release artifacts: 37 executable
reader/case results passed and the absent `gwz-py` v0.10.0 distribution is
reported as declared unsupported. The remaining cross-platform execution is
the Linux x86_64 and Windows x86_64 behavioral lanes. Linux arm64 and macOS
x86_64 retain artifact-smoke coverage; Windows arm64 is historically
undistributed and remains explicit substitute evidence rather than a skip.

`.github/workflows/retained-readers.yml` runs this same case set on Linux and
Windows x86_64, Linux arm64, and both supported macOS architectures. It uploads
normalized evidence plus a commit/run attestation for every lane. Unit tests
keep the checked macOS evidence byte-canonical and bind it to the current
manifest, cases, fixtures, generator, and evaluator; the separate CI
attestation binds each platform run without introducing a self-referential
commit hash into checked evidence.

Compare fresh macOS evidence with the checked portable semantic projection
(only Git version, Python version, and Python executable digest are excluded):

```sh
python3 scripts/retained_readers/retained_reader_evidence.py compare \
  --checked scripts/retained_readers/evidence-macos-aarch64.json \
  --actual /path/to/fresh-evidence-macos-aarch64.json \
  --manifest scripts/retained_readers/manifest.json \
  --cases scripts/retained_readers/cases.json \
  --fixtures /path/to/generated-fixtures
```
