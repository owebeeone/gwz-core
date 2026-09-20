# gwz-git: proposed foundation API

Status: proposed, unimplemented, unpublished. This guide describes the first
read-only package only. It does not provide commit/tag/history/fetch operations
yet. It runs synchronous local repository reads; call it on a suitable worker.

The initial development checkout will be a sibling `gwz-git` member in the
prepared GWZ workspace, with its own Cargo workspace and lockfile. It requires
Rust 1.95 and the qualified sibling git2-rs checkout. There is no crates.io
install or remote-only bootstrap yet. Downstream production use awaits source
distribution and platform qualification. No service is installed or configured.

## Open, inspect, close

Proposed usage (not runnable until the first package is implemented):

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
`GIT_DIR` does not choose a different repository. Opening does not create one.
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
    pub fn git_dir(&self) -> &Path;
    pub fn common_dir(&self) -> &Path;
    pub fn work_dir(&self) -> Option<&Path>; // None for bare repositories
    pub fn object_format(&self) -> ObjectFormat;
    pub fn read_commit(&self, id: ObjectId) -> Result<CommitRecord, Error>;
}
```

Paths are native filesystem paths, not UTF-8 strings. No canonical-path or
stable-on-disk-identity guarantee is made. Accessor borrows end with Repository.

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
| RepositoryOpen | Native opening failed, including nonexistent paths |
| ObjectRead | Commit lookup/type/parsing failed, including missing objects |

The first two kinds have no native diagnostic; the latter two retain native
code/class/message. Future callers must handle unknown kinds. G0 reads do not
retry, translate failures into EOF, execute helpers or mutate data to recover.
Later operation APIs will add their own documented errors; no future mutation
or cancellation behavior is implied by these four read-only categories.
