# GWZ Transactional Store Alternatives (G46)

**Status:** design exploration. Five alternatives structurally distinct from
A1–A5 in `GwzTransactionalStoreAlternatives.md`. No plan, no decision.
**Date:** 2026-09-14
**Basis:** the external prompt in `GwzTransactionalStoreAlternatives-ExternalPrompt.md`
and the A1–A5 document. No source was read and nothing was measured. Costs are
orders of magnitude, labelled *estimate*.

A1–A5 are not restated. They remain the reference rows in §7.

**Assumptions used in every cost claim**

| Symbol | Meaning | Order of magnitude *(estimate)* |
| --- | --- | --- |
| Barrier | `fsync` / macOS `F_FULLFSYNC` on a laptop SSD | 1–10 ms; **5 ms** used when a single number is needed |
| Local RTT | Unix socket / loopback Git protocol | 0.1–1 ms |
| WAN RTT | Git host or regional object store | 20–100 ms; **50 ms** used |
| Small W1 pack | K new commits + trees, tiny diffs | ~1–4 KB per member; K=500 still well under 2 MB |
| Today *(inferred)* | Ordered intent/outcome barriers, sequential, whole-call lock | ~2K barriers; wall clock grows with K into hundreds of ms (K=5) to seconds (K=500) |

Object CPU, page-cache writes and JSON rendering are treated as cheap against
barriers and WAN RTTs unless noted.

---

## 1. Discarded candidates

Privately brainstormed, then dropped. One line each.

- **CORFU/Tango shared-log maps.** Same sequenced-log commit point as B1; a
  variant of deterministic pre-ordering, not a new axis.
- **Hosted merge queue as the only writer (GitHub merge queue, bors, Gerrit
  `submitwholetopic`).** Serialisation theatre: independent Git hosts still
  cannot flip K refs in one crash-atomic step, so it collapses to B3 or to A1
  beside the host.
- **Pijul/Darcs patch store as the record, Git as export.** Same architectural
  role as A4 (one history of record, members as views), different product.
- **Nix store + atomic GC-root symlink.** Nix already is SQLite metadata plus a
  CAS (A3 local / A1).
- **Percolator-style 2PC over a dumb KV.** Participants are rows not Git repos,
  but it is still O(K) prepares; if the KV is FoundationDB it is A3; if not, it
  reintroduces the rejected 2PC cost model.
- **Raft/LiteFS around an A1 ledger.** Product swap for A1's durability, not a
  new commit point.
- **Append-only Git-bundle WAL.** Format swap for A1's ledger role.
- **Workspace reftable hidden behind a libgit2 backend.** Already rejected:
  refs become invisible to raw `git`.
- **eBPF / `LD_PRELOAD` intercept of Git without a VFS.** A5 minus the worktree
  boundary; editors still write around the interceptor.
- **Persistent memory, TxF, NVMe fused writes, CXL.** Dead, deprecated, or not
  a transaction that spans K repositories on a laptop.
- **Blockchain / signed global consensus.** Latency and operational burden
  contradict the speed goal.
- **Kafka (or any log product) as the metadata plane.** Product swap for A3.
- **Perforce/Piper/Sapling segmented changelog as the record.** Product swap
  for A4's "one depot of record".
- **Overlayfs / device-mapper / VHDX generation flip.** Already rejected as
  filesystem snapshots / shadow paging.
- **Per-lane-only transactions with commutative integration.** Changes the unit
  of conversation, not the substrate; cross-member atomicity inside a lane is
  still unsolved.
- **WASM-replay of recorded Git traces.** Implementation of B1, not a store.

---

## 2. B1 — Deterministic sequencer (commit-before-execute)

### 2.1 Name and shape

A sequencer totally orders workspace transactions *before* any Git mutation.
Clients submit a stored procedure plus arguments (`commit`, `merge-plan`,
`merge-close`, …) and a declared read/write set of refs. The sequencer appends
that record to a durable log (the commit point), then an executor applies the
procedure in log order against ordinary member object databases and projected
refs. Concurrent transactions in one epoch whose write sets do not overlap
execute in parallel; overlapping ones see each other's outputs in sequencer
order and never OCC-abort after assignment. Single-host the sequencer is an
in-process mutex plus a log file; multi-host it is a Raft log. This is Calvin
(and Aria's epoch parallelisation) applied to GWZ, not a SQL replica.

### 2.2 Why it is not a variant of A1–A5

**Axis: commit-before-execute versus execute-before-commit, and who
coordinates.** A1/A2/A3 write objects, then commit refs with OCC or row locks.
A5 executes against in-memory state and *then* WALs; its distinctive worktree
is a VFS. A4 has no sequencer. B1's durable success is "this procedure is in
the log at epoch N"; Git on disk is a deterministic projector of the log, so a
crash mid-execute is always "replay from N", never "classify this mutation".

### 2.3 Structure

| | |
| --- | --- |
| Commit point | Log append at the sequencer (group-committed). |
| Bytes | Member Git object databases, written by the executor *after* assignment. Inputs that are not yet objects (index trees, conflict resolutions) travel in the log record, or as pre-copied staging objects named by oid in the record. |
| Worktree | Ordinary directories. Projection rolls forward after execute. Dirty state is captured into the log record before assignment (target 5 of A1–A5, but it is an *input*, not a side effect). |
| Concurrency | Total order on conflicting write sets; parallel execute otherwise. No compare-and-set abort after assignment. |

Breaks the A1–A5 "plan on a snapshot, hold the critical section only across
the commit" slogan in one place: the sequencer is a critical section for
*ordering*. It is microseconds of CPU plus the log's group-commit window, not
Git work. That is what it buys: executors never throw away merge CPU because a
ref moved.

### 2.4 Commit protocol for W1

1. Capture the selection's indexes / declared paths into tree objects in a
   *staging* CAS (unsynced). Include those oids in the procedure arguments so
   the log is self-contained.
2. Declare the write set: `HEAD`/`refs/heads/*` of each of the K members plus
   the root lock/marker refs.
3. Sequencer assigns epoch *e*, **appends the record, group-commits** (commit
   point). The client may treat the transaction as durable here. Staging objects
   not named by a committed record are GC.
4. Executor (same process or a replica) applies *e* in order: promote staging
   objects into each member ODB (or one shared ODB), write commit objects if
   the procedure computes them, project refs and `gwz.conf/`. No barriers
   required of projection.
5. If the process dies between 3 and 4, replay from the last executed epoch.
   Application is idempotent because objects are content-addressed and ref
   updates are "set to oid X".

### 2.5 W2 — conflict, continue, abort

A merge is three (or more) sequenced procedures, not a saga:

- `merge-open(plan)` logs the frozen plan and the pre-merge ref oids. It does
  not move branch refs. Executing it materialises conflict worktrees for the
  three members and records conflict state *in the log* (index stages as tree
  oids).
- `merge-continue(resolutions)` is a new transaction whose arguments are the
  resolution trees. The executor produces the remaining commits.
- `merge-close` moves every branch ref, the lock and the marker in one
  procedure. That is the only epoch that publishes composition.
- `merge-abort` is a procedure that names `merge-open`'s epoch and projects
  the pre-merge ref oids. There is nothing to roll back in Git: branch refs
  never moved. Worktrees roll forward from the snapshot captured at open.

A crash during execute of any of these is replay. `RecoveryRequired` and
`RollingBack` do not exist as operator-visible states.

### 2.6 Cost

W1 barriers: **two** (staging-object batch; log group-commit), **flat in K**.
If staging objects are included in the log payload, one barrier. Critical
section: sequencer assign + log flush, milliseconds, not the Git work.

| K | Commit-point *(est.)* | End-to-end dominated by |
| --- | --- | --- |
| 5 | 1 group-commit ≈ 5 ms | Staging capture + 5 commit-object writes, parallel |
| 50 | same | Parallel object CPU; projection of 50 ref files (unsynced) |
| 500 | same | Object CPU and worktree/index projection, still one log flush |

W3, one machine, 20 agents: group commit coalesces their log records onto one
barrier per flush window *(est. 1–5 ms)*. Disjoint write sets execute in
parallel after assignment; overlapping sets serialise in epoch order with no
retry. Throughput *(est.)*: 10²–10³ durable txns/s, bounded by the log device,
not by K.

W3, five machines: a 3-of-5 Raft log. Commit-point latency *(est.)* 1–2 WAN
RTTs (50–100 ms) plus the local execute. Throughput follows Raft batching, not
K. This is the first B-series design that gives *linearizable* multi-host
transactions without a SQL server.

### 2.7 Failure (W4) and external writers (W5)

W4: replay the log from the last applied epoch; re-project lagging members.
Ambiguous Git mutations cannot occur because Git is not mutated until after
the log is durable, and mutation is a pure function of the record. No human.

W5: raw `git commit` is not a sequenced procedure. Next GWZ operation either
imports it as a foreign procedure (declared write set = that member's HEAD) or
refuses on divergence if the sequencer already assigned a conflicting write.
The hole is the same class as A1: cooperating clients are safe; editors and
raw Git are not in the write set until imported. B1 does not close W5 without
a VFS or a Git-format lock (see B5).

### 2.8 Git compatibility

Members remain ordinary repositories on disk with ordinary remotes. Raw `git`
works, with projection lag measured in milliseconds after each epoch. Hosting
providers are unchanged; the outbox still pushes members then root. The log is
*not* pushed; it is local (or Raft-internal).

### 2.9 Changes to today's design

Eliminates the per-action intent/outcome saga, merge `done/`/`quarantine/`,
filesystem identity/capability gates for recovery, and OCC retry of expensive
merges. Adds a log format, a sequencer, stored-procedure implementations that
are deterministic (no wall clock, no unordered map iteration, no reading a
worktree that was not snapshotted into the record), and a GC rule for staging
objects.

### 2.10 Costs and risks

- Portability: a log file with `fsync` works on macOS, Linux, Windows, Pi.
  Raft is optional. No FUSE.
- Offline: single-host mode is fully offline. Multi-host linearizability is
  not; a partitioned replica must not execute gaps (standard Raft).
- Operational burden: none in-process; Raft is a cluster.
- Effort: **L**. Determinism of merge-ort and of filter/CRLF is the hard part;
  the sequencer is commodity.

### 2.11 Prior art

Calvin (2012) and Fauna's transaction protocol; Aria (2020) for epoch-parallel
execute; FoundationDB's recovery is *log then replica replay*, not this
client-declared write-set shape. Uncertain: whether `git2`'s merge-ort is
deterministic across versions for the same trees — treat merge code version as
part of the procedure id.

---

## 3. B2 — Multi-master CRDT refmap (merge, don't lock)

### 3.1 Name and shape

Refs, composition, stash bundle ids, lane pointers and in-flight merge records
live in one **op-based CRDT** (a map from `(repo, ref)` to a register of oid +
actor + lamport, plus an OR-set of operations). Each agent is an actor with
its own append-only op log. A coordinated W1 is *one* op that names all K new
oids; persisting that op is the local commit point. Peers merge logs; concurrent
ops on disjoint refs commute; concurrent ops on the same ref are resolved by a
deterministic rule (see §3.5), never by abort. Object bytes stay in Git ODBs
or a shared CAS and are written before the op that names them is persisted.
There is no primary, no CAS retry loop, and no SQL.

Deliberately breaks A1–A5 principle 1 *across machines*: there is no single
global instant at which every replica has the same composition. What that
buys is offline agents on five laptops without electing a leader. Locally, one
actor still has one commit point per op.

### 3.2 Why it is not a variant of A1–A5

**Axis: concurrency model and who coordinates.** A1 is single-host OCC that
*aborts* when a ref version moved. A2/A3 coordinate through a primary database.
A5 coordinates through one daemon's locks. A4 is one lock on one index. B2 has
no coordinator; conflicts *merge*. Replacing SQLite with Automerge and keeping
OCC would be an A1 variant; this is not that.

### 3.3 Structure

| | |
| --- | --- |
| Commit point | Local op-log append + one barrier, per actor. Global state is the merge of all ops. |
| Bytes | Git ODBs / shared CAS; object-first, op-last, per actor. |
| Worktree | Ordinary projections of that replica's merged view. |
| Concurrency | Causal broadcast. No locks across actors. Same-ref concurrency is a merge, not a retry. |

### 3.4 Commit protocol for W1

1. Read the actor's current merged view (in memory; replay if cold).
2. Write new objects in parallel; one batched object barrier.
3. Append one op `{actor, lamport, writes: [(repo, ref, oid), …], parents:
   observed op ids}`. **Fsync the op log** (commit point).
4. Project affected refs. Gossip the op (or ship it with the next lane merge).

Dry-run is "build the op, do not append".

### 3.5 W2 — conflict, continue, abort

Open merge is an op that inserts a `Merge(plan_id)` object into the CRDT,
holding pre-merge oids and conflict trees, and does **not** overwrite
`refs/heads/*`. Continue appends resolution oids onto that object. Close is an
op that deletes `Merge(plan_id)` and writes the branch refs + lock.

Abort is **not** CRDT-undo (undo is the usual CRDT trap). It is a forward op
`Abort(plan_id)` that is an inverse only in the application sense: it removes
the merge object and leaves heads where they were. Peers that already saw
`Merge` and not `Abort` still show an open merge until they receive `Abort`.
If a peer concurrently `Close`s, the deterministic rule is: `Close` and
`Abort` of the same `plan_id` with concurrent parents → **Abort wins** if
either actor had not observed Close (conservative), and the losing Close's
commit objects remain as garbage. Document this; do not invent a second
automatic merge of two closes.

Same-ref concurrent *commits* (two agents W1 the same member): do not LWW.
Produce a merge commit with both oids as parents, deterministically ordered by
`(actor, lamport)`, and set the ref to that merge. That is Git-native and
matches how lane integration already thinks.

### 3.6 Cost

W1 barriers: **two** (objects, op log), flat in K. No network on the commit
path.

| K | Commit-point *(est.)* | Notes |
| --- | --- | --- |
| 5 / 50 / 500 | ≈ 5 ms | Op payload is O(K) ref names; still KB, not a barrier issue |

W3, one machine, 20 agents: each actor fsyncs its *own* log (20 files) or a
shared log with a mutex. Shared-log form serialises the barrier but still
group-commits *(est. hundreds/s)*. Per-actor logs need no lock; merge happens
on read. Disjoint and overlapping sets both "succeed"; overlapping HEADs
become merge commits at read/project time.

W3, five machines: gossip. Commit latency stays local. A linearizable read of
"the" lock needs a quorum round (at which point you have reinvented Raft and
should use B1). Default GWZ reads are local-merged, which is the point.

### 3.7 Failure (W4) and external writers (W5)

W4: replay the actor's op log; objects are already durable or the op is not.
A crash between object fsync and op fsync leaks objects. Never a human, but
**concurrent offline writes of the same ref become automatic merge commits**,
which is a product surprise, not a runbook.

W5: a file watcher or next-command fingerprint wraps the raw commit as an op
from actor `git-foreign`. If a merge is open, that op is a parent of
`Continue` or is refused as divergence — pick one rule and keep it. Foreign
ops cannot be prevented, only absorbed.

### 3.8 Git compatibility

High locally: projected ordinary repos, ordinary remotes, outbox unchanged.
The CRDT document is not Git; hosting providers never see it. Two machines'
outboxes must not push the same ref to GitHub with different oids without a
convergence step (the merge-commit rule) or GitHub becomes the unexpected
LWW.

### 3.9 Changes to today's design

Eliminates the workspace mutation lock, OCC, and the saga. Adds an op-log
schema, gossip (even if "gossip" is `gwz merge --remote <lane>` carrying ops),
and a documented same-ref merge rule. Family/lane metadata become CRDT maps
and cannot drift from the ops that created them.

### 3.10 Costs and risks

- Portability: op log + fsync only.
- Offline: the design centre.
- Operational burden: none, until someone wants a linearizable "workspace
  HEAD" across laptops — then this is the wrong alternative.
- Effort: **L**. Abort/close races and GitHub dual-push are the sharp edges,
  not the CRDT library.
- Risk: op-log compaction; unbounded history if every agent txn is kept
  forever (snapshot the merged map periodically, like any CRDT).

### 3.11 Prior art

Bayou (1995) for anti-entropy and application-level merge; Automerge /
Yjs for the document; Git itself for the same-ref rule (a merge commit).
Uncertain: any off-the-shelf CRDT that already encodes Git refs well — assume
a purpose-built map, not a JSON Automerge of `gwz.conf/`.

---

## 4. B3 — Atomic multiplex Git host

### 4.1 Name and shape

The store of record is a **workspace-aware Git server**. One protocol session
uploads a pack and a list of ref updates that may span many member
repositories and the root, and the server applies that list with a single
server-side commit point. Local directories are ordinary clones (caches).
`gwz commit` / `merge --close` become `gwz-receive-pack` (a `receive-pack`
variant with a multi-repository command list). Hosting providers stay Git
mirrors fed by an outbox *from the server*, or the server *is* the host.

Internally the server may use a reftable, SQLite, or Postgres to implement
that one commit point. That does not make B3 an A1/A2 variant: the
*architecture* is "the Git protocol session is the transaction; clients are
Git", not "an embedded ledger beside each clone".

### 4.2 Why it is not a variant of A1–A5

**Axis: who coordinates, and where the commit point lives.** A2 stores objects
as `bytea` and makes members database rows; B3 stores Git packs and keeps
Git object identity as the public API. A3 splits a general CAS from a general
DB; B3's CAS is a Git ODB and its atomicity is a Git session. A4 collapses to
one history; B3 keeps N DAGs and N remote URLs. A1/A5 are local.

This is the "extend Git's protocol" direction.

### 4.3 Structure

| | |
| --- | --- |
| Commit point | Server-side atomic apply of the session's ref list after the pack is durable. |
| Bytes | Git object databases on the server (one shared ODB with namespaced refs is allowed). |
| Worktree | Ordinary local clones. Not in the transaction. |
| Concurrency | Server CAS / row locks per `(repo, ref)`. Disjoint sessions commit in parallel; group commit on the server log. |

### 4.4 Commit protocol for W1

1. Client builds commit objects locally (libgit2 / git2). They need not be
   fsynced; unreferenced locals are irrelevant if the session fails.
2. Open one connection. Send the command list: K member `old→new` plus root
   lock/marker. Send one pack with every new object (delta against wants).
3. Server: verify old oids (CAS), fsync pack, **atomic ref apply**, ACK.
4. Client updates remote-tracking refs; optionally fast-forwards local
   branches. Local projection is not required for durability.

Network: **one session**, one RTT plus bulk. Not O(K) connections.

### 4.5 W2 — conflict, continue, abort

Two workable shapes; pick one:

- **Server-side merge.** Session command `merge <plan>`. Server runs merge-ort
  next to the objects, returns a conflict pack / want-list for the three
  members, and holds an `open-merge` row. Continue uploads resolution trees.
  Abort deletes the row. Branch refs move only on close — one session.
- **Client-side merge, server-side close.** Conflicts are ordinary local Git
  conflict state. The open merge is only durable when the client chooses to
  push a `merge-state` ref in a session. Abort that never pushed is `rm` local
  state. This is weaker (a crash loses the plan unless the client pushed
  `merge-state` first). Prefer server-side merge for the no-runbook goal.

### 4.6 Cost

W1 barriers on the client: **zero** required. Server: **two** (pack, ref
transaction), flat in K.

| K | Commit-point *(est.)* | End-to-end *(est.)* |
| --- | --- | --- |
| 5 | 50 ms WAN + server flush | Dominated by RTT, not K |
| 50 | same | Pack still small; CPU verify O(K) on server, ms |
| 500 | same | Pack still ~MB; one connection. Server CAS of 500 refs is a single DB/reftable txn |

W3, 20 agents, one machine: if they all talk to a local server over loopback,
this is A1-quality latency with a process boundary *(est. 1–5 ms + execute)*.
W3, five machines: this is the design centre. Disjoint write sets proceed in
parallel on the server. Overlap: CAS reject, client retries the session
(objects already on the server, so retry is refs-only). Throughput is the
server's group commit *(est. 10²–10³/s)* and is independent of how many
laptops exist.

### 4.7 Failure (W4) and external writers (W5)

W4: sessions are atomic. Crash before ACK → client retries; apply is
idempotent if `new` oids and CAS `old` still match, or the server reports
"already at new". Crash after server commit, ACK lost → retry sees
already-applied. Local clones may be behind; `fetch` repairs. No human.

W5: a raw `git commit` in a clone is local until pushed. The workspace of
record is unchanged. A concurrent W2 on the server is unaffected. A later push
of that commit CAS-fails if the server's ref moved — ordinary Git. This is the
cleanest W5 story of the five: **raw Git is a cache writer, as with GitHub
today.**

### 4.8 Git compatibility

The product promise flips from "members are ordinary Git repositories of
record on disk" to "members are ordinary Git *remotes*; local folders are
clones". Raw `git` works in the clone. Interoperability with GitHub: either
the multiplex server pushes per-member to GitHub (outbox, members-then-root,
same ordering as today) or GitHub never sees a multi-repo transaction — which
is already true. A GitHub-side implementation of `gwz-receive-pack` should be
treated as politically implausible; self-host or a smart mirror.

`git clone https://gwz-host/ws/member.git` is a first-class path.

### 4.9 Changes to today's design

Eliminates local saga, local crash-recovery capability matrix, family index as
a durability problem, and "root last" as a *local* ordering constraint (it
becomes a server invariant inside one apply). Adds a Git server, authn/authz,
and an offline outbox of unpushed sessions. Lanes are server-side ref
namespaces (`refs/gwz/lanes/<name>/…`) plus cheap worktree clones, not reflink
copies of whole trees.

### 4.10 Costs and risks

- Portability of *clients*: any OS with Git. Server: Linux first; Windows as
  client-only is fine.
- Offline: local commits exist but are not workspace-durable until a session
  succeeds. Mitigate with a local A1/B4 outbox (a nested store). That nested
  store is a concession, not a variant of A1 as the architecture.
- Operational burden: **medium–high** (a Git host).
- Effort: **L**. Protocol is small; production hosting is the XL part if GWZ
  becomes the host.

### 4.11 Prior art

Git `receive-pack` and `proc-receive` (one repository); Gitaly's
transactional ref updates *per* repository (GitLab) — **not** cross-repo
atomic, cited only as the per-repo building block; `git push --atomic` is
single-repository. Google's Gerrit atomic topic submit is **not** claimed here
as a crash-atomic multi-repo disk transaction; treat it as a UI for sequential
pushes. The multiplex session itself is a GWZ protocol extension, not an
existing Git feature.

---

## 5. B4 — Single-key generation object (no metadata database)

### 5.1 Name and shape

There is no SQL, no daemon, no Git-of-gitlinks. Immutable objects live in a
content-addressed store (local directory of files, or S3). The entire
authoritative metadata of the workspace — every ref in every member and the
root, composition, current operation, stash ids, lanes, outbox — is **one
CBOR document** addressed by a single pointer `HEAD`. A transaction writes new
objects, then installs a new document with a compare-and-swap on that pointer
(`renameat` locally; `If-Match` / `If-None-Match` on S3). That CAS is the only
commit point. To keep the document small under retry, `HEAD` actually points
at an **immutable delta chain** (`gen N` includes the previous gen's oid plus
a diff of refs); compaction writes a snapshot generation.

### 5.2 Why it is not a variant of A1–A5

**Axis: which layer provides atomicity.** A1/A2/A3 use a database transaction.
A4 uses a Git ref in a monorepo of *source trees*. A5 uses a WAL of in-memory
state. B4 uses **single-object atomicity**, the one primitive every filesystem
rename and every object store actually has. Putting metadata in S3 as "just
another blob store" is not A3: A3's claim is that metadata *needs* a
transactional DB. B4 denies that.

### 5.3 Structure

| | |
| --- | --- |
| Commit point | CAS of the `HEAD` pointer to a new generation oid. |
| Bytes | CAS objects `obj/{oid}`; Git ODBs are caches filled from it, or members use `alternates` into a local CAS. |
| Worktree | Ordinary projections. |
| Concurrency | One-writer-at-a-time on `HEAD`. Disjoint write sets still retry the CAS; they do not run in parallel. Readers are lock-free (read `HEAD`, then immutable gens). |

### 5.4 Commit protocol for W1

1. Read `HEAD` → generation G (snapshot for planning).
2. Write new Git objects to the CAS, parallel, then **one batched barrier**
   (local) or parallel PUTs (S3; each PUT durable before return — *estimate*
   the client still waits on the slowest).
3. Write generation G' = `{parent: G, writes: [(repo, ref, oid), …]}` as an
   immutable object; barrier/PUT.
4. CAS `HEAD` from G to G'. On failure, reload, rebase the write set onto the
   new G if disjoint, else abort to the user (true overlap).
5. Project refs in clones/members, unsynced.

### 5.5 W2 — conflict, continue, abort

`MergeOpen` is a generation that adds `op: {kind: merge, plan, pre, conflicts}`
without changing `heads`. Continue/close/abort are further generations.
Abort's generation sets `op: null` and leaves heads at `pre`. Because branch
refs never moved, abort cannot need preservation stashes. A crash leaves
`HEAD` at a complete generation; projection catches up.

### 5.6 Cost

W1 barriers local: **three** in the naive form (objects, generation object,
`HEAD` rename). Collapse to **two** by putting G' in the same batched fsync as
objects, then renaming `HEAD`. S3: **one RTT for HEAD CAS** after object PUTs;
object PUTs dominate when they are many, not when they are tiny W1 commits.

| K | Local commit-point *(est.)* | S3 commit-point *(est.)* |
| --- | --- | --- |
| 5 / 50 / 500 | 5–10 ms (flat in K; delta is O(K) bytes) | 50 ms + object PUTs; still one CAS |

W3, 20 agents, one machine: all contend on `HEAD`. Successful throughput is
**one durable CAS per barrier interval** *(est. ~100–200/s locally, ~20–50/s
on S3)*. That is enough for 20 agents doing interactive commits; it is the
worst concurrent design in this document when txn rate climbs or when retries
collide. Disjoint sets do not help. Five machines on S3: same hot key, WAN
CAS, retry storms under overlap.

This is the dual of A2's row-level locks: simplest atomicity, coarsest
concurrency.

### 5.7 Failure (W4) and external writers (W5)

W4: `HEAD` rename is atomic; a crash before CAS leaves unreachable CAS
objects for GC with a grace period. A crash after CAS before projection:
re-project. S3 `If-Match` is atomic by the store's contract. No human. GC must
not delete an oid reachable from any generation newer than the grace window
(same lease problem as A3, smaller because the root set is "walk from HEAD
back N generations plus object refs in those docs").

W5: same import-or-diverge fingerprint as A1. Raw Git is outside the CAS.

### 5.8 Git compatibility

Members can stay ordinary Git repos whose refs are projections of the current
generation. Raw `git` works with lag. Hosting via outbox. Alternatively,
skip on-disk member `.git` entirely and materialise worktrees from the CAS
(lower compatibility, faster). The architecture does not require that choice.

### 5.9 Changes to today's design

Eliminates the saga, SQL, merge YAML, and the capability matrix. Adds a CAS,
a generation schema, compaction, and GC. The lock file in `gwz.conf/` becomes
a projection of the generation, committed in the root only when projecting for
GitHub, not because it is authoritative.

### 5.10 Costs and risks

- Portability: local rename works on all four platforms. S3 is optional.
  Raspberry Pi is a local-HEAD deployment.
- Offline: local `HEAD` is offline; sync is a CAS when reconnecting (may
  retry-rebase).
- Operational burden: none local; a bucket if remote.
- Effort: **S–M**. Smallest implementation of a real commit point. The
  concurrency ceiling is the reason not to stop here if 20 overlapping agents
  are the load.

### 5.11 Prior art

S3 conditional writes (`If-Match`); POSIX `rename` as the atomic publish
used by Git refs themselves, lifted to workspace scale; DynamoDB is *not*
used (that would become A3). Uncertain: S3's documented durability of
conditional PUT under concurrent writers — treat as "use the store's
conditional API as specified, plus idempotent retries".

---

## 6. B5 — Git workspace repository-format extension

### 6.1 Name and shape

Change Git, not the database beside it. A new repository format
(`extensions.gwzWorkspace=1`, or a successor of `core.repositoryFormatVersion`)
defines a **workspace store**: one object database, one **reftable** whose
names are `refs/repositories/{id}/heads/main` (and tags, `HEAD`, `MERGE_*`),
and logical members as first-class views. `member/.git` is a gitfile that
binds worktree + index + `HEAD` to one namespace in that store. `git commit`
inside a member takes the reftable writer lock and is therefore in the same
atomicity domain as `gwz commit` across members. A coordinated transaction is
`update-ref --stdin` (or the reftable API) listing every participating
namespaced ref plus the root's lock/marker: one Git ref transaction, O(1)
barriers in K, **without** collapsing histories into a monorepo tree.

Until upstream Git accepts the extension, GWZ ships a `git` shim / patched
git2 that understands it; an unpatched Git *refuses* the store (safe), rather
than seeing the wrong refs.

### 6.2 Why it is not a variant of A1–A5

**Axis: which layer provides atomicity, and what a worktree's `.git` is.**
A1's listed variant is a *side* ledger repo of gitlink *views*. B5 is the
members' actual ref storage, in a format raw Git is taught to read. A4 is one
commit DAG of concatenated source trees and path-filtered publication; B5
keeps N DAGs, N signatures, N remotes, N object identities. Rejected "custom
libgit2 backend as record" made refs invisible to raw Git; B5's point is that
they are visible. Rejected "reftable inside each member" is atomic per
repository only; B5 is one table for the workspace.

This is the "extend Git's formats" direction.

### 6.3 Structure

| | |
| --- | --- |
| Commit point | One reftable transaction (Git ≥ 2.45 reftable is crash-atomic for the refs in that table). |
| Bytes | One shared Git ODB (pack + MIDX). Members do not own objects; they borrow them. `git repack` is a workspace operation. |
| Worktree | Ordinary files. Each member has its own index and `HEAD`. Not a VFS. |
| Concurrency | Reftable writer lock serialises ref-mutating Git in the workspace (including raw `git`). Readers proceed. Disjoint members do **not** mutate refs in parallel. |

### 6.4 Commit protocol for W1

1. Read refs from the reftable (one process, no K `git_repository_open`
   cold starts if the store stays open — a daemon is optional, not required).
2. Write trees/commits into the shared ODB; **one batched object barrier**.
3. One `update-ref` transaction: K member branch tips, root lock, marker,
   reflogs. **One ref barrier.** Commit point.
4. Update member indexes if the tool wants `HEAD` and the index aligned;
   unsynced.

`git commit` in one member is the same path with K=1. `gwz commit` is K>1 in
one transaction. There is no projection lag *for refs*: raw Git and GWZ read
the same table.

### 6.5 W2 — conflict, continue, abort

Store merge state as namespaced refs (`refs/repositories/{id}/gwz/merge/…`)
in the same table, plus `MERGE_HEAD` per member view. Prefer not moving
`heads/*` until close (one transaction). Abort is one transaction that deletes
the merge refs and leaves heads untouched; worktrees are checked out from the
still-current heads. Continue is ordinary per-member index writes (not
atomic across members — indexes are still Git's weak spot) followed by a
close transaction for commits+heads.

Index/worktree remain non-transactional, so CRLF/filter surprises during
*conflict resolution editing* still exist. They cease to exist as
*recovery-of-interrupted-ref-moves*, which is today's runbook. That is an
honest remaining gap versus A5.

### 6.6 Cost

W1 barriers: **two**, flat in K.

| K | Commit-point *(est.)* | End-to-end |
| --- | --- | --- |
| 5 | 5–10 ms | Shared ODB writes; no K packfiles |
| 50 | same | Reftable append is designed for large ref counts |
| 500 | same | Same commit point; worktree updates if checkout needed still O(changed paths) |

W3, 20 agents, one machine: they share one reftable lock. Throughput is
**serialised ref transactions** *(est. 10²/s)*, similar to B4's hot key but
with Git-native readers and no retry of a fat document. Overlapping and
disjoint look the same at the lock. This is worse concurrent *ref* throughput
than A1 WAL (which at least pipelines readers) and much worse than A2/B3
row locks. It is still orders of magnitude above today's whole-call lock.

W3, five machines: a shared on-disk store on a network filesystem is unsafe
(Git's own rule). Multi-host requires B3 in front of this format, or one
machine owns the store.

### 6.7 Failure (W4) and external writers (W5)

W4: reftable recovery is Git's. Incomplete object writes are unreferenced.
Worktree dirt after a crash is ordinary Git status, not a GWZ parked record.
No human for refs/composition. A half-written index in one member is Git's
usual `index.lock` story — not a cross-repo saga.

W5: **this is the only alternative in B1–B5 that puts raw `git` on the same
commit point as GWZ for refs**, without a VFS. Two writers serialise on the
reftable lock. A raw `git commit` during an open GWZ merge either runs inside
the same store (and should be imported as that member's resolution) or blocks
briefly and then is visible. Define: if `gwz/merge/*` refs exist for that
member, a raw commit that concludes the conflict is adopted; a raw commit that
moves `heads/*` before close is either forbidden by a pre-commit hook GWZ
installs or becomes the close for that member. Hooks are not atomic; prefer
teaching the shim to refuse `update-ref` of `heads/*` while a merge ref
exists, except through `gwz merge --continue`.

### 6.8 Git compatibility

The long-term compatibility story is the best of the ten: members *are*
ordinary Git once the format is ordinary Git. Hosting: `git push` from a
member view uses a refspec that strips `refs/repositories/{id}/` onto
`refs/heads/` of that member's configured URL. Signatures survive (commits are
not rewritten). Private members are a separate ODB or a separate store;
do not mix private objects into a shared ODB (same constraint as A3
alternates / A4's private-member problem, solved by not sharing that ODB).

Short-term: unpatched Git, IDEs, Cargo git deps must go through the shim or
see `unknown extension`. That is a product cost until upstream.

### 6.9 Changes to today's design

Eliminates N object databases, N ref stores, the saga, lock-vs-HEAD drift, and
stash-metadata vs stash-payload split (stash is refs+objects in the same
transaction). Adds a format spec, a shim, migration of existing member `.git`
directories into the store, and a `repack`/`gc` that understands namespaces.
Lanes are additional namespaces plus worktrees, O(1) refs, no tree copy.

### 6.10 Costs and risks

- Portability: wherever Git runs, including Pi; no FUSE. Windows Git
  distribution lag is a real risk.
- Offline: full.
- Operational burden: none once the format is local Git; high *political*
  burden to upstream.
- Effort: **XL** to land in Git/git2/gix/Git-for-Windows; **L** for a private
  extension that only GWZ and a shipped `git` binary understand.
- Risk: IDEs calling `/usr/bin/git` and erroring on the extension.

### 6.11 Prior art

Git reftable (JGit, Git 2.45 `extensions.refStorage=reftable`); git
namespaces (`GIT_NAMESPACE`); `commondir` / gitfile / multiple worktrees —
none of which give N independent `main` branches with N remotes in one
crash-atomic table today. The workspace format itself is a proposed extension,
not an existing Git feature. Do not claim libgit2 already implements it.

---

## 7. Comparison

Barriers are for one W1 touching K repositories, excluding worktree file
bytes. "Members ordinary Git on disk" means *of-record checkouts*, not
"Git protocol exists somewhere".

| | Commit point | Barriers / W1 | Critical section | Members ordinary Git on disk | Multi-host txns | Ops burden | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Today *(inferred)* | none; saga | O(K), ordered | whole call | yes | no | none | — |
| A1 ledger | SQLite WAL | 2 | commit only | yes (projected refs) | no | none | M |
| A2 Postgres | PG txn | 1, group-shared | row locks | no (endpoints) | yes | high | XL |
| A3 split plane | DB txn | 2 local / 1 DB + PUTs | commit only | yes (alternates) | yes | medium–high | L |
| A4 record repo | one Git ref | 1–2 | one ref / one index | no (workspace is the repo) | yes, `push --atomic` | low | L + product |
| A5 daemon+VFS | WAL append | 1, group-shared | in-memory | mostly virtual | no | medium | XL |
| **B1 sequencer** | log append *before* execute | 2 (stage, log) | sequencer + flush | yes (projected) | yes, if Raft | none / cluster | L |
| **B2 CRDT refmap** | local op fsync | 2 | actor log (or none) | yes (projected) | causal only, not linearizable | none | L |
| **B3 multiplex Git** | server apply of one session | 0 client; 2 server | server CAS | clones yes, authority no | yes | medium–high | L (+XL if becoming a host) |
| **B4 generation object** | CAS of `HEAD` | 2 local / 1 S3 CAS | the `HEAD` key | yes (projected) | yes, hot key | none / bucket | S–M |
| **B5 workspace Git format** | one reftable txn | 2 | reftable writer | yes (same table as raw git) | no (needs B3 in front) | none local | L shim / XL upstream |

---

## 8. Speed ranking (all ten)

Ties are closer than the measurement noise these estimates deserve.

**Commit-point latency** (durable success, not checkout):

1. A5, B1 in-process — group commit, no SQL parser, sub-ms CPU + 1 barrier
2. A1, A3-local, A4, B4-local, B5 — 1–2 barriers
3. B2 — 1 barrier plus merge of the op log on a cold read
4. A2, A3-remote, B3, B1-Raft, B4-S3 — plus RTT
5. Today — O(K)

**End-to-end W1 latency** (capture, objects, commit point, ref visibility;
K → 500):

1. A5 — warm caches, no K opens, VFS checkout is lazy
2. A4 — one index, O(changed paths)
3. B5 — one ODB, one table, still K indexes if you touch K worktrees
4. A1, A3-local, B1, B4-local — parallel objects + projection
5. B2 — same plus CRDT apply
6. B3, A2, A3-remote, B4-S3 — network
7. Today — sequential saga

**Concurrent throughput** (W3):

1. A2 / A3 across hosts — row locks, group commit, many machines
2. A5, B1 in-process — group commit, parallel execute of disjoint sets (B1)
3. B3 — same shape as A2 at the Git boundary
4. A1 OCC — disjoint proceeds; overlap retries
5. B2 — best *offline* multi-master; linearizable throughput is the wrong
   metric
6. B5, B4 — serialised writers (~10²/s local); fine for 20 interactive
   agents, poor as a server
7. A4 — one index lock
8. Today — one txn per whole call

---

## 9. Recommendation

**Do not replace A1 with any of B1–B5 as the next substrate.** Among the ten,
A1 is still the only design that simultaneously (i) gives O(1) barriers, (ii)
keeps members as ordinary on-disk Git, (iii) adds no operator, (iv) can be
adopted incrementally, (v) matches the actual deployment (laptops, raw `git`,
GitHub as a dumb mirror). That is a product-fit ranking, not a speed ranking.

Among **B1–B5**, treat them as successor paths, not peers of each other:

1. **B4 as the on-disk *shape* of a first commit point** if SQLite is
   politically heavier than a file rename — then *stop* using B4's concurrency
   model. A 20-agent overlapping load will want A1's row-ish OCC or B1's
   sequencer on top of that generation chain.
2. **B5 as the compatibility endgame** if "raw `git` during a merge" (W5) is
   accepted as a first-class requirement that A1's import fingerprint will
   never make honest. Ship a private extension (effort L) and only then spend
   XL on upstream. B5 does not by itself solve multi-host.
3. **B3 when the load is W3-across-five-machines linearizable transactions.**
   That requirement is the fact that kills A1/B4/B5 as the authority. Do not
   build B3 because it is aesthetically "more Git".
4. **B1 when the pain is wasted merge CPU under OCC retry**, or when you want
   A5's group-commit without a VFS. Use it as the execution engine *in front
   of* A1 or B5's store, not as a second source of truth.
5. **B2 only if agents must commit offline on five machines and a
   linearizable workspace lock is explicitly *not* required.** Otherwise it
   buys a merge-commit surprise and a dual-push hazard at GitHub for no
   locally relevant gain.

**Facts that would change this**

| If this is true | Then choose |
| --- | --- |
| Members must remain Git-of-record on disk, single machine, no new daemons | Stay A1 (or B4 as a thinner A1) |
| Raw `git` must participate in the same ref transaction as GWZ | B5 (or A5, if worktrees matter more than Git-format politics) |
| Agents on multiple machines must see one lock, crash-atomically | B3 or A3; B1-Raft if you refuse SQL and Git hosting |
| Status/checkout at K=500 is the measured bottleneck after A1 | A5, not a different commit point |
| The ordinary-member-repo promise can die | A4 beats every B-series on simplicity |
| Offline multi-laptop without a leader is the product | B2, and drop linearizable `gwz log` across laptops |
| You will operate a Git host anyway | B3, and implement its commit point with A1 on the server |

What stays non-transactional in B1–B5 as in A1–A5: hosted per-repo `git push`
without B3 on that host; hooks, LFS upload, signing; Git config; worktree
bytes except under A5 (and, for *refs only*, B5).
