# gwz-git: foundation API

Status: API design accepted 2026-09-21 at core
`3efc1a79a1e5044b6e2495ed392a5c43d8f90b64`. G0 implementation is accepted locally at library
`aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; unpublished, with production
and platform gates still pending. See the [acceptance record](GwzGitLibraryG0.md). See its [README](../../gwz-git/README.md) for build/test instructions.
This guide describes the first
read-only package only, as amended below (A1). It does not provide
commit/tag/history/fetch operations yet. It runs synchronous local repository
reads; call it on a suitable worker.

**Amendment A1 (2026-10-25, convergence plan step 5.1b; operator decisions D-G
and D-K).** Three additions the capability map
([GwzGitCapabilityMap.md](GwzGitCapabilityMap.md), findings F2 and F3) found
missing: `Repository::open_from_env`, `Repository::resolve`, and
`ErrorKind::NotARepository`. They are specified in the sections below, marked
"A1". Nothing else in this guide changes, with one behaviour difference noted
under Errors: an open that finds no repository now reports `NotARepository`
instead of `RepositoryOpen`. Library `gwz-git` implements them with tests;
no consumer calls them yet, and the unpublished/platform-pending status above
is unchanged.

**Supersession (F2).** The operator decision of 2026-10-14 recorded in
`history/parked/GwzNoFallbackPlan.md` §6 (quoted in core commit `8cefbe40`),
that native commit and tag "stay a module of gwz-core", is superseded by
decisions D-G (2026-10-24) and D-K (2026-10-25) of
[the convergence plan](../../dev-docs/GwzTransportReleaseConvergencePlan.md) §10: step 5.2
moves native commit and tag, with their characterization tests, to gwz-git.
The 2026-10-12 route rule and the 2026-10-13 `commit -a` choice are unchanged
and move with the code. This line is the annotation the 2026-10-14 text lacks;
that archived document is not edited.

The initial development checkout is a sibling `gwz-git` member in the
prepared GWZ workspace, with its own Cargo workspace and lockfile. It requires
Rust 1.95 and the qualified sibling git2-rs checkout. There is no crates.io
install or remote-only bootstrap yet. Downstream production use awaits source
distribution and platform qualification. No service is installed or configured.

## Open, inspect, close

Usage with the accepted local G0 foundation:

```rust,no_run
use gwz_git::{ObjectId, Repository};

fn inspect(path: &std::path::Path, full_hex_id: &str)
    -> Result<(), gwz_git::Error>
{
    let repo = Repository::open_exact(path)?;
    let id = ObjectId::parse_hex(repo.object_format(), full_hex_id)?;
    let commit = repo.read_commit(id)?;
    println!("{} has {} parents", commit.id, commit.parents.len());
    drop(repo); // releases native resources; commit remains usable
    Ok(())
}
```

Supply an explicit worktree root, Git directory or bare repository root. Linked
worktrees are supported. A nested directory does not search parents. Ordinary
filesystem links are followed; opening is not a path-security boundary.
`GIT_DIR` does not choose a different repository, in either open below: with a
path supplied the path chooses. Opening does not create one.
No remote access, lazy fetching, credential prompt or subprocess is performed.
No refs, index, worktree or process cwd/environment are changed.

`Repository` is opaque and `Send`, but not Clone or Sync. Ownership may move
between workers for sequential use, including opening on one worker and reading
or dropping on another. Shared concurrent access to one handle is prohibited;
independent handles may be used concurrently. Run synchronous reads on a suitable
worker. No explicit close is needed: drop releases resources.
It does not lock the repository against other handles/processes, so a sequence
of reads is not a snapshot. Returned records own their memory. Read methods:

```rust,ignore
impl Repository {
    pub fn open_exact(path: &Path) -> Result<Self, Error>;
    pub fn open_from_env(path: &Path) -> Result<Self, Error>; // A1
    pub fn resolve(&self, revision: &str) -> Result<Option<ObjectId>, Error>; // A1
    pub fn git_dir(&self) -> &Path;
    pub fn common_dir(&self) -> &Path;
    pub fn work_dir(&self) -> Option<&Path>; // None for bare repositories
    pub fn object_format(&self) -> ObjectFormat;
    pub fn read_commit(&self, id: ObjectId) -> Result<CommitRecord, Error>;
}
```

Paths are native filesystem paths, not UTF-8 strings. No canonical-path or
stable-on-disk-identity guarantee is made. Accessor borrows end with Repository.

### A1: environment-honouring open

`open_exact` ignores Git's environment variables. `open_from_env(path)` is the
same open (exact path, no parent search, linked worktrees, no creation) plus
libgit2's `FROM_ENV` flag, so the native layer reads, when the open runs,
the variables the `git` program reads for a repository:
`GIT_WORK_TREE` (the work tree, visible in `work_dir()`), `GIT_COMMON_DIR`
(`common_dir()`), `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
`GIT_INDEX_FILE`, `GIT_NAMESPACE`, and the configuration locations
(`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`). It is the
open native commit and tag use. The crate reads and sets no variable itself;
the process environment is the native layer's input, so a caller that wants a
hermetic open uses `open_exact` or runs in a controlled process. Two facts from
the vendored libgit2 1.9.7 and tests (`tests/open_resolve.rs`): `GIT_DIR` is
consulted only when no path is given, so it does not choose the repository in
either open (a path is always given), and
`GIT_CEILING_DIRECTORIES` has no effect because there is no search. In a
clean environment the two opens return handles that agree on every accessor.
A handle from `open_from_env` has the same ownership rules as any `Repository`.

### A1: revision resolve

`resolve(revision)` turns a revision expression into the id of the object it
names: a ref name (`HEAD`, `main`, `refs/heads/main`, `v1`), a unique id prefix
of at least four hex digits, or an expression with suffixes (`main^{commit}`,
`HEAD~2`). It peels nothing unless the expression says so: an annotated tag
name returns the tag object's id and `v1^{commit}` its commit. It returns
`Ok(None)` when the revision names nothing, including a ref that does not
exist, an unknown prefix and an unborn `HEAD`; this is the native
"not found/unborn" distinction core's `read_ref` maps to `None` today. An
expression the native layer cannot parse or cannot decide (for example an
ambiguous prefix or `main@{`) is an `ObjectRead` error with native diagnostics.
The result carries the repository's object format. `resolve` reads refs and
objects only: no fetch, no abbreviation setting, no helper, no mutation. It is
a separate method from `read_commit`, which still takes a full id and does not
resolve refs or peel tags.

## IDs and commit data

`ObjectFormat` has `Sha1` and `Sha256` variants and is non-exhaustive. No implicit
default format. `ObjectId` is opaque and Copy/Clone/Eq/Hash; equality includes
the format. `parse_hex(format, text)` accepts exactly 40 or 64 ASCII hex digits,
respectively, either case, without whitespace/prefix/revision syntax. Display
returns full lowercase hex. `format()` and `as_bytes()` expose the format and
borrowed 20/32 bytes. Parsing validates representation, not object existence.

```rust,ignore
impl ObjectId {
    pub fn parse_hex(format: ObjectFormat, text: &str) -> Result<Self, Error>;
    pub fn format(&self) -> ObjectFormat;
    pub fn as_bytes(&self) -> &[u8];
}

pub struct CommitRecord {
    pub id: ObjectId,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Signature,
    pub committer: Signature,
    pub message: Vec<u8>,
    pub encoding: Option<Vec<u8>>,
}
pub struct Signature {
    pub name: Vec<u8>,
    pub email: Vec<u8>,
    pub seconds: i64,
    pub offset_minutes: i32,
}
```

Records are owned, Debug/Clone, and non-exhaustive. Names/email are native
parsed identity bytes, with no lossy UTF-8 conversion. Message is the raw
message, including leading newlines. Encoding is the first `encoding ` header's
raw value or None. Times are seconds since Unix epoch and signed offset minutes.
Parent order is the stored commit order. This is not the complete raw object
serialization (extra headers/signatures and original date spelling are omitted).

`read_commit` does not peel tags or resolve refs. Wrong-format IDs fail before
native lookup. Missing objects, malformed objects and non-commit objects fail;
there is no empty/default commit. Parent/tree IDs are returned without requiring
those referenced objects to be present. No promise of unlimited object size or
recoverable out-of-memory allocation is made.

## Errors

`Error` is opaque, implements Debug/Display/std::error::Error and owns its data.
`kind() -> ErrorKind` is the machine-readable category. `native() ->
Option<&NativeDiagnostic>` retains native failures with public `code: i32`,
`class: i32`, `message: String`. Code and class are different domains; message
text is diagnostic, not a stable match key. Display is diagnostic too.

`ErrorKind` is Copy/Eq and non-exhaustive:

| Kind | Meaning |
| --- | --- |
| InvalidObjectId | ID text has invalid characters or length |
| ObjectFormatMismatch | Valid ID uses another repository format |
| RepositoryOpen | Native opening failed for a reason other than "no repository" (for example an unsupported repository extension) |
| NotARepository | A1: the path is not a repository: a plain directory, a nonexistent path, or a nested directory (no parent search). Native code `-3` (not found) |
| ObjectRead | Object or revision lookup/type/parsing failed, including missing objects and, since A1, a revision `resolve` cannot parse |

The first two kinds have no native diagnostic; the last three retain native
code/class/message. Before A1, `RepositoryOpen` also covered nonexistent paths
and plain directories; both now report `NotARepository`, so a caller that asks
"is this a repository?" matches one kind and does not compare the native code.
No consumer matched `RepositoryOpen` for that purpose when A1 was made. Future callers must handle unknown kinds. G0 reads do not
retry, translate failures into EOF, execute helpers or mutate data to recover.
Later operation APIs will add their own documented errors; no future mutation
or cancellation behavior is implied by these five read-only categories.
