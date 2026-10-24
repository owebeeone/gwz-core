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

**Amendment A3 (2026-10-25, convergence plan step 5.3; D-G, D-K).** The refs and
reads of row G2 move to gwz-git: `Repository::{head, tag_names, commit_exists,
blob_at, is_ancestor, ahead_behind, merge_base, changed_paths, status, remotes}`,
the owned records `Head`, `AheadBehind`, `Status`, `FileStatus`, `StatusOptions`
and `Remote`, and `ErrorKind::{InvalidPath, StateRead}` (section "A3: refs and
reads" below). Read-only, local, synchronous; nothing else in this guide changes.
Step 5.5 completes the row: `Repository::index_snapshot` with the owned records
`IndexSnapshot` and `IndexEntry` (core's `repository_index`), and the storage
locations (core's `repository_paths`), which need nothing new: `git_dir`,
`common_dir` and `work_dir` are the calls.

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
    pub fn is_repository(path: &Path) -> Result<bool, Error>; // A2: open_exact; Ok(false) for NotARepository
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
open native commit and tag use inside gwz-git (they open through it); gwz-core
has no caller of its own. The crate reads and sets no variable itself;
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
        // policy: RoutePolicy      crate-private; set only by with_policy (test-support)
        pub notes: Option<&'a RouteNotes>,
        pub process: &'a dyn ProcessGate,
    }                      // Services::new(env, process).with_notes(..); .with_policy(..) is test-support only
    pub trait ProcessGate { fn output(&self, command: &mut Command) -> io::Result<Output>; }
    pub struct GitEnvironment;   // from_vars(iter of (name, value)); no process() capture
    pub struct RouteNotes;       // new(Fn(&str)): one line per "ran the Git CLI because ..."
    pub struct RoutePolicy;      // AUTO (default); CLI_ONLY exists only with the `test-support` feature
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
  repository afresh and checks the result: HEAD is the new commit
  (`Repository::resolve("HEAD")`); the tag exists, by an exact lookup of the
  reference `refs/tags/<name>`, and peels to a commit; the tag is gone, by the
  same lookup. Tag existence is never a revision lookup: the revision rules would
  let a name shaped like `git describe` output (`t-1-g<abbrev>`) or a branch
  named `refs/tags/<name>` stand in for a tag that is not there.
- **Errors, by side of the write.** A refusal or failure of the operation
  itself is `ErrorKind::Operation`, with the native diagnostics when libgit2
  reported it. The reads that precede the write (the open for the route
  decision, the CLI route's HEAD read before `git commit`) keep their own kinds
  (`NotARepository`, `RepositoryOpen`, `ObjectRead`): nothing was written. A
  verification read that fails after the write is also `Operation`, with a message
  that states the write happened: `<what>: written; verification read failed:
  <native message>`; the native diagnostics of the failed read are kept. A
  consumer can therefore tell "nothing changed" from "written, not confirmed".
  `Error::message()` is the text
  gwz-core copies into its `ModelError` (code `GitCommandFailed`, unchanged).
  `Error::message()` and `Repository::is_repository(path)` (open as
  `open_exact`; `Ok(false)` for `NotARepository`) also moved from gwz-core's
  error translation and `is_repository` (row G9).
- **Not moved.** Staging (`stage_paths`), repository creation and the scoped and
  merge commits stay in gwz-core (rows G3 and G10). The native `-a` staging code
  is kept, reachable only by a test-only policy, as decided on 2026-10-13. Forcing a
  route at all (`RoutePolicy::CLI_ONLY`, `Services::with_policy`) is behind the
  non-default cargo feature `test-support`, which gwz-core enables in its
  `[dev-dependencies]` only: production gwz-core cannot force the Git CLI route.

## A3: refs and reads (step 5.3, row G2)

Moved from gwz-core's `Git2Backend` (`head`, `status`, `status_with_options`,
`tag_list`, `read_ref`, `commit_exists`, `read_file_at_commit`, `is_ancestor`,
`ahead_behind`, `merge_base`, `changed_paths_between`, `remotes`; step 5.5 adds
`repository_index` and `repository_paths`). Each is a
method of `Repository` taking and returning owned values; none changes a ref, the
index or the worktree, none reads the process environment or runs a process, and
none touches the network. A handle from either open works.

```rust,ignore
impl Repository {
    pub fn head(&self) -> Result<Head, Error>;
    pub fn tag_names(&self) -> Result<Vec<String>, Error>;           // sorted
    pub fn commit_exists(&self, id: ObjectId) -> Result<bool, Error>;
    pub fn blob_at(&self, commit: ObjectId, path: &str) -> Result<Option<Vec<u8>>, Error>;
    pub fn is_ancestor(&self, ancestor: ObjectId, descendant: ObjectId) -> Result<bool, Error>;
    pub fn ahead_behind(&self, local: ObjectId, upstream: ObjectId) -> Result<AheadBehind, Error>;
    pub fn merge_base(&self, left: ObjectId, right: ObjectId) -> Result<Option<ObjectId>, Error>;
    pub fn changed_paths(&self, old: ObjectId, new: ObjectId) -> Result<Vec<String>, Error>;
    pub fn status(&self, options: &StatusOptions) -> Result<Status, Error>;
    pub fn remotes(&self) -> Result<Vec<Remote>, Error>;
    pub fn index_snapshot(&self) -> Result<IndexSnapshot, Error>;    // step 5.5
}

#[non_exhaustive] pub struct Head { pub branch: Option<String>, pub commit: Option<ObjectId>, pub is_detached: bool }
#[non_exhaustive] pub struct AheadBehind { pub ahead: u64, pub behind: u64 }
#[non_exhaustive] pub struct Remote { pub name: String, pub url: String, pub push_url: Option<String> }
#[non_exhaustive] pub struct StatusOptions { pub include_ignored: bool }  // new(), with_ignored(bool)
#[non_exhaustive] pub struct FileStatus {
    pub path: String, pub index_status: char, pub worktree_status: char, pub original_path: Option<String>,
}
#[non_exhaustive] pub struct IndexEntry {          // step 5.5
    pub path: Vec<u8>, pub id: ObjectId, pub mode: u32, pub flags: u16, pub flags_extended: u16,
    pub ctime: (i32, u32), pub mtime: (i32, u32), pub stat: [u32; 5],   // stat: dev, ino, uid, gid, size
}
#[non_exhaustive] pub struct IndexSnapshot { pub path: Option<PathBuf>, pub entries: Vec<IndexEntry> }
#[non_exhaustive] pub struct Status {
    pub is_dirty: bool, pub staged: usize, pub unstaged: usize, pub untracked: usize,
    pub ignored: usize, pub unresolved: usize, pub files: Vec<FileStatus>,
}
```

- **Head.** `head()` reports the branch `HEAD` is on (short name; `None` when
  detached) and its commit. An unborn branch is not an error: the branch comes
  from the `HEAD` file and the commit is `None` (as `resolve("HEAD")` is
  `Ok(None)`). `tag_names` lists `v1`, not `refs/tags/v1`, annotated tags included.
  `read_ref` of core is `resolve` (A1).
- **Typed ids.** The methods that name commits take `ObjectId`, so the text a
  caller holds is parsed once, with `ObjectId::parse_hex` in the repository's
  format: exactly 40 or 64 hexadecimal digits. An id of the other format fails
  before any native lookup with `ObjectFormatMismatch`. `commit_exists` is
  `Ok(false)` for a missing object and for one that does not peel to a commit
  (a tree, a blob); an annotated tag of a commit is `true`.
- **Committed content.** `blob_at` reads the tree of `commit`, never the index or
  worktree. `path` is relative with `/` separators; an absolute path or one with a
  `.`, `..` or root component is `InvalidPath` before any lookup. `Ok(None)` when
  the tree has no such path; a path that names a directory is an `ObjectRead`
  error ("not a file"), not a miss.
- **Graph.** `is_ancestor` is the strict relation (a commit is not its own
  ancestor). `ahead_behind(local, upstream)` counts the commits reachable from
  one and not the other (zero both for the same commit). `merge_base` is the best
  common ancestor, `Ok(None)` when the histories share none. `changed_paths`
  lists both sides of every delta between the two trees, sorted, without repeats
  (a rename names both paths); names that are not UTF-8 are converted lossily.
- **Status.** Untracked files and directories are recursed; renames are detected
  between `HEAD` and the index and between the index and the worktree, and a
  rename is one entry with `original_path`. `index_status` is one of `A M D R T`
  or a space and `worktree_status` one of `? ! M D R T` or a space (the
  `git status --short` columns). The counts include every entry; `files` lists
  those whose path is UTF-8 (a path that is not is counted, not listed).
  `is_dirty` is any staged, unstaged, untracked or unresolved entry; ignored
  entries are listed and counted only with `with_ignored(true)` and do not make
  a status dirty. A bare repository has no worktree and reports `StateRead`.
- **Remotes.** The configured remotes in configuration order, each with its URL
  and, when one is configured, its separate push URL. Resolving a remote's
  credentials or opening a connection is not part of this call.
- **Index snapshot (step 5.5).** `index_snapshot` reads the index and returns every
  entry in the index's order with the facts the index file stores, owned: the path
  as bytes, the object id (in the repository's format), the mode, both flag words
  (the stage of a conflicted entry is in `flags`), the change and modification
  times as seconds and nanoseconds, and the stat words. It is what core's
  `repository_index` built from the native index, and the same read: a bare
  repository gives an empty snapshot, as it did. A failure to read the index is
  `StateRead` with the native diagnostic. Core maps `id` to the raw bytes its model
  carries.
- **Storage locations (step 5.5).** Core's `repository_paths` (the worktree, Git
  directory and common directory of a repository, a linked worktree included) is
  the three accessors above (`work_dir`, `git_dir`, `common_dir`) copied into core's
  owned record; no gwz-git API was added for it.
- **Errors.** Failures keep the libgit2 diagnostic. `InvalidPath` and `StateRead`
  are new kinds (see Errors); object, reference and tree failures are
  `ObjectRead`. gwz-core translates every one with `model_error`:
  `InvalidPath` becomes `InvalidRequest` (core's code for a committed-file path
  that is not normalized and relative), the rest `GitCommandFailed` with the
  diagnostic text unchanged.

Differences from the core implementations they replace, all recorded in the
step's evidence: ids are strict full hexadecimal text in the repository's format
(core's `Oid::from_str` also took shorter text, zero padded, which never named an
object); core now opens the repository before it parses an id or checks a
committed path, so a bad id or path on a path that is not a repository reports
the open error where it used to report the id or path problem first; and the
unborn `HEAD` file read reports `StateRead` where core reported `IoError`. In step 5.5
the index snapshot carries the object id as an `ObjectId` where core's record held
the raw bytes (core converts back, byte for byte), and an index read failure reports
`StateRead` (code `GitCommandFailed`, the diagnostic text unchanged) as the old
`git_error` translation did.

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
| Operation | A2: a native commit or tag operation was refused or failed, or a verification read after the write failed (the message says the write happened); the message says why, native diagnostics present when libgit2 reported it |
| InvalidPath | A3: a path argument is not normalized and relative (absolute, or with a `.`, `..` or root component); no native diagnostic, no native call was made |
| StateRead | A3: reading repository state that is neither an object nor a reference failed: the worktree status (including a bare repository), the configured remotes, the index (step 5.5), or the `HEAD` file of an unborn branch |
| ObjectRead | Object, reference or revision lookup/type/parsing failed, including missing objects, since A1 a revision `resolve` cannot parse, and since A3 the reads of tag names, `HEAD`, the commit graph, trees and committed files |

The first two kinds and `InvalidPath` have no native diagnostic; the read kinds
retain native code/class/message when libgit2 reported one. Before A1, `RepositoryOpen` also covered nonexistent paths
and plain directories; both now report `NotARepository`, so a caller that asks
"is this a repository?" matches one kind and does not compare the native code.
No consumer matched `RepositoryOpen` for that purpose when A1 was made. Future callers must handle unknown kinds. G0 reads do not
retry, translate failures into EOF, execute helpers or mutate data to recover.
Later operation APIs will add their own documented errors; no future mutation
or cancellation behavior is implied by the read-only categories; `Operation` is the one mutation category.
