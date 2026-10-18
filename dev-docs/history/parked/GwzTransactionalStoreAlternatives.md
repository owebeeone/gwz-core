# GWZ Transactional Store Alternatives

**Status:** design exploration. Five alternatives; no plan, no decision.
**Date:** 2026-09-14
**Basis:** the `gwz-cli/docs` user documentation only (Concepts, merge, local,
stash, push, pull, Local Clones, Merge Recovery, Machine Output). No source code
was read and nothing was measured. Statements about where the current engine
spends time are inferences from documented behaviour and are marked
*(inferred)*.
**Goal:** replace "N+1 Git repositories on a filesystem" as gwz's transactional
substrate with something that gives quick, reliable cross-repository
transactions, optimised for transaction latency and throughput as the member
count grows.

## 1. The problem in database terms

### 1.1 Where workspace state lives today

| Store | Holds | Atomicity it offers |
| --- | --- | --- |
| Object database, per repository | blobs, trees, commits, tags | Immutable, content-addressed; writes are idempotent |
| Refs, per repository | branches, HEAD, tags, `refs/gwz/merge/…`, `refs/gwz/local-imports/…`, reflogs | One ref at a time (`.lock` + rename); a multi-ref update is not crash-atomic on the files backend |
| Index, worktree, sequencer state, per repository | staged/unstaged work, conflict stages, `MERGE_HEAD` | None; checkout rewrites files one by one, and filters, CRLF and the stat cache can make disk bytes disagree with what Git believes |
| Root repository `gwz.conf/` | manifest, lock, markers, snapshots, integrity marker | The root's own commit, ordered after the member commits it describes |
| `.gwz/` runtime files | merge records (`done/`, `quarantine/`), stash bundles, family index and pointers, locks, catalog/anchors | Hand-built: staged write, rename, flush, read-back |
| Outside gwz's control | hosted remotes, native stashes, Git config (filters), editors and raw `git` | None |

A cross-repository transaction spans roughly three independent stores per
repository plus gwz's own runtime files, and none of them can join a transaction
with another.

### 1.2 What that forces on gwz

- **A hand-built saga.** `gwz merge` persists a frozen plan, records each pending
  action before its Git mutation, classifies it after a crash (`NotStarted`,
  `ExpectedConflict`, `CompletedExactly`, `Ambiguous`), compensates in reverse
  order and publishes lock and marker last. Branch refs move repository by
  repository while the operation is open, which is why `RollingBack`,
  `Preserving` and `RecoveryRequired` exist.
- **Correctness bound to the filesystem.** Proving what an interrupted start
  created needs durable volume identity and persistent file handles, so crash
  recovery is unsupported on btrfs, overlayfs without `nfs_export`, FUSE and
  network mounts.
- **An operator runbook.** Merge Recovery cases A–F (foreign filters, divergent
  rollbacks, CRLF worktrees, unclosable records, LFS pointers) all follow from
  verifying and restoring non-transactional worktrees byte for byte.
- **A coarse lock.** A mutating command holds the workspace mutation lock for its
  complete service call, and the lock still cannot stop a raw writer.
- **Two-part artifacts that can disagree.** Stash bundle metadata versus native
  stash payloads (`stash_incomplete`), family index versus clone pointers, lock
  versus member HEADs.

### 1.3 Where transaction time goes *(inferred)*

The unit of cost is the **durability barrier**: an `fsync`, or on macOS an
`F_FULLFSYNC` that flushes the drive cache and commonly costs milliseconds. CPU
work and page-cache writes are cheap by comparison. For a coordinated change
touching K repositories:

- each of the K repositories is opened and its index/worktree scanned for
  preflight;
- each participant costs at least two *ordered* barriers — intent before the
  mutation, outcome after — and each record write is stage, rename, flush,
  read back; if the record is one growing file, bytes rewritten grow as K²;
- each ref update is a lock file, rename and possibly a directory flush;
- root publication and verification reads come last;
- all of it runs sequentially under one lock held for the whole call.

Ordered barriers cannot be batched: an intent must be durable before *its*
mutation, so Git's `core.fsyncMethod=batch`-style group flushing cannot help. A
saga over files is O(K) barriers and O(K) wall-clock per transaction by
construction, and throughput is one transaction per complete call.

## 2. Targets shared by every alternative

1. **One commit point per workspace transaction.** A single atomic durable write
   whose success defines the new refs of every participating repository *and*
   the composition (lock, marker). Cost O(1) in K.
2. **Immutable data first, unordered, parallel.** Objects are content-addressed:
   writing them needs no coordination, only durability before the commit point,
   and many writes can share one batched barrier. An abandoned transaction leaves
   only garbage for GC.
3. **Projections roll forward, never back.** Worktrees, indexes, loose refs,
   `gwz.conf/` files and hosted remotes are derived from committed state and are
   rebuilt idempotently after a crash, so they need no barriers of their own.
4. **Plan on a snapshot; hold the critical section only across the commit.**
   Compare-and-set on the versions of the touched refs replaces a whole-call
   lock, so transactions on disjoint repositories stop serialising.
5. **Uncommitted work becomes data before it is at risk.** A transaction that
   will rewrite a worktree first snapshots dirty state into objects (Jujutsu's
   working-copy commit), recorded at the same commit point. Abort and undo become
   lossless; most "dirty member" refusals and `--abort --preserve` disappear.
6. **External effects go through an outbox.** Pushes to hosted remotes can never
   join a local transaction. The commit point records publication intents
   (members before root, as today); a publisher drains them idempotently.

The alternatives differ in where the commit point lives, where the bytes live,
what a worktree is, and how much of "every member stays an ordinary Git
repository" survives.

## 3. The alternatives

### A1. Local transaction ledger, repositories as projections

**Shape.** One embedded database per workspace (SQLite in WAL mode with
`synchronous=FULL` and `fullfsync` on macOS; LMDB is the alternative) at
`.gwz/ledger.db`. It is the authority for every ref gwz manages in every member
and in the root, and for composition, operations, stash bundles, family rows,
snapshots, markers and the outbox. Objects stay in each repository's own Git
object database. On-disk refs, `HEAD`, indexes, worktrees and `gwz.conf/` files
become projections of the ledger.

Rows are versioned: each committed transaction creates a new *view* version, and
a ref row is valid for a version range. That gives MVCC snapshots for planning,
`op log` / `op undo` over whole-workspace transactions (the Jujutsu operation-log
model), and "the lock stays at baseline while a merge is open" for free.

**Commit protocol** (a K-repository `gwz commit`):

1. Read snapshot: load refs and composition for the selection from the ledger at
   version V. No per-repository ref files are read.
2. Import check: compare a cheap fingerprint per repository (`packed-refs`, loose
   ref directory, `HEAD`, index checksum). A mismatch means raw `git` wrote
   something; ingest it as its own foreign operation before planning, or refuse
   if both sides moved (divergent).
3. Write objects in parallel: trees from each index, commit objects with
   trailers, and the root commit, whose tree (lock, marker) is a pure function of
   the member commit ids. Use unsynced writes, then **one** batched hardware
   flush.
4. **Commit point:** `BEGIN IMMEDIATE`, check that every touched ref is still at
   its version-V row, insert the new ref rows, composition, marker, operation and
   outbox rows, `COMMIT`. This is one barrier, whatever K is.
5. Project in parallel with no barriers: refs through one `update-ref --stdin`
   transaction per repository, reflogs, the root checkout of `gwz.conf/`. Record
   `projected_version` per repository.
6. Recovery on the next command: re-project any repository whose
   `projected_version` lags the ledger; drain any undrained outbox rows.

**Merge under A1.** Executing produces objects and conflict worktrees, but moves
no branch ref. Clean participants' results are computed in memory and written as
objects. Conflicted participants are projected into ordinary Git conflict state.
`--continue` builds resolution commits as objects; a raw `git commit` that
concludes a conflict is imported and adopted by its parents. **Close** is one
commit point that moves every branch ref, the lock and the marker together.
**Abort** before close is "project the current view again": the ledger never
moved, so there is nothing to roll back, only worktrees to reconcile forward from
their snapshots. `RollingBack`, `Preserving` and most of `RecoveryRequired`
reduce to projection retries.

**Speed.**

- Commit point: sub-millisecond CPU plus one barrier.
- Object writes: one batched barrier across all K repositories on the same
  device.
- Projections: parallel and unsynced.
- Reads: selection and ref reads come from one query instead of K repository
  opens.
- Critical section: milliseconds, not the whole call. Disjoint transactions from
  concurrent agents interleave under optimistic compare-and-set.
- What remains O(K): the import fingerprint (cheap, and removable with a file
  watcher) and worktree work for commands that change checkouts.

**Reliability.**

- The commit point is SQLite's, which is heavily power-loss tested. It needs
  working POSIX locks and `fsync`, not durable volume identity or persistent
  handles, so the btrfs/overlayfs crash-recovery gap largely closes.
- Network filesystems remain unsafe for SQLite, as they are for today's recovery.
  Put the ledger in local app data keyed by workspace id when the tree is remote.
- Raw Git stays a supported but non-transactional writer: it is imported, never
  overwritten, and both-sides-moved is reported as divergence.
- Losing the ledger loses operation history, not work. Rebuild a base view from
  the repositories' refs plus root markers and snapshots.

**Git compatibility.** Highest of the five. Members stay ordinary repositories on
disk with ordinary refs, hosting and credentials. `gwz.conf/` is still committed
in the root, so sharing a workspace through a Git host is unchanged.

**What goes away.**

- Merge record YAML, `done/` and `quarantine/` parking, and the multiple-records
  wedge (a unique constraint).
- The durable-identity bar for crash recovery, and the catalog and anchor
  artifacts.
- Stash-bundle/payload drift (bundle rows and snapshot oids commit together).
- The family YAML index (rows).

**Costs and risks.**

- There are two sources of truth during projection lag (milliseconds to
  seconds): raw `git` can briefly see old refs.
- The import rules for foreign writes have to be designed carefully.
- The ledger schema needs versioning and migration.
- It is single-host only: two machines cannot share one transaction.
- It can be adopted incrementally: operation records first, then composition,
  then refs.

**Ledger variant.** The ledger can itself be a Git repository: a reftable-backed
`.gwz/ledger.git` whose views are trees of gitlinks, one commit per transaction,
and whose commit point is one ref update. It is slower than SQLite (two or three
barriers), but views are inspectable with Git and replicable with `git push`.

### A2. PostgreSQL for everything: a database-native workspace server

**Shape.** `gwz-core` runs as a service (the Taut `GwzCore` boundary already
anticipates this) in front of PostgreSQL. Repositories are rows, not directories:

```text
repo(repo_id, workspace_id, member_id, source_id, active, …)
object(oid PK, kind, size, data bytea /* zstd, small objects */, pack_id, pack_offset)
pack(pack_id, bytes /* or large object */)             -- bulk and large blobs
ref(repo_id, name, oid, version, PRIMARY KEY (repo_id, name))
commit_graph(oid, parent_oid, generation)              -- reachability without inflating commits
view(version, op_id, …)  composition(version, member_id, path, oid, …)
op(op_id, kind, state, request_cbor, base_version, commit_version)
outbox(op_id, seq, remote, repo_id, refspec, state)
```

Local checkouts are clients. `gwz materialize` streams trees into them. A client
commit or merge resolution is uploaded like a `receive-pack` and becomes a
transaction. Merges are computed server-side next to the objects (merge-ort over
a DB-backed object store with a hot cache), so conflict prediction needs no
object transfer.

**Commit protocol.**

1. Ingest objects with `COPY`/`INSERT … ON CONFLICT DO NOTHING`, either before
   the transaction (idempotent) or inside it.
2. `BEGIN`; `SELECT … FOR UPDATE` on the touched `(repo_id, name)` rows, or run
   `SERIALIZABLE`.
3. Compare-and-set every ref; insert view, composition, op and outbox rows.
4. `COMMIT`: one WAL flush, shared with concurrent transactions by group commit.

**Speed.**

- Commit latency is one round trip plus a WAL flush, flat in K.
- Throughput rises with concurrency, because group commit amortises the barrier
  and row-level locks let disjoint transactions from many machines run in
  parallel. This is the only alternative besides A3 where agents on different
  hosts transact against one workspace.
- A lane (local clone) becomes `INSERT INTO ref SELECT …` into a new namespace:
  O(refs) milliseconds and zero bytes. `merge --remote A` is a server-side merge
  with no transfer.
- The weak spots are bytes:
  - bulk checkout means streaming many small objects (it needs pack-chunked
    reads and a client-side cache);
  - large blobs bloat TOAST, vacuum and backups;
  - every command pays network latency from a laptop.

**Reliability.** ACID, point-in-time recovery, replication and backups are
PostgreSQL's job. Clients have no filesystem capability matrix: the recovery bar,
runbook case D and the family index's interrupted states have nothing to attach
to. Reachability for `dispose` and GC is a recursive query over `commit_graph`.

**Git compatibility.** The lowest for local workflows. A member stops being a
repository of record on disk and becomes an ordinary Git *endpoint*
(`git clone https://gwz-host/ws/member.git` works). A raw `git commit` in a
checkout is local until pushed back, exactly like a normal remote. The promise
changes from "members are ordinary Git repositories" to "members speak ordinary
Git". Hosting providers are mirrors fed by the outbox.

**What goes away.** Everything bound to the local filesystem: merge records and
parking, crash-recovery capability probes, stash bundle files, family index and
pointers, dispose preservation walks over directories, and reflink lane copies.
Log coalescing becomes a join on `op`.

**Costs and risks.**

- gwz becomes a forge: HA, upgrades, backups, authentication and multi-tenant
  authorization all belong to it.
- Offline work needs a local cache plus later reconciliation.
- PostgreSQL is a poor blob store at scale, which is what A3 fixes.
- It is the largest rewrite of the five.

### A3. Split plane: metadata in a transactional DB, bytes in a content-addressed store

**Shape.** Only metadata ever needs a transaction: refs, composition, operations,
the outbox and GC roots, which amount to kilobytes. Object bytes (megabytes to
gigabytes) go to an immutable content-addressed store (CAS) that needs no
coordination. This is the Mononoke shape (SQL metadata plus a blobstore), applied
to a multi-repository workspace.

- **Metadata plane:**
  - PostgreSQL, for the simplest operations.
  - FoundationDB: strict-serializable and horizontally scalable. Its 10 MB / 5 s
    transaction limits fit ref updates and forbid bytes by construction.
  - SQLite, for single-host mode. That mode is A1 plus a shared object plane, so
    A1 can be A3's local tier.
- **Object plane, local tier:** one CAS per machine in Git pack/loose format,
  shared by every workspace, member and lane through `objects/info/alternates`.
  Members stay real repositories with near-empty object databases of their own.
  Optionally, a raw-blob tier keyed by oid lets checkout `clonefile`/`FICLONE`
  filter-free files straight into worktrees: zero data copied on APFS, XFS, btrfs
  and ReFS, with an ordinary copy elsewhere.
- **Object plane, remote tier:** an S3-compatible bucket of packs addressed by
  content hash, with the pack index manifest in the metadata DB. PUTs are
  idempotent.

**Commit protocol.**

1. Plan against a read version.
2. Write objects to the local CAS with one batched barrier. For cross-host
   visibility, upload the missing packs in parallel; existence comes from the
   manifest, so there are no per-object probes.
3. Run one metadata transaction: compare-and-set the touched ref versions, then
   write refs, view, composition, op and outbox rows.
4. Project locally: refs and worktrees, as in A1.
5. GC: mark from the DB roots, sweep with a grace period. Never delete an object
   younger than the oldest open transaction or lease. Uploads from abandoned
   transactions age out.

The invariant is **object first, ref last**. A ref can never name bytes that are
not durable, and a lost race only leaks garbage.

**Speed.**

- The commit point is O(1) in K and a few milliseconds even for 10 000 refs.
- The byte path is embarrassingly parallel and never blocks another transaction's
  commit.
- Deduplication removes repeat fetches across repositories, lanes, workspaces and
  CI hosts. Twenty agent lanes share one object store; a lane costs its
  worktree's reflinks plus ref rows, with no object copy.
- Materialisation by reflink from the raw-blob tier turns checkout into metadata
  operations.
- In single-host mode the latency equals A1's. In multi-host mode it is one DB
  round trip; uploads proceed in parallel and never hold the commit.

**Reliability.**

- The database supplies the atomicity; the CAS is verify-on-read and immutable,
  so there is no torn-object ambiguity.
- GC correctness (grace windows, leases, alternates) is the subtle part and needs
  its own design and tests.
- Local-tier failure modes are Git's own alternates caveats: a `git gc` in a
  member must not assume borrowed objects are local, and moving or deleting the
  shared CAS breaks every borrower. So the CAS lives under gwz's control, and
  `git repack -a` "unshares" a member on detach or export.

**Git compatibility.** High locally: members are ordinary repositories with
alternates and projected refs, as in A1. Remote readers use a Git-protocol
gateway over the same planes (A2's endpoint model without A2's blob problem).

**What goes away.** Everything A1 removes, plus the per-lane object copies and
repeated network fetches, which is a large share of `local clone` and
`materialize` time on non-reflink filesystems.

**Costs and risks.**

- Two systems to run (DB and object store) in multi-host mode.
- The GC and lease design.
- Filtered paths (LFS, CRLF, git-crypt) cannot be reflinked from raw blobs and
  fall back to a normal checkout.
- Cold checkouts from S3 are slow without a warm local tier.

### A4. One Git repository of record, members as projected views

**Shape.** Stop coordinating N databases: keep one. The workspace's authoritative
store is a single Git repository whose commits contain the whole workspace tree,
with members as subdirectories. A cross-repository transaction is an ordinary
commit plus one ref update, atomic by Git's own rules. Per-member repositories are
**views**: deterministic path-filtered histories, served and pushed back through
a filter proxy (josh, "Just One Single History", is the prior art). Use the
reftable ref backend (Git ≥ 2.45) so the many-ref updates lanes and namespaces
create stay atomic and fast.

A member-id/source-id mapping file committed in the record repository replaces
manifest rows. The lock, markers and log coalescing become redundant: one commit
id *is* the composition.

**Commit protocol.**

1. One index, one `write-tree`. Unchanged member subtrees reuse their tree ids,
   so the cost is O(changed paths), not O(K).
2. Write one commit object, then update one ref. That is one or two barriers
   (object batch, ref) in total.
3. The outbox publishes the changed member views to their upstream hosts.

**Merge under A4.** One merge-ort pass over one tree, with untouched member
subtrees skipped by oid equality. One index holds all conflicts. `git merge
--continue` and `git merge --abort` are correct again, because there is exactly
one repository. Lanes are branches plus `git worktree add`, or a reflink copy of a
single repository. `merge --remote A` is `git merge lane/A`.

**Speed.** The best commit and merge latency of the five, with no coordinator at
all:

- **Status:** one index with fsmonitor and the untracked cache; Git handles
  millions of files this way.
- **Remote transactions:** `git push --atomic` to one central repository updates
  every member atomically, which is the only cheap multi-host transaction here.

**Reliability.** Git's single-repository crash semantics, with reftable making
ref updates atomic. The coordinator, the recovery state machine and nearly the
whole Merge Recovery runbook disappear. Filters and CRLF go back to being Git's
ordinary behaviour, because gwz no longer restores worktrees blob-exact behind
Git's back.

**Git compatibility.** It inverts the current promise: the *workspace* is an
ordinary Git repository, and a *member* is a path locally and a repository only
as a published view. The costs are concrete:

- **History identity.** View commit ids differ from record commit ids unless the
  filter is round-trip stable. Signatures on member commits and tags do not
  survive filtering. Existing member histories need a one-time join.
- **External contributors** who push directly to a member's upstream must be
  imported through the reverse filter and merged on import.
- **Per-repository tooling.** Anything that expects a member-root `.git` (IDEs,
  CI scripts, Cargo git dependencies) needs a materialised per-member clone,
  which is then a read-mostly projection.
- **Private members** (`private: true`, as with `gwz-core-evidence`) cannot be
  hidden inside one object database. Each audience needs its own filtered
  publication, and the record repository itself stays private.

**What goes away.** Merge records, recovery states, the runbook, markers,
coalescing heuristics, the stash bundle split (one native stash), identity
evidence walks for attach, and most of the lane machinery.

**Costs and risks.**

- A product change, not a storage change.
- A dependency on a reftable-capable Git implementation (Git ≥ 2.45; verify
  library support before assuming it).
- Operating the filter proxy and keeping its mapping cache correct.
- Migration of existing member histories.

### A5. Transactional workspace daemon with virtual worktrees

**Shape.** A long-lived `gwzd` (one per machine or per workspace) owns all mutable
workspace state:

- Refs, indexes, `HEAD`s, composition and operations live in memory.
- Durability comes from an append-only **write-ahead log with group commit**.
- Checkpoints to standard Git formats run asynchronously, so the on-disk
  repositories remain valid Git as of the last checkpoint.
- Worktrees are served through a **virtual filesystem**: FUSE on Linux, NFS
  loopback or FSKit on macOS, ProjFS on Windows.
- Prior art: EdenFS with Sapling, VFS for Git, Google CitC.

Every file write passes through the daemon, which puts the working copy itself
*inside* the transaction boundary. The daemon knows exactly which paths changed,
with no scans and no stat-cache races. It can snapshot dirty state as tree objects
instantly (target 5 at no cost). Checkout, switch and merge swap inode trees and
hydrate content lazily.

**Commit protocol.**

1. CLI, Python and agents send Taut `GwzCore` requests over a local socket.
2. The daemon executes against in-memory state under fine-grained locks or MVCC.
3. It appends one WAL record covering every ref, index and composition change.
   Concurrent transactions share the flush through group commit.
4. It acknowledges, then applies the change to the VFS views.
5. It checkpoints to Git formats and drains the outbox in the background.

**Speed.** The best *end-to-end* latency, not just the best commit point:

- No per-command process start or K repository opens: caches stay warm.
- `status` is O(changed files), because the VFS keeps a journal.
- `materialize`, `switch` and abort are O(changed directories), with lazy
  hydration.
- One barrier is shared by every concurrent agent.
- A lane is a new VFS view over the same objects plus a ref namespace: O(1),
  with no reflink copy. `merge --remote A` never transfers anything.

**Reliability.**

- **Crashes.** The daemon replays its WAL on restart. Editors and build tools
  write through the VFS, so the "cannot freeze an editor or raw writer" hole
  closes for worktree files.
- **Raw `git` on `.git` metadata** is either intercepted or imported, as in A1.
- **A daemon outage is a new failure mode:** while `gwzd` is down, worktrees are
  unavailable.
- **Upgrades** must hand live mounts over.

**Git compatibility.** Checkouts look ordinary and the Git CLI works. How well it
works depends on interception quality and on the performance of Git and build
tools over a VFS. Hosting is unchanged.

**What goes away.**

- Worktree scans and the stat-cache traps (runbook cases A–C are VFS-level
  decisions instead).
- Stash as a separate mechanism, dirty refusals and lane copy cost.
- Most of the merge state machine.

**Costs and risks.**

- The highest engineering cost of the five.
- Per-OS filesystem work, with macOS historically the weakest platform.
- Build output such as `target/` must be redirected to real disk; EdenFS calls
  these redirections.
- IDE indexers that read everything force full hydration.
- A daemon lifecycle and local socket security to own.

## 4. Comparison

Barriers are counted for one coordinated transaction touching K repositories,
excluding worktree file writes.

| | Commit point | Barriers | Critical section | Members stay ordinary Git on disk | Multi-host transaction | Ops burden | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Today *(inferred)* | none; saga over files | O(K), ordered | whole service call | yes | no | none | — |
| A1 ledger | SQLite WAL commit | 2 | commit only | yes (projected refs) | no | none | M |
| A2 Postgres | PG transaction | 1, group-shared | row locks | no (Git endpoints) | yes | high | XL |
| A3 split plane | DB transaction | 2 (local) | commit only | yes (alternates + projection) | yes | medium–high | L |
| A4 record repo | one ref update | 1–2 | one ref / one index | no (workspace is the repo) | yes, `push --atomic` | low | L + product change |
| A5 daemon | WAL append | 1, group-shared | in-memory | mostly (virtual checkouts) | no | medium | XL |

## 5. Speed ranking and recommendation

- **Commit point, local:** A1, A3-local, A4 and A5 are all O(1) in K and within a
  barrier of each other. A2 and multi-host A3 add a round trip.
- **End to end with many repositories** (including scans, checkout and repo
  opens): A5 > A4 > A1 ≈ A3 > A2.
- **Concurrent throughput with many agents:** A5 on one host, A2 ≈ A3 across
  hosts, then A1 (optimistic CAS), then A4 (one index per worktree), then today.

**Recommendation: A1, with the schema designed as A3's local tier, and target 5
(working-copy snapshots) adopted alongside it.** The reasons:

- It turns O(K) ordered barriers into two and shrinks the critical section from
  the whole call to the commit.
- It removes the filesystem crash-recovery bar and most of the Merge Recovery
  runbook.
- It keeps the product's central promise that members are ordinary Git
  repositories, and it adds no operational burden.
- It can land incrementally: operation records first, then composition, then
  refs.
- Keeping the ledger metadata-only over content-addressed objects preserves the
  path to multi-host A3 without committing to it now.

When to choose the others instead:

- **A4:** only if the ordinary-member-repository promise is relaxed. It is the
  fastest pure-Git design and the simplest to reason about.
- **A5:** only if, after A1, end-to-end status and checkout on very large
  workspaces is still the bottleneck.
- **A2:** only if gwz becomes a hosted, multi-tenant service.

## 6. What stays non-transactional in every alternative

- **Publication to hosted remotes.** An outbox with ordering and idempotent
  retries; `git push --atomic` is atomic only within one remote repository
  (A4's single record repository is the exception that benefits).
- **Hooks, LFS server uploads and signing.** They run before the commit point
  and behave like object writes; failures abort before commit.
- **Editor and build-tool writes** to worktrees, except under A5.
- **Git configuration** (filters, `autocrlf`, identities). It is an input to
  projection, never part of committed state.

## 7. Considered and not proposed

- **Filesystem snapshots or shadow paging** (ZFS/btrfs snapshots,
  symlink-flip generations, `RENAME_EXCHANGE`):
  - The commit point is O(1), but open handles and shells' working directories
    pin the old generation and silently lose writes.
  - Snapshots are volume-level, often privileged, and not portable.
  - It deepens the filesystem capability matrix gwz is trying to leave.
- **Two-phase commit across repositories:** K prepare barriers plus a
  coordinator log; the cost model does not change.
- **Reftable in each member only:** atomic within one repository; it lowers
  per-repository ref cost, not cross-repository cost.
- **Custom libgit2 ref/object backends over a database as the store of record:**
  refs become invisible to raw `git`. It is useful only as a read accelerator
  beneath A1/A3 projections.

## 8. Questions to settle before choosing

1. **Confirm §1.3 by measurement.** Count barriers and wall time for `commit`,
   `merge`, `materialize` and `local clone` at K ∈ {5, 50, 500} on APFS, ext4,
   XFS and ReFS/NTFS.
2. **Is "members stay ordinary Git repositories on disk" a hard constraint?** The
   answer decides A2 and A4.
3. **Are multi-host transactions a requirement** (agents on different machines
   against one workspace)? The answer decides A1 versus A3.
4. **What projection-lag semantics are acceptable** for raw `git` users under
   A1/A3, and what is the divergence rule when both sides move?
5. **What does the Git backend gwz links actually support?** Check reftable (A4
   and the ledger variant) and alternates behaviour (A3) before relying on
   either.
6. **What does a working-copy snapshot cost** on large members, with and without
   a filesystem monitor?
