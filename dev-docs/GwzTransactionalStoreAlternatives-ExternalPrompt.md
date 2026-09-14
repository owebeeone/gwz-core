# Alternative transactional architectures for a multi-repository Git workspace tool

You are a senior systems architect with depth in database internals (WAL, MVCC,
group commit, two-phase commit, deterministic databases), version-control
internals (Git object and ref storage, packfiles, reftable, worktrees),
filesystems (fsync semantics, reflinks, snapshots) and distributed systems. I
want architectural alternatives, not code.

## 1. The tool

GWZ ("Git Workspace Zone") coordinates many ordinary Git repositories as one
workspace.

- **Root repository.** A small Git repository owns a tracked `gwz.conf/`
  directory containing:
  - a *manifest*: members, each with an id, path, source identity, remote URL
    and active/inactive state;
  - a *lock*: the exact commit of every active member;
  - named *snapshots*: member→commit maps;
  - *commit markers*: one per coordinated commit, used as identity evidence and
    for history coalescing.
- **Members.** Every member is an ordinary Git repository in a subdirectory.
  This is a product promise today: people and tools run raw `git` inside
  members, and members push to ordinary hosts such as GitHub.
- **Engine.** A Rust core with Python bindings, exposed as a message service
  (deterministic CBOR/JSON requests, responses and events). CLIs, UIs, AI agents
  and remote clients share one operation model, and embedders choose the
  transport.
- **Platforms.** macOS (APFS), Linux (ext4, XFS, btrfs), Windows (NTFS, ReFS Dev
  Drive), Raspberry Pi. Today it mostly runs on developer laptops and single
  machines, often with many AI agents working concurrently.

Cross-repository operations select "root plus all members" by default, or a
subset:

| Operation | What it does across repositories |
| --- | --- |
| `commit` | Commits staged changes in each selected member, then commits the root last so the root records the new lock and a marker. Every commit carries the same coordination trailers. |
| `merge <ref>` | Coordinated merge of a ref into each selected repository's current branch. It is a durable state machine (Executing, AwaitingResolution, Halted, Finalizing, Preserving, RecoveryRequired, RollingBack, Completed, Aborted) with `--status`, `--continue` and `--abort [--preserve]`. It publishes the updated lock and marker in one root commit at the end; while the merge is open, the accepted lock stays at the pre-merge baseline. |
| `pull` | Fetches, then fast-forwards, merges or rebases each member. `--sync merge` predicts conflicts in memory, freezes the fetched commits and revalidates before the first local mutation. |
| `push` | Captures refs first, pushes members, and withholds the root if any member push fails. |
| `branch`, `tag` | Create, delete and switch across members; tags include the root. |
| `stash` | One bundle id plus a native `git stash` entry per dirty member; bundle metadata lives in the workspace's runtime directory. |
| `snapshot`, `capture`, `materialize` | Record member→commit maps; check out the lock, a snapshot, a tag or a branch across members. |
| `repo add/clone/create/attach/detach/sync` | Membership lifecycle. Re-attaching a historical member verifies that every commit recorded for it in snapshots and markers exists. |
| `log` | One newest-first history across repositories, coalescing coordinated commits. |
| `local clone`, `merge --remote <lane>`, `local dispose` | *Lanes*: a copy of the entire workspace for an isolated agent, merged back by name. The copy is reflink/copy-on-write where the filesystem supports it and an ordinary copy elsewhere. A lane may be deleted only when its history is provably preserved in another lane. |
| `forall` | Runs a command in every member. |

Global behaviours:

- **Dry run.** `--dry-run` plans an operation without mutating anything.
- **Partial runs.** `--partial` lets some members proceed when others fail; by
  default a partial mutation is rejected.
- **Output.** Every operation returns typed per-member results in JSON or JSONL.

## 2. The problem

Git uses the filesystem as its database:

- loose and packed object files;
- one file per ref, updated by lock file plus rename;
- an index file;
- a worktree rewritten one file at a time.

No transaction spans two repositories. Even inside one repository, a multi-ref
update is not crash-atomic on the default files backend. A GWZ operation over K
repositories therefore touches about three independent non-transactional stores
per repository (objects, refs, index/worktree), plus GWZ's own runtime files,
plus remote hosts.

GWZ compensates with a hand-built saga:

1. It writes a durable per-operation record before execution.
2. It writes an intent record before each Git mutation and an outcome record
   after it.
3. After a crash, it classifies each pending action as not started, expected
   conflict, completed exactly, or ambiguous.
4. It rolls back in reverse order.
5. It creates preservation refs and stashes before a destructive abort.
6. It publishes the root last.

The consequences (the cost claims are inferred from documented behaviour, not
measured):

- **Slow as K grows.** Durability barriers (`fsync`, or the expensive
  `F_FULLFSYNC` on macOS) are ordered and O(K) per transaction. They cannot be
  batched, because each intent must be durable before its own mutation. Every
  command also opens and scans K repositories, sequentially, under one workspace
  lock held for the whole command.
- **Filesystem-dependent crash recovery.** Proving what an interrupted operation
  created needs durable volume identity and persistent file handles. Crash
  recovery is therefore unsupported on btrfs, on overlayfs without `nfs_export`,
  on FUSE and on network mounts.
- **Manual recovery cases.** Content filters (git-crypt), CRLF smudging, LFS
  pointers and Git's stat cache make byte-exact worktree restoration and
  verification fragile. The result is an operator runbook, including "park a
  merge record that no command can close".
- **Unprotected against external writers.** The lock serialises cooperating GWZ
  commands, but it cannot stop editors or raw `git`.
- **Split state that can disagree.** Stash metadata versus native stash
  payloads, the lane index versus lane pointers, the lock versus member HEADs.

## 3. The question

Propose **five architectural alternatives** to the "N+1 Git repositories on a
filesystem" substrate that give quick, reliable cross-repository transactions.

**Optimise for transaction speed across many repositories:**

- latency of one coordinated transaction as K grows (K = 5, 50, 500 in a
  500-repository workspace);
- throughput when many agents transact concurrently.

Reliability is the second criterion: the workspace must be crash-safe at any
instant, with no manual recovery runbook.

## 4. Already proposed: do not repeat these or produce variants of them

Swapping in a different product for the same architectural role is a variant,
not a new alternative. Examples: LMDB or RocksDB in place of SQLite in A1,
CockroachDB or Spanner in place of PostgreSQL in A2, a different blob store in
A3.

- **A1. Local transaction ledger, repositories as projections.**
  - An embedded SQLite (WAL) database in the workspace is authoritative for every
    ref, the composition, operations, stashes and lanes.
  - Objects stay in each repository's own object database.
  - On-disk refs, indexes, worktrees and `gwz.conf/` are projections, rolled
    forward after one SQLite commit.
  - Raw `git` writes are detected by fingerprint and imported as foreign
    operations.
  - Versioned views give MVCC and an operation-log undo, in the style of
    Jujutsu.
  - A merge moves no branch refs until one closing commit, so abort just means
    "project the current view".
  - About two durability barriers per transaction; single host.
  - Variant: the ledger kept as a reftable-backed Git repository of gitlink
    "views".
- **A2. PostgreSQL for everything.**
  - A workspace server stores refs, objects (bytea or packs), a commit-graph
    table, the composition and operations.
  - Local checkouts are clients, and merges run on the server.
  - One PostgreSQL transaction spans all repositories, with group commit and
    row-level locks.
  - Members become Git-protocol endpoints rather than repositories on disk.
  - Hosting providers become mirrors fed by an outbox.
- **A3. Split plane: metadata in a transactional DB, bytes in a
  content-addressed store.**
  - PostgreSQL or FoundationDB (SQLite in single-host mode) holds refs,
    composition, operations and GC roots.
  - Immutable objects live in a shared content-addressed store: a local
    Git-format store shared through `objects/info/alternates`, an optional
    raw-blob tier for reflink checkout, and S3 for multi-host use.
  - Objects are written first and refs last; GC uses grace periods and leases.
  - Similar to Mononoke.
- **A4. One Git repository of record, members as projected views.**
  - The whole workspace is one Git history (reftable backend), so a
    cross-repository transaction is one commit plus one ref update.
  - Per-member repositories are deterministic path-filtered views, pushed and
    pulled through a filter proxy, in the style of josh.
  - This inverts the "members are ordinary repositories" promise, and it costs
    history identity, signatures and private-member isolation.
- **A5. Transactional workspace daemon with virtual worktrees.**
  - A long-lived daemon holds all refs, indexes and composition in memory, with
    a WAL, group commit and asynchronous checkpoints to Git formats.
  - Worktrees are served through a virtual filesystem (FUSE, NFS loopback,
    ProjFS), so the working copy is inside the transaction boundary.
  - Lanes are O(1) views, as in EdenFS or VFS for Git.

All five share six principles. You may break any of them if you explain what
that buys:

1. One commit point per workspace transaction.
2. Immutable objects are written first, in parallel.
3. Projections roll forward, never back.
4. Plan against a snapshot, and hold the critical section only across the
   commit.
5. Snapshot uncommitted work into objects before a transaction can touch it.
6. External pushes go through an outbox.

Already considered and rejected:

- **Filesystem snapshots or shadow paging** (ZFS or btrfs snapshots,
  symlink-flip generations): open handles and working directories pin old
  generations, and the approach is not portable.
- **Two-phase commit across per-repository participants:** still O(K) prepare
  barriers.
- **Reftable inside each member only:** atomic within a single repository only.
- **Custom libgit2 ref or object backends as the store of record:** refs become
  invisible to raw `git`.

## 5. What counts as a new alternative

A new alternative must differ from every one of A1–A5 on at least one structural
axis:

- where the commit point lives;
- the concurrency model;
- what a worktree is;
- where object bytes live;
- which layer provides atomicity;
- who coordinates.

Directions worth exploring (not exhaustive, none required):

- commit-before-execute versus execute-before-commit ordering;
- deterministic or pre-ordered execution;
- merge-based or CRDT-style convergence instead of locking;
- peer-to-peer or replicated designs across agent machines;
- extensions to Git's formats or protocol;
- transactional features of the operating system or storage hardware;
- a Git host, code-review system or merge queue acting as coordinator;
- changing the unit of transaction: per ref, per repository, per workspace or
  per lane.

Privately brainstorm at least ten candidates. Discard any that are variants of
A1–A5 or implausible, then present the five most structurally distinct and
plausible. Also list the discarded candidates, one line each with the reason.

## 6. Reference workloads

Use these so that claims can be compared across alternatives.

- **W1:** a coordinated commit touching K of 500 repositories (K = 5, 50, 500),
  with small diffs.
- **W2:** a coordinated merge across 50 repositories where 3 conflict; the user
  resolves the conflicts and continues. Also cover the abort path.
- **W3:** 20 concurrent agents running W1 on one machine, on disjoint and on
  overlapping sets of repositories. Then the same load spread across 5
  machines.
- **W4:** power loss at an arbitrary instant during W1 or W2. What does recovery
  do, and can it ever need a human?
- **W5:** a human runs raw `git commit` in one member while a W2 merge is open.

Express costs as durability barriers per transaction, network round trips,
bytes copied and critical-section length, and state your assumptions. Where you
estimate, give orders of magnitude and label them as estimates.

## 7. Output format

For each of your five alternatives, cover:

1. **Name and shape.** One paragraph.
2. **Why it is not a variant of A1–A5.** Name the axis it changes.
3. **Structure.** Commit point, where bytes live, worktree model and concurrency
   model.
4. **Commit protocol for W1**, step by step.
5. **W2:** conflict, continue and abort behaviour.
6. **Cost:** W1 at K = 5, 50 and 500, and behaviour under W3.
7. **Failure and recovery (W4) and external writers (W5).**
8. **Git compatibility:**
   - how much of "members are ordinary Git repositories on disk" survives;
   - whether raw `git` still works;
   - interoperability with hosting providers.
9. **Changes to today's design:** what it eliminates and what it adds.
10. **Costs and risks:**
    - portability across macOS, Linux and Windows;
    - offline use;
    - operational burden;
    - implementation effort (S, M, L or XL).
11. **Prior art.** Name a real system or feature only if you are confident it
    exists and does what you describe. Mark anything uncertain, and do not
    invent APIs.

Then provide:

- **A comparison table** with your five alternatives plus A1–A5 as reference
  rows. Columns: commit point, barriers per W1, critical section, members stay
  ordinary Git on disk, multi-host transactions, operational burden, effort.
- **A speed ranking** of all ten, separately for commit-point latency,
  end-to-end latency and concurrent throughput.
- **Your recommendation**, with reasons, and the facts about the product that
  would change it.

Be concrete and technical. Skip introductions and do not restate the problem.
