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

**Amendment A2 (2026-10-25, convergence plan step 5.2; D-G, D-K).** Error
translation helpers and native commit and tag move to gwz-git: `Error::message`,
`Repository::is_repository`, `ErrorKind::Operation` and the `native_commit`
module (section "A2: native commit and tag" below). The "does not provide
commit/tag" sentence above describes G0 and is superseded for those two
operations; history, fetch and the remote adapter are still not provided.

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

## A2: native commit and tag (step 5.2, row G1)

Moved from gwz-core's `git/gitbackend/native_commit` with their characterization
tests (133 tests run on macOS; the Windows count is in the step's evidence). Local Git only; no network, no credential, no history API.
The decision of 2026-10-12 (libgit2 when nothing needs the Git CLI, the CLI
otherwise) and of 2026-10-13 (`commit -a` always runs the CLI) move unchanged.

```rust,ignore
pub mod native_commit {
    pub fn commit(services: &Services<'_>, path: &Path, message: &str, all: bool)
        -> Result<CommitOutcome, Error>;
    pub fn tag_create(services: &Services<'_>, path: &Path, name: &str,
        message: Option<&str>, signed: bool) -> Result<TagOutcome, Error>;
    pub fn tag_delete(services: &Services<'_>, path: &Path, name: &str)
        -> Result<(), Error>;

    #[non_exhaustive] pub struct Services<'a> {
        pub environment: &'a GitEnvironment,
        pub policy: RoutePolicy,
        pub notes: Option<&'a RouteNotes>,
        pub process: &'a dyn ProcessGate,
    }                      // Services::new(env, process).with_policy(..).with_notes(..)
    pub trait ProcessGate { fn output(&self, command: &mut Command) -> io::Result<Output>; }
    pub struct GitEnvironment;   // from_vars(iter of (name, value)); no process() capture
    pub struct RouteNotes;       // new(Fn(&str)): one line per "ran the Git CLI because ..."
    pub struct RoutePolicy;      // AUTO (default), CLI_ONLY
    #[non_exhaustive] pub struct CommitOutcome { pub id: ObjectId }
    #[non_exhaustive] pub struct TagOutcome { pub name: String, pub commit: ObjectId }
}
```

- **Route.** The route rule (`route.rs`) decides native or CLI before anything is
  written; every refusal has a reason (`CliReason`, with its one-line text),
  delivered to `Services::notes` and nowhere else. The reasons are produced by the
  rule, so they live with it; what stays in gwz-core is the decision whether
  anyone is told (`Git2Backend::with_route_notes`, re-exporting `RouteNotes` for
  gwz-cli). The outcome carries no route.
- **Captured inputs, no globals.** gwz-git never reads the process environment:
  the caller captures `GitEnvironment` (gwz-core: once, when a backend is built)
  and passes it. The Git CLI route's child is launched by the caller's
  `ProcessGate`; gwz-core implements it with the host's `SpawnGate` (held for the
  spawn, never for the wait). gwz-git's own `Command::new("git")` sites (three) are
  the entries of its spawn allowlist, each `debt` (the child inherits the live
  process environment; RemPlan-1, Safety P3-11).
- **Self-verification (AD1).** After a native or CLI change gwz-git opens the
  repository afresh and checks the result with `Repository::resolve`: HEAD is the
  new commit; the tag exists and resolves to a commit; the tag is gone. No other
  read API is needed, so G1 does not wait for the refs-and-reads row (G2).
- **Errors.** Every failure is `ErrorKind::Operation`, with the native
  diagnostics when libgit2 reported it, and `Error::message()` is the text
  gwz-core copies into its `ModelError` (code `GitCommandFailed`, unchanged).
  `Error::message()` and `Repository::is_repository(path)` (open as
  `open_exact`; `Ok(false)` for `NotARepository`) also moved from gwz-core's
  error translation and `is_repository` (row G9).
- **Not moved.** Staging (`stage_paths`), repository creation and the scoped and
  merge commits stay in gwz-core (rows G3 and G10). The native `-a` staging code
  is kept, reachable only by a test-only policy, as decided on 2026-10-13.

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
| Operation | A2: a native commit or tag operation was refused or failed; the message says why, native diagnostics present when libgit2 reported it |
| ObjectRead | Object or revision lookup/type/parsing failed, including missing objects and, since A1, a revision `resolve` cannot parse |

The first two kinds have no native diagnostic; the three read kinds retain native
code/class/message. Before A1, `RepositoryOpen` also covered nonexistent paths
and plain directories; both now report `NotARepository`, so a caller that asks
"is this a repository?" matches one kind and does not compare the native code.
No consumer matched `RepositoryOpen` for that purpose when A1 was made. Future callers must handle unknown kinds. G0 reads do not
retry, translate failures into EOF, execute helpers or mutate data to recover.
Later operation APIs will add their own documented errors; no future mutation
or cancellation behavior is implied by the read-only categories; `Operation` is the one mutation category.
