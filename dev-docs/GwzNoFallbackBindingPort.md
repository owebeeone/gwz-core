# Qualified member binding port (L2-A)

Date: 2026-09-20. Status: candidate, awaiting Code/State/Surface review.
Authority: [first-package checkpoint](GwzNoFallbackCheckpoint.md).
This ports the already accepted per-remote binding; it introduces no new Rust
API or production route. The only new public tool input is `--git2-source`.

## Source and ownership

Member `git2-rs` is on `codex/per-remote-transport`, branched through GWZ from
release commit `dffaf272eb0e62ac15b74283c4e488252db9afc3`. Original fork main
`f42a01267a3042b26d30e9d8acf286c6c739bd8a` is preserved. The original two files
match `tests/transport_native/binding-pin.json`; applying its accepted patch
produces exactly the two pinned patched hashes. No patch bytes were revised.

The sole manifest edit removes the root crate's old native path dependency
and selects registry `libgit2-sys = "=0.18.8"`. The isolated fixture retains
its locked `0.18.8+1.9.7` graph and current features. The release branch's old
C gitlink remains historical and unused; no submodule update or C fork occurs.
The member's own workspace-wide tests are not claimed qualified: its auxiliary
workspace packages still reference the old path sys package. This package
qualifies the root git2 library as an external dependency only.

`prove.py` retains archive mode and adds mutually exclusive member mode.
Member admission compares the complete file set and content against the exact
release tree/blob objects (not attribute-sensitive archives), allowing only the two pinned binding outputs and the exact
manifest edit. It rejects unrelated tracked/untracked/ignored input changes,
missing files, executable-mode changes and file/symlink substitution. It omits
Git metadata, root build output and the unused C submodule directory. Verified
bytes are copied to temporary storage before Cargo runs. The checkout, cache,
production manifests and checked-in fixture lock are not mutated.

## Executed evidence

Host: macOS aarch64, Rust 1.95.0; Git 2.52.0.

- Source-admission tests first failed because `copy_member` did not exist.
  Seven Python tests now pass, including the two retained lock-graph guards,
  exact admission/isolation, content/native-edge/partial-patch drift, extra and
  missing files, and symlink substitution. A real-repository regression first reproduced
  hidden `info/attributes` concealing a missing release file; object reads now
  reject that attack. The same attack was rejected in a temporary clone
  containing the actual pinned release, after positive admission succeeded.
- The unmodified member was refused by source admission. A temporary isolated
  copy with only the native manifest alignment reproduced compile errors for
  the missing `RemoteCallbacks::smart_transport` method in both existing test
  files. No installed source or registry cache was edited.
- `python3 tests/transport_native/prove.py --git2-source ../git2-rs` from core
  passes all seven existing native integration tests. File transport, named/
  anonymous/clone, fetch/push, registry coexistence, errors/panics, nested and
  concurrent contexts, and retained Remote lifetime remain covered.
- The retained `--git2-archive` path passes the same native tests.
- Build output reports `libgit2_vendored`, `libgit2_experimental_sha256`, and
  static `git2` from the registry sys build output. The native source is the
  locked bundled 1.9.7 baseline, not the member's old C gitlink or system library.
- Changed binding files pass Rust 1.95 rustfmt checking, and source hashes match
  the accepted patch. New branches are braced; no conditional declarations were
  added. Existing upstream conditional-style debt is not migrated.

Reproduce Python guards from workspace root:

```sh
python3 -m unittest discover -s gwz-core/tests/transport_native -p test_prove.py
```

The [fixture README](../tests/transport_native/README.md) documents prerequisites,
commands, input defaults, isolation, retained build output and cleanup.

## Limits and next gate

No GWZ production dependency or lock changes, package rename, publication,
push, real SSH endpoint, pooling or message-pump activation occurred. Native
qualification here is macOS aarch64 with the existing experimental-SHA256 ABI;
it is not a new complete SHA-1/SHA-256 behavior matrix or five-platform result.
Windows, Linux and macOS x86_64, all production consumers/features, packaged
release consumption and eventual dependency activation remain separate gates.

Actual diff: 72 added/changed Rust lines across the two binding files; one
manifest line replacement; 88 changed runner lines; 98 added Python test lines;
18 README lines plus this report. Seven L2-owned/source-alignment files total;
within the accepted 150 Rust/8 manifest/100 test/120 tool/180 documentation
ceilings. Shared test wiring and other lane outputs are outside this package.
