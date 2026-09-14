# GWZ transactional storage: consolidated proposals and default ranking

**Status:** first-stage consolidation and opinionated ranking; not an implementation decision or a protocol change. **Date:** 2026-09-14. No performance measurements or source-code size estimates were produced.

**Default recommendation: G1, a local transactional metadata database with a shared Git-format object store, using the existing GwzCore protocol.** G2, the ledger with objects left in each member, is the smaller initial change. G1 ranks first for the complete many-repository product because it also consolidates object ownership, lane sharing and durable storage independently of working-directory filesystems. This recommendation does not include a distributed database, object-storage service, virtual filesystem or replacement Git implementation.

## 1. Ranking rules and enumeration

Transactional correctness is an eligibility requirement, not something a fast or short implementation can compensate for. Among candidates capable of providing the required contract, the preference is approximately:

- **45% implementation and maintenance burden:** GWZ-owned code, state machines, platform branches, migration, testing, recovery, upgrades and operational glue.
- **40% speed:** end-to-end commit/merge/pull/stash, repeated status/log, and useful throughput across many repositories and agents.
- **15% product fit:** existing protocol semantics, ordinary member histories, platform coverage, offline operation and deployment burden.

These weights describe judgment, not calculated benchmark scores. Reusing a substantial proven database can require much less GWZ code than maintaining a short custom storage engine. “Small” means the complete supported solution, not its happy-path prototype. The workload assumption is primarily one machine with many repositories and lanes, with remote clients possible; five independent authoritative machines are not assumed mandatory.

The 15 source proposals reduce to **12 distinct implementation choices**. G1–G12 are listed in default preference order. G13–G15 are retired duplicate aliases, retained only to provide the requested G1–G15 cross-reference. They are not additional ranked alternatives. Subsequent ranking changes should change the rank column, not these identifiers.

The three sources are:

- [Original A1–A5](GwzTransactionalStoreAlternatives.md).
- [A6–A10](GwzTransactionalStoreA6.md).
- [G46 B1–B5](GwzTransactionalStore-G46.md).

## 2. Consolidated ranking

Effort is relative and includes production completeness. M/L/XL are bands, not measured lines of code. **Logical** atomicity means accepted refs, composition and recorded operations; it does not automatically cover arbitrary editor writes or independent remote hosts.

| Rank / ID | Consolidated proposal | Sources | Correctness boundary | Complete implementation burden | Speed assessment and default verdict |
| --- | --- | --- | --- | --- | --- |
| **1 · G1** | **Transactional metadata + shared Git-format object store; local first** | A3; B3 as an optional service/transport | One DB commit after referenced bytes are durable; managed checkouts are derived | **L**; one DB, shared-object retention/GC and checkout reconciliation | Best balance for many repos and lanes: indexed reads, parallel preparation, shared objects and short publication. **Preferred complete architecture.** |
| **2 · G2** | **Local ledger + existing per-member object databases** | A1 | One DB commit after every referenced member object is durable | **M–L**; least initial disruption, but per-member storage/retention and external-write integration remain | Excellent initial improvement; less consolidation of object I/O and lane duplication. **Preferred smaller first increment.** |
| **3 · G3** | **Immutable metadata generations + one conditional head update** | B4 | One locked/conditional publication of a complete metadata generation | **L** when recovery, indexes, compaction, retention and retries are included | Compact commit mechanism, but one global head and custom query/maintenance code. Keep as a serious comparator; not obviously less total code than SQLite. |
| **4 · G4** | **Database-native workspace server, objects included** | A2 | One database transaction for metadata and bytes; local clones are clients | **L–XL**; mature storage engine, substantial Git serving/client and operations work | Strong multi-client transactions and queries; byte transfer, blob management and deployment weigh against laptop-first use. |
| **5 · G5** | **One repository of record, filtered member views** | A4 | One Git history/ref publication; worktrees retain Git's ordinary limitations | **L–XL** including import/export and migration | Fast native combined operations, but independent histories, signatures, private members and reverse imports make the complete product expensive. |
| **6 · G6** | **Shared namespaced Git ref store with a workspace format extension** | B5 | One shared reftable transaction for logical member refs | **XL** across Git implementations and tooling | Efficient ref reads/publication with preserved member DAGs; requires a supported modified Git ecosystem. Stronger candidate than G9 if changing Git is acceptable. |
| **7 · G7** | **Deterministic pre-ordered transaction execution** | A6 + B1 | Durable replayable input order; successful outcome waits for execution | **XL** for deterministic execution, input retention and upgrades | Promising under costly contention or replicated execution; too much custom machinery for the default modest-code target. |
| **8 · G8** | **Transactional daemon with virtual worktrees** | A5 | Daemon WAL plus an explicitly enforced virtual-worktree boundary | **XL** with substantial platform work | Highest potential for warm status and lazy checkout; mount behavior, external applications and daemon lifecycle make it costly. |
| **9 · G9** | **Per-repository transactional ref descriptors** | A8 | One shared decision selecting prepared ref versions across repositories | **XL** for format, readers/writers, helping, durability and GC | Preserves distributed ref ownership but retains O(K) preparation work. Hard to justify over G6 or a database. |
| **10 · G10** | **Sealed immutable lane capsules** | A10 | Atomic artifact publication, not advancement of the existing live workspace | **L** | Useful, relatively bounded handoff feature; **not eligible as the sole replacement** for current commit/merge semantics. |
| **11 · G11** | **Causally replicated atomic change bundles / CRDT workspace** | A7 + B2 | Whole-bundle local acceptance and eventual convergence; no unique linearizable global head | **XL** for meaningful conflict semantics and retention | Fast offline proposal acceptance; **not eligible for a single accepted workspace contract** without an additional integration authority. |
| **12 · G12** | **Operating-system transactions over ordinary Git files/processes** | A9 | A new kernel/filesystem transaction spanning data and metadata | **XL; a separate systems product** | Conceptually broad atomicity, but no supported portable implementation established here. **Research only.** |

G10–G12 are below the eligible implementation choices because they change the requested contract or require unavailable infrastructure. Their placement is not a claim that G9 is faster than capsule creation or easier than a small artifact feature.

### Duplicate register: G13–G15

| Alias | Source | Canonical choice | Consolidation decision |
| --- | --- | --- | --- |
| **G13** | G46 B1, deterministic sequencer | **G7** | Same pre-order/execute/replay architecture as A6. Keep A6's stricter durability and acknowledgement distinctions. |
| **G14** | G46 B2, multi-master CRDT refmap | **G11** | Same causal/convergent family as A7. Keep whole-bundle conflict semantics; discard the assumption that arbitrary concurrent Git commits always merge automatically. |
| **G15** | G46 B3, multiplex Git host | **G1**, with a service-facing variant | A multiplex session does not implement atomic storage. Its proposed Git packs + transactional metadata are A3's storage shape. The transport could also front G2, G4 or G6; retain it as an adapter, not an independent substrate. |

Two near-overlaps remain deliberately distinct:

- **G3 versus G1/G2:** they share the authoritative-metadata/projection pattern, but G3 replaces a database with one generation pointer and custom indexing, compaction and conditional publication. That is a materially different implementation choice for the user's code-size criterion. If counting only broad architecture families, fold G3 into the ledger family; do not count both as independent inventions.
- **G6 versus G9:** both require changing Git, but G6 consolidates refs in one table while G9 retains separate ref stores and coordinates their visibility through descriptors. Their durability work, read algorithms and implementation burden differ substantially.

## 3. What survives from each proposal

### G1 — Local split storage

Use an embedded transactional database for accepted member/root refs, membership, composition versions, operation outcomes, merge drafts, stash metadata, lane retention and publication intents. Keep canonical Git object bytes in a service-owned shared Git-format store. Publish objects durably before the database references them; interrupted preparation leaves unreferenced data, not half a workspace commit.

The benefit over G2 is control over the entire authoritative storage location. A working directory on a difficult filesystem can be treated as input/output, while the database, canonical objects and acknowledged draft captures remain on qualified local storage. Keeping only the database local while indispensable objects remain on an unreliable mount does not achieve this.

The main extra code is reachability/retention, object import/export and checkout reconciliation. Use existing Git pack facilities; a generic multi-tier CAS, raw-blob reflink tier, distributed database and cloud backend are outside the recommended initial shape. Shared objects need a defined trust boundary; knowing an object hash must not grant access across private repositories. Multiple security domains may need separate stores.

### G2 — Ledger with existing object stores

Use the same accepted-state and draft model, but retain member ODBs. This minimizes migration and keeps native repositories familiar. It does not eliminate K object-durability operations, independent Git GC interactions or copying between isolated lanes. Accepted objects and draft/stash payloads must remain reachable even before native ref projection catches up. Unmanaged GC cannot be allowed to collect data the ledger still owns.

Choose G2 first when incremental delivery matters more than shared-object efficiency and member object stores already reside on qualified storage. The ledger is authoritative data: losing it can lose acknowledged operations, unresolved drafts, stashes and unprojected refs. Native refs are not a complete backup of it.

### G3 — Single generation head

Retain its useful idea: one immutable version describes the complete logical transaction and one conditional head publication selects it. Locally, serialize head validation and replacement under a real lock and implement file/directory durability. Plain rename is atomic replacement, **not compare-and-swap against an expected old value**. Two writers can otherwise both “succeed” and lose an update. [rename documentation](https://man7.org/linux/man-pages/man2/rename.2.html)

S3 offers conditional writes using an ETag precondition; retries and conflict responses still need handling. That is a valid remote primitive, not evidence that every local filesystem rename supplies the same API. [S3 conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html)

The attractive short implementation expands when adding fast status/log queries, operation lookup, snapshots, safe history truncation, concurrent readers, schema migration and storage-full recovery. Rank the complete engine against SQLite, not a pointer-swap demonstration against an entire database dependency.

### G4–G6 — Strong alternatives with larger product commitments

**G4** buys mature transaction/query machinery and central multi-machine authority. A self-hosted process does not necessarily require a public forge, but it still needs authentication where remote, durable object ingestion, Git interoperability and failure handling. Prefer it when centralized deployment is already accepted.

**G5** removes cross-history coordination by changing the product to one history. Preserve it as a serious option only if native member history identities and private-member separation can change. One Git repository does not make arbitrary worktree/index mutation crash-atomic, so “ordinary Git abort is sufficient” is not a complete answer to the stated reliability target.

**G6** retains independent histories while combining their authoritative ref storage. That is better suited to the product than filtered monorepo histories, but stock Git compatibility is not present until implementations actually support the proposed format. A shim cannot transparently fix IDEs and libraries that bypass it. Separate indexes and worktrees still need durable drafts and an ownership policy.

### G7–G9 — Specialized performance investments

**G7:** retain deterministic ordering as a possible future scheduling strategy. Logging object IDs does not persist unsynced objects; the input bytes must be durable or embedded. Log admission is not successful execution, and exact-base operations can still reject after ordering. Signing, filters, tool versions and merge inputs must be replayable. A warm service and bounded parallelism do not require adopting deterministic execution.

**G8:** virtual worktrees can reduce scanning and materialization, but virtualization alone does not group arbitrary application writes into one transaction or guarantee coherent multi-file reads. Supporting the necessary filesystem semantics across macOS, Linux and Windows is a major implementation, not an optional small adapter.

**G9:** retain the distinction between logical ref publication and physical worktree state. The descriptor scheme needs all Git readers/writers to participate and usually retains O(K) durable preparation. Its additional algorithm and ecosystem surface count heavily against it.

### G10–G12 — Useful boundaries, not default replacements

**G10** is a good bounded feature for delivering and preserving lane results. Materializing a new capsule at a new path is explicit; it does not secretly update an existing shell's working directory. It can complement G1/G2 without becoming their authoritative model.

**G11** must retain incompatible bundles as alternatives or require explicit integration. Sorting merge parents cannot resolve arbitrary text or semantic conflicts. An acknowledged close cannot simply lose to a concurrent abort and become garbage while retaining today's meaning of successful commit. A quorum read alone also does not turn a causal system into linearizable transaction processing.

**G12** would own a kernel/filesystem stack. That is incompatible with the present modest-code and portable-deployment preference, regardless of its theoretical elegance.

## 4. Rank the whole operation suite, not just a ref commit

The following execution model belongs behind G1/G2's existing service boundary. These are design requirements, not claims about implemented behavior.

| Operation | Transactional requirement | Main speed lever and remaining cost |
| --- | --- | --- |
| **Commit** | Capture selected staged inputs; persist objects; atomically publish all accepted member refs plus root composition/marker and outcome | Parallel hashing/object construction, cached repository handles and short validation/publication. K indexes and bytes still cost work. |
| **Merge** | Durable draft with frozen inputs; accepted lock stays at baseline; one closing transaction; abort closes the draft without resetting unrelated work | Compute clean members concurrently; retain their results while humans resolve conflicts. No lock held across the human interval. |
| **Pull** | Fetch immutable inputs first; freeze exact targets; validate and apply the chosen FF/merge/rebase plan to accepted state | Bounded parallel fetch and computation. Independent hosts still impose network and authentication costs. |
| **Push** | Durable per-operation publication intent and outcome; captured refs; root withheld until member dependencies are published | Parallel member pushes with ordered root publication. Outbox admission is not successful push, and independent hosts are not one atomic destination. |
| **Status** | Distinguish accepted composition from actually observed refs/index/worktree state | One metadata query, warm handles, reliable invalidation and parallel filesystem checks. The database alone cannot answer whether an arbitrary editor dirtied a file. |
| **Log** | Read a pinned accepted view; retain member provenance, coordination markers and declared ordering | Indexed operation/member history and commit-graph caches, with pagination through the existing stream. Building/importing those indexes has a cost. |
| **Stash push/list** | Capture staged, unstaged and selected untracked bytes durably with bundle identity before clearing anything | Unified metadata lookup and parallel capture; actual bytes still need reading. Native stash entries, if promised, need explicit compatibility handling. |
| **Stash apply/pop/drop** | Preserve current edits; allow ordinary conflicts; consume a stash only under the defined successful-application contract | Prepare in a draft/private view, retain payloads until completion; pop is not merely deletion of a metadata row. |
| **Branch/tag/snapshot/capture** | Publish the selected logical ref/composition change as one unit with exact member identities | Mostly indexed metadata and object work; external branch switching still changes files. |
| **Materialize/lane operations** | Separate durable accepted version from checkout readiness; keep required history and captured work reachable | Shared objects eliminate repeated object copies; actual checkout files still cost I/O. Lane disposal needs preservation proof, not just a name in a table. |

For K = 5/50/500, database metadata publication remains one transaction, but validation, hashing, object payload, result enumeration and checkout work do not become O(1). Object durability can have a fixed number of ordered phases while still issuing O(K) flushes. Measure both.

One root branch also gives otherwise disjoint operations a shared ordered publication step. SQLite itself permits only one writer at a time; its advantage is that preparation and reads can overlap while the write transaction is short. Do not describe it as concurrent row-level writers. [SQLite WAL concurrency](https://sqlite.org/wal.html)

Twenty agents cannot all touch disjoint sets of 50 repositories in a 500-repository workspace; there are at most ten such sets. At K = 500 every transaction overlaps. Count useful successful operations, not requests admitted to a queue or conflicting proposals retained for later integration.

## 5. Protocol preservation and the actual correctness boundary

The checked-in [Taut schema](../protocol/gwz.taut.py) already exposes `GwzCore` operations, request/operation identities, typed member results, events, `operation.result`, and the `log.output` stream. Its storage implementation can change without creating a second message protocol.

Preserve method identities, field tags, typed per-member outcomes, stream behavior, selection rules, dry-run semantics and explicit partial-operation policy wherever their meaning remains true. Preserve ordinary Git commit identities and the root lock/marker representation for Git interchange. Protocol compatibility is semantic, not merely emitting the same JSON field names.

Specific boundaries needing design review before implementation:

- **Success versus pending:** accepted input, committed logical state, checkout readiness and remote publication are different milestones. Existing successful operations must not silently become queue receipts. Use existing events/results where sufficient; any missing representation belongs in Taut through the normal schema process.
- **Status:** report actual checkout/lock differences honestly. A clean database view is not a clean working directory, and an ordinary filesystem scan is not a simultaneous snapshot of all externally writable files.
- **Stash:** the current schema explicitly describes native Git stashes and their lifecycle. Replacing them with database-owned snapshots may preserve user intent but is not automatically a drop-in contract change.
- **Merge:** retain the existing open-operation restrictions and lifecycle contract unless explicitly revised. More internal concurrency does not authorize changing which public operations are allowed during a merge.
- **Dry run:** no durable operation, object, checkout, fetch, stash or outbox mutation merely to prepare a plan. Work needing mutations must be simulated in memory or described as unknown under the existing contract.
- **Partial:** choose the explicitly permitted participant subset and report failures; do not accidentally recreate K independent commits under a command promising atomic selected-state publication.
- **Retry:** persist operation identity, request identity/content binding, decision and outcome. “Refs already equal the requested new values” is not sufficient proof that this particular operation committed, especially after intervening updates.

**No proposed metadata store can atomically control unrestricted editors or stock Git modifying arbitrary shared live directories.** The supported boundary needs one owner per mutable managed lane, captured durable inputs, private merge drafts, and preservation of foreign work. A watcher is an invalidation hint, not a lock. If foreign changes cannot safely be reconciled in place, preserve the directory and prepare a new explicit managed view; do not overwrite it during replay.

Thus the filesystem problem is narrowed rather than declared solved: canonical storage needs ordinary qualified durability; working-directory mutation is outside that atomic boundary. The requirement for persistent filesystem identity to infer old saga actions can disappear. Arbitrary mounts do not suddenly become safe database volumes. G1 is preferable when authoritative data must be independent of workspace placement.

Automatic recovery means reopening the database, recovering the last committed version, retaining incomplete drafts, and resuming safe publication/materialization. It does not mean inventing conflict resolutions, recovering unsaved editor buffers, or continuing after permanent storage loss. If all current paths must always be atomically updated for arbitrary raw readers, none of the modest-code choices meets that stronger requirement.

## 6. First-stage conclusion

Choose **G1 as the target**, constrained to a local embedded database and existing Git-format object machinery. Choose **G2 as a potential migration step**, not as a second concurrent source of truth. A full distributed A3 deployment is not part of this recommendation.

The implementation should reuse a small number of common responsibilities: durable metadata/outcomes, immutable object ingestion and retention, operation planning/finalization, safe checkout capture/materialization, and remote publication. Commit, merge, stash and lane management should use those same responsibilities rather than each acquiring another recovery journal. This is the main route to a modest codebase.

G1 and G2 are close. G2 becomes first if initial change size dominates, lanes are infrequent and all member ODBs already sit on reliable local storage. G4 or server-mode G1 rises if a centrally operated multi-machine service is already required. G5 rises if independent member histories cease to matter. G8 rises only if measured status/checkout cost dominates enough to fund platform filesystem work. G3 rises only if a complete prototype, including queries and recovery, demonstrates a real total-code advantage.

Do not choose a custom WAL, replicated sequencer, Git fork or virtual filesystem merely for an estimated barrier saving. The next decision should compare the bounded local G1 design with G2 and G3 on complete code ownership and representative workloads. This document consolidates and ranks the options; it does not authorize a schema migration or implement a storage engine.
