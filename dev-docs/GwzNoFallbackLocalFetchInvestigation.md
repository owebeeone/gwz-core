# No-fallback local fetch investigation

Accepted **L1-A route-selection evidence only**, reviewed at core
`c63f497df29d51ad5d864738fa0056b513c3ab7d` after
[State GO](../../dev-docs/GwzNoFallbackCharacterization-ReviewState.md).
Replacement design/implementation and activation remain separate gates.

## Status and scope

This is the L1-A characterization package for the accepted no-fallback plan.
It records stock libgit2 behavior before selecting an implementation route.
The test calls the native `git2` fetch path and then the existing backend on a
separate, identically seeded receiver.  No production path or public API was
changed.

The core lock uses `git2 0.21.0` with `libgit2-sys 0.18.8+1.9.7`.  The test
asserts `git2::Version::get().libgit2_version() == (1, 9, 7)`, so the native
observations below are from the pinned production dependency.  Its source is
canonicalized and passed as a `file://` URL, the same local transport family
used by `fetch_anonymous`; the options disable `FETCH_HEAD` updates and tag
autodownload.

## Characterization matrix

The matrix has 25 independent rows: each of five source refs is paired with
each of five receiver states.  Source and receiver commits use distinct labels
and OIDs.  The backend receiver is a third repository, so a successful native
transfer cannot hide a backend failure.

| Source/ref object | Receiver hint states |
| --- | --- |
| commit (`refs/heads/main`) | commit, tree, blob, annotated commit tag, annotated tree tag |
| direct tree ref | the same five states |
| direct blob ref | the same five states |
| direct annotated-commit tag-object ref | the same five states |
| direct annotated-tree tag-object ref | the same five states |

Every row requests the source ref into
`refs/gwz/local-imports/characterization`.  The expected destination is the
source ref's object ID, including the tree, blob, and tag-object rows.

## Observed results

- Native libgit2 completed all 25 rows successfully and published the expected
  object ID in every destination ref.
- The existing backend completed all 25 independent rows and published the
  same expected object IDs.
- Native receivers had no configured remotes or tracking refs after the
  anonymous fetch.  `FETCH_HEAD` was present as an empty file (`Some([])`) in
  the observed successful rows.
- The matrix does not reproduce the original noncommit error because its
  receiver-only objects are absent from the source.  The decisive fixture
  below does reproduce it.

The matrix is executable evidence of successful native transfer for these
ref/object forms.  It does not prove behavior for unavailable-object, empty,
cancelled, or repeated-import conditions.

## Decisive shared-object fixture

Three repositories start from the same initial commit and tree.  Each receiver
gets a direct `refs/codex/checkpoints/tree` ref to that shared tree.  The source
then advances `refs/heads/main` with a new commit that reuses the old tree, so
the fetch must negotiate a new commit while the receiver's noncommit object is
also present in the source object database.

The direct native fetch fails with the pinned exact class and message:
`InvalidSpec` / `Invalid`, `object is not a committish`.  The existing backend
on a separate identically seeded receiver completes the same fetch and writes
the advanced source commit.  This explains why the 25-row matrix passed: its
receiver hints had distinct objects that were not in the source and therefore
did not reach the erroneous source-side revwalk condition.

## Partial and error behavior

The multi-ref case requests one valid ref followed by a missing source ref.
Native libgit2 returns success, publishes the first destination with the source
commit ID, leaves the second destination absent, and leaves `FETCH_HEAD` as an
empty file.  This is observable partial publication even though the operation
reports success; atomicity and the desired contract remain pending.

A malformed destination refspec fails with `GenericError` / `Invalid` and the
message `'+refs/heads/main:refs/heads/[invalid]' is not a valid refspec.`  A
source ref pointing at a missing object fails with `GenericError` / `Odb` and
an `object not found - no match for id` message.  Neither error is the
noncommittish fallback trigger.

These checks preserve error detail in the test output and deliberately assert
the meaningful classes observed.  The exact native error surface may still
need a compatibility mapping if production begins handling these cases.

## Route evidence and pending work

The narrow C correction route now has a precise reproducer to target: the
shared-object fixture fails in native `local.c` with the exact noncommittish
error while the existing backend succeeds through its compatibility path.  The
fixture identifies the trigger and preserves a regression oracle; it does not
select the eventual C change or establish its complete error/atomicity
contract.

The native direct-transfer route remains a separate hypothesis.  It would need
to define object closure, ref update ordering, partial-failure rollback,
`FETCH_HEAD`, cancellation, repeated imports, and SHA-1/SHA-256 behavior
before implementation.  This characterization selects neither route.

Omitted from L1-A are Git-unavailable backend gating, cancellation, repeated
import collisions, empty or unborn repositories, SHA-256 repositories, and
large pack behavior.  These are follow-up evidence, not conclusions.

## Package bounds

The bounded first package contains one new test file (476 lines), one new
investigation document (this file), zero production additions, zero production
moves, zero tool changes, and zero protocol changes.  It makes no dependency,
manifest, lockfile, compiler, or C-source changes.

Focused runner:

```text
cargo +1.95.0 test --locked -p gwz-core --lib local_clone::tests::transport_noncommit -- --nocapture
```

Executed on macOS aarch64: three focused tests passed; no failures.
