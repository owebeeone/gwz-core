# Transactional store alternatives A6–A10

## Comparison contract and cost model

**No candidate below simultaneously preserves unrestricted stock Git writes to shared live worktrees, provides atomic visibility to arbitrary readers, works natively on every listed filesystem, and guarantees automatic recovery.** Each proposal names the boundary it can enforce. A6 and A8 retain strong coordinated publication by restricting writers; A7 and A10 change the meaning of a successful transaction; A9 moves enforcement into an operating system that would have to be built and maintained.

These are architectural proposals, not descriptions of existing GWZ implementations. Prior-art links support the identified mechanisms, not the complete proposed systems. Costs are analytical estimates, not benchmark results. A1–A5 mean the architectures in the supplied prompt; A6–A10 are the five additional candidates.

Three completion times must remain separate:

- **Admission:** a request or proposal is durably recorded. This need not mean its requested branch changes succeeded.
- **Commit:** the promised state transition is durable and its outcome is known. An acknowledgement lost in a crash is resolved by operation ID, not by executing a second transaction.
- **End to end:** commit plus making the selected worktrees ready for the next command. Remote publication is a further completion condition when explicitly requested.

For estimates, assume warm repository metadata, bounded worker pools, one local durable volume, small staged changes, and no network hooks or interactive signing in the critical section. Root work is additional to K member repositories. A cold scan of 500 repositories is charged separately and is unnecessary for W1 if a maintained catalog identifies the selected K. Git object construction, hashing, compression, index reading, and the root lock's serialization remain real work.

Notation:

| Symbol | Meaning |
| --- | --- |
| D | New compressed Git object bytes across the K members, including new trees and commits; small text edits can still rewrite large blobs |
| I | Index bytes read or rewritten; not necessarily proportional to diff size |
| W | Worktree bytes newly materialized; approximately zero for an ordinary staged W1 commit |
| M | Metadata and operation envelope; O(K), plus potentially O(500) for today's full root lock |
| F | One correctly implemented durable flush phase, including the required device/cache guarantees |
| R | One network RTT; payload transfer time is additional |
| P | Useful parallel workers, bounded by CPU and storage; examples use P = 16 |
| H | Historical object bytes that must accompany a self-contained export |

**Barrier counts distinguish dependency depth from total flush requests.** K files flushed concurrently still entail K flush requests and shared-device contention. An expression such as `2 phases / O(K) calls` does not mean two cheap system calls. Directory entries, object reachability, WAL rotation, and checkpoints must also become durable. Group commit amortizes barriers across transactions; it does not make their wait disappear.

The prompt's claim that intent barriers cannot be batched is not an architectural lower bound. If K actions can be fully planned independently, all K intents can be recorded in one durable prelude before any of those mutations, followed by parallel execution and batched outcomes. Actions whose inputs depend on prior outcomes still require additional rounds, and ambiguity/recovery complexity remains. An improved saga is therefore a necessary performance baseline, although it is not counted as a new architecture here.

Illustrative sensitivity values, **not measurements or platform guarantees**: F = 0.2–10 ms, LAN R = 0.2–2 ms, WAN R = 20–100 ms, sequential write bandwidth = 0.1–1 GiB/s. SD cards, saturated drives, full durability on some laptop storage, and cold repositories can be substantially slower. With D = 20 KiB per changed repository, K = 5/50/500 produces about 0.1/1/10 MiB before index and metadata costs. Its sequential transfer floor is roughly 0.1–1/1–10/10–100 ms. No architecture removes that byte cost or O(K) input validation.

Root serialization is a common hidden bottleneck. If every coordinated commit appends to one ordinary root branch and creates a full 500-member lock, even otherwise disjoint W1 transactions share a publication order. Compute member results concurrently, then construct each root commit against its actual predecessor. Do not certify the whole old root as an unchanged read dependency when only selected member bases matter; genuine manifest dependencies must still be validated. A7 and A10 avoid this bottleneck by abandoning the requirement for a unique immediately current root branch.

W3 also has a combinatorial limitation: 20 mutually disjoint transactions of K = 50 or 500 cannot fit into 500 repositories. At K = 50 there are at most ten disjoint groups; at K = 500 there is one. The disjoint comparisons below mean 20 agents at K = 5, ten disjoint groups with queuing at K = 50, and total overlap at K = 500. Overlap means the same branch or a read/write dependency, not merely separate branches in the same repository.

For W4, the fault model is process death or power loss with storage honoring successful durable writes, eventual access to that storage, and correct software. Permanent media loss, lost encryption keys, and faulty storage cannot be solved by a transaction protocol. Unsaved editor buffers are not durable input. Normal source conflicts can require a person; **repairing uncertain transaction state must not**. Resource exhaustion can leave an operation automatically paused without authorizing destructive cleanup.

For the user-space proposals, “portable” means the design can use a tested durability implementation on APFS, ext4/XFS/btrfs, or NTFS/ReFS; it does not claim their flush and namespace APIs are interchangeable. Replay or immutable artifacts remove the need to infer creation from persistent file handles, so btrfs is not inherently excluded by that particular requirement. FUSE, overlay and network mounts still need an explicit storage contract and qualification. A9 instead needs its own supported transactional filesystem and cannot inherit this portability statement.

Ordinary remote hosting is outside every local atomic boundary. Git's atomic push concerns a supported receiving transport, not a distributed transaction over independent hosts. Member publication therefore remains retryable and the root stays withheld until dependencies are available. Permanent rejection becomes a typed publication failure, not rollback of the local commit. [Git push documentation](https://git-scm.com/docs/git-push)

## A6. Pre-ordered deterministic execution from a durable command stream

### 1. Name and shape

A sequencer durably orders **fully specified transaction inputs before execution**. Executors consume that order, run deterministic Git transformations, and expose only completed transaction boundaries. The stream, together with retained inputs and checkpoints, is sufficient to reproduce the workspace. On five machines, replicate the ordered inputs through consensus; run independent repository computations concurrently, with dependency ordering for conflicting operations. No participant votes after execution to decide whether a cross-repository transaction commits.

### 2. Why this is new

The changed axes are **execution ordering and concurrency control**. A1–A5 normally execute speculative work and certify its resulting state at the commit point. Here ordering precedes execution; an input's outcome is a deterministic function of its position and preceding state. This is not “A2 with a different database.” Putting an ordinary result transaction behind a queue would be such a variant. The proposed gain requires deterministic scheduling, replayable inputs, and no post-execution distributed commit protocol.

### 3. Structure

- **Commit point:** a durable sequence position determines the eventual outcome; successful W1 acknowledgement additionally waits for deterministic execution and validation. An ordered request may deterministically fail its expected-base condition.
- **Bytes:** incoming canonical blob bytes and nondeterministic inputs are embedded in the durable input stream or flushed before its reference to them. Per-repository Git object databases are rebuildable output caches. Checkpoints retain all inputs or resulting objects needed after log truncation.
- **Worktrees:** private ordinary Git sandboxes pinned to completed sequence positions. Shared managed branch state has one writer service. Stock Git activity in a sandbox is an input proposal, not an authoritative branch mutation.
- **Concurrency:** a scheduler orders read/write dependencies; unrelated members execute in parallel. A short publication frontier hides partly executed transactions. One global frontier can cause head-of-line blocking; finer dependency frontiers require validated snapshot reads.

### 4. W1 protocol

1. Pin a completed workspace version and capture selected indexes and object inputs. Acquire cooperating index locks during capture. Arbitrary editors remain outside this boundary; GWZ commits the captured staged bytes.
2. Run hooks, filters and signing that cannot be replayed deterministically before submission. Capture their outputs, authorship, timestamps, configuration digest, object format, and tool/merge algorithm version. A failure here submits nothing.
3. Submit an operation ID, expected member bases, declared dependencies, member object inputs, and root update recipe. Small inputs can reside in one checksummed stream record; large external inputs must already be durable.
4. Batch the request into the local durable stream, or into a replicated log entry on a quorum. Return “admitted” if useful, but not “commit succeeded.”
5. Execute in dependency order. A stale expected base produces a durable-by-replay rejection result; GWZ must not silently reparent a user's staged commit. If validation succeeds, construct K member commits and then the root commit against its ordered predecessor. New object files need not be independently flushed before acknowledgement if the durable stream can recreate their exact bytes.
6. Expose the completed transition and return exact object IDs. Materialize sandboxes separately. A retry with the same operation ID returns the same outcome.

This breaks the immutable-objects-first principle for **derived** objects, while retaining it for unrecoverable inputs. It also replaces “plan, then briefly lock to commit” with “order, then execute under predetermined dependencies.”

The one-flush model assumes the synthesized root commit is unsigned, or can be reproduced exactly without a post-order external interaction. A nondeterministically signed root whose parent is chosen by the sequencer needs an additional durable signed-output record before successful acknowledgement. Alternatively pre-sign against an exact expected root and reject stale roots, sacrificing disjoint-transaction efficiency. Capturing member signatures before ordering does not solve this root-signature dependency.

### 5. W2: conflict, continue, abort

Opening the merge records frozen source commits, bases, tool versions, and a draft identity. The 47 clean results and the three conflict descriptions become draft data; accepted member branches and lock remain at baseline. No scheduler slot or branch lock waits for a human.

Resolution happens in a private sandbox. Each acknowledged save stores the actual bytes and index stages, rather than relying on a future clean/smudge round trip. Continue submits a new fully specified command referring to those bytes. If accepted bases advanced, execution returns a stale-draft result and creates a new merge draft automatically; a source conflict may still need resolution. Abort is an ordered draft-closure command. It never reverses accepted refs and keeps captured resolution data under a retention policy.

### 6. Cost and W3

| K | Local W1 durability | Ready-input commit time, excluding queueing | Bytes written |
| --- | --- | --- | --- |
| 5 | 1 stream flush; 2 phases if payload is external | F + one worker wave + root finalization | D + M to stream; about D again to output cache |
| 50 | Same phase count | F + roughly 4 worker waves + root finalization | Same formula, proportional to 50 |
| 500 | Same phase count | F + roughly 32 worker waves + root finalization | Same formula, proportional to 500 |

The stream flush writes input bytes, so a “one barrier” claim does not exclude D. Checkpoint and cache writes add amortized traffic, potentially more than the approximate 2D above. The global sequencer section is O(M) append/order assignment plus batch flush, not Git execution. Dependency ownership lasts through execution.

On one machine, 20 disjoint agents share batch flushes and worker capacity. Overlapping transactions avoid repeated speculative execution when their operations can execute on ordered state; exact-base W1 requests can still reject and require resubmission. They cannot all succeed against the same old parent. Large K jobs can delay smaller transactions through publication-frontier blocking.

On five machines with a stable leader: approximately one client request/reply plus one leader–quorum replication RTT on the critical path, and possibly a fetch phase for uncollocated inputs. Followers must persist the payload or a previously durable reference before acknowledging. This is O(1) network phases, O(D + M) traffic per replica, and no K-participant prepare round. Partitions without quorum cannot accept globally ordered commits. Offline work remains local drafts.

### 7. W4 and W5

Recovery loads a checkpoint, verifies the stream, discards an incomplete tail, and replays the complete prefix. A crashed executor does not require action classification against its half-written Git caches. Rebuilding must replace only service-owned caches, never a user's unrecorded files. Persisted tool versions and captured nondeterminism are mandatory; “rerun whatever Git is currently installed” is not recovery.

A human's raw commit while a merge is open advances their private sandbox branch. GWZ captures that commit as a new proposal. The draft either incorporates it on a subsequent continue or remains based on its original version; abort cannot erase it. If stock Git is allowed to bypass the service and mutate the authoritative caches, the guarantee fails. Fingerprint polling does not close that race.

### 8. Git compatibility

Sandboxes are ordinary repositories and stock Git works there. Existing Git object IDs and signatures survive import. The authoritative workspace's ordered state is not directly writable using stock Git. Standard hosting works through export and an outbox; a deterministic root commit cannot use an uncaptured interactive signer during replay.

### 9. Changes to today's design

Eliminates per-mutation intents, reverse rollback of service-owned state, and filesystem-identity inference for that state. Adds a sequencer, deterministic execution specification, retained replay inputs, versioned checkpoints, and explicit admission/completion statuses. Drafts, stash captures, lane retention, and export requests become ordered commands.

### 10. Costs and risks

Portable user-space core on macOS/Linux/Windows; smaller worker/cache budgets on Raspberry Pi. Single-host use is offline. Shared five-machine authority requires a quorum and careful upgrades. Operational burden is medium locally, high when replicated. **Effort: XL.** Main risks are hidden nondeterminism, log growth, rejection under stale proposals, and head-of-line blocking. This is most attractive when five-machine throughput outweighs implementation cost.

### 11. Prior art

Calvin orders and replicates transaction inputs before deterministic execution, reducing the need for distributed agreement at the end. It does not make arbitrary Git commands deterministic or provide this workspace architecture. [Calvin, SIGMOD 2012](https://dsf.berkeley.edu/cs286/papers/calvin-sigmod2012.pdf)

## A7. Peer-to-peer atomic change bundles with causal convergence

### 1. Name and shape

Every coordinated change becomes one immutable **bundle event** containing all member changes, causal dependencies, and its root record. Peers merge sets of complete events. Disjoint independent bundles compose; conflicting bundles remain explicit alternatives until a merge event resolves them. There is no global sequencer and no single compulsory “current workspace” during a partition. The transaction promise is atomic inclusion of a complete change bundle, not linearizable advancement of one shared branch.

### 2. Why this is new

The changed axes are **coordination and consistency semantics**. A1–A5 have a transactional authority choosing one accepted version. A7 makes concurrent acceptance fundamental and preserves competing versions. A replicated A1 ledger using leader consensus would not qualify. Nor does making each member ref an independent CRDT: that can split a coordinated change and is expressly excluded here.

### 3. Structure

- **Commit point:** durable local acceptance of a complete event and its required bytes. Remote acceptance occurs independently after full dependency validation.
- **Bytes:** per-peer append-only event/payload segments; Git objects can be embedded and later indexed into local member repositories. Repository identity scopes access even when object hashes coincide.
- **Worktrees:** ordinary private checkouts pinned to a chosen causally closed view. A view with unresolved competing bundles cannot be presented as one unqualified scalar lock.
- **Concurrency:** causal delivery, idempotent event IDs, and merge of event sets. Independent effects commute only if declared read/write dependencies allow it; disjoint writes alone do not prevent write skew.

The atomic unit in the replicated set is the **whole bundle**, never a bag of per-ref updates. If bundles X and Y overlap in one member, retain X and Y as alternative complete compositions, including their nonoverlapping changes. Do not select X's member A and Y's member B independently and call that a resolved transaction. A resolution event explicitly chooses or merges their combined effects.

### 4. W1 protocol

1. Capture staged input in a private checkout and identify its causal baseline. Freeze the read set, membership dependencies, and K proposed member commits.
2. Construct a bundle with immutable member object IDs, root record, operation ID, causal parents, and all missing payload bytes. Unchanged members are inherited from the chosen baseline.
3. Validate local dependencies and reserve storage. Append a framed event with payload and checksum; flush it before acknowledging local acceptance. Separate large payload files require an earlier payload durability phase.
4. Install the complete event in the local visible set. An event with missing objects or dependencies remains invisible staging data.
5. Gossip the event and missing dependencies. Each peer durably validates and accepts the complete unit before showing it. A transfer cut in half never exposes half its member map.
6. Compose with independent bundles or show explicit concurrent alternatives. Export a chosen resolved view only when it exists.

This breaks the single global commit-point principle. It buys offline writes and removes a global serialization bottleneck by giving up a unique immediately agreed workspace head.

### 5. W2: conflict, continue, abort

A merge draft records the two chosen views and all 50 member inputs. Store the three conflicts and 47 clean outputs without adding a resolved bundle to the public event set. Continue creates one resolution bundle depending on all inputs it resolves. New concurrent events stay visible as additional alternatives; there is no promise that resolving three text conflicts eliminates unrelated concurrent semantic conflicts.

Abort closes the private draft and retains captured edits. If a proposal was already published, cancellation is another event with defined ownership and causal semantics; it cannot erase work other peers have accepted. “Abort” after accepted publication is therefore a new revert/cancellation operation, not rollback.

### 6. Cost and W3

| K | Local W1 barriers | Local serialization | Communication / bytes |
| --- | --- | --- | --- |
| 5 | 1 integrated event flush, or 2 payload/event phases | One append plus O(5) metadata | 0 required RTT; D + M locally |
| 50 | Same | One append plus O(50) metadata | 0 required RTT; D + M locally |
| 500 | Same | One append plus O(500) metadata | 0 required RTT; D + M locally |

Git construction still takes roughly 1/4/32 worker waves at P = 16. Indexing objects into repositories may write D again. Publication is not free because the full event must arrive and be persisted.

Twenty disjoint agents can accept changes without cross-agent coordination. On one machine they contend for bandwidth and event append capacity; on five machines each can accept locally. A direct all-peer distribution has at least four additional copies of the new bytes for five complete replicas. A remote durability receipt needs at least one data/ack RTT; missing-dependency discovery can add another. A fixed number of peer receipts protects against loss but does not establish a unique current version.

Overlapping agents have high **proposal acceptance throughput**, not necessarily high **resolved integration throughput**. Twenty incompatible commits can yield twenty alternatives and substantial reconciliation work. If all agents read the full 500-member composition as a semantic dependency, even disjoint writes may require resolution. Global convergence time is unbounded during a partition.

### 7. W4 and W5

Recover complete local event frames and rebuild the visible causal set. Incomplete frames are discarded or retransmitted. Missing causal dependencies remain unavailable, never silently substituted. No rollback classification is needed. Removing old events requires a replica-retirement and causal-stability policy; without it, retain history rather than delete data another offline peer needs.

A raw Git commit is captured as a one-member bundle from its recorded baseline. During W2 it becomes either a causal extension or an explicit competing bundle. The system preserves it; it does not silently overwrite it with a conflict resolution. Stock Git changes in a private checkout are outside the durability contract until captured, and continuous external mutation may delay capture. Recovery can still restore the last acknowledged bundle automatically.

### 8. Git compatibility

Ordinary member repositories, raw commits, and existing commit identities survive. A raw reader sees its pinned local checkout, not a globally agreed workspace. Standard hosts can store chosen branches or proposal refs; publishing one canonical branch requires a chosen resolution policy. Choosing a leader to serialize every publish would reintroduce a centralized integration bottleneck, even though offline proposal work remains distributed.

### 9. Changes to today's design

Eliminates the whole-workspace mutex and global decision service. Adds causal event storage, dependency exchange, whole-bundle conflict presentation, replica retention, and a distinction between accepted proposals and resolved workspace views. Stashes and lanes become owned events/views; membership deletion must not globally garbage-collect another peer's retained history.

### 10. Costs and risks

Portable user-space implementation on all listed desktop platforms; excellent offline behavior. Replication, identity, selective private-member access, and retention create medium-to-high operational burden. **Effort: XL.** The main product risk is semantic: users may reasonably reject “your commit succeeded, but the workspace now has competing accepted versions.” CRDT convergence of metadata does not resolve source-code meaning.

### 11. Prior art

CRDT research establishes convergence through suitable state merges or commuting operations, including registers that preserve concurrent values. Applying this to indivisible multi-repository bundles, view selection, and Git conflicts is the proposed design work, not a claim that an existing CRDT library supplies workspace transactions. [Shapiro et al., Conflict-free Replicated Data Types](https://www.lip6.fr/Marc.Shapiro/papers/RR-7687.pdf)

## A8. Git-native transactional refs using shared decision descriptors

### 1. Name and shape

Extend Git's repository/ref format so each affected ref can contain an immutable candidate version associated with a **transaction descriptor**. A descriptor identifies the complete expected-old/new ref set across repositories. One durable decision makes all those candidate versions logically visible. Until that decision, readers resolve every candidate to its old value. Unrelated ref updates can proceed concurrently, and a surviving process can finish or retire an abandoned descriptor.

### 2. Why this is new

The changed axis is **Git's own reference semantics and atomicity layer**. Authoritative ref versions stay in their owning repositories; there is no A1 database of all current refs and no projection back to ordinary ref files. A shared directory contains decisions, not the workspace's authoritative branch map. This requires an actual Git format extension used by all readers and writers, not a libgit2-only backend hidden from raw Git.

It resembles multiword compare-and-swap. It is not distributed two-phase commit: all repositories are in one local ownership domain, with no independently failing participant managers or durable per-repository votes. Nevertheless it retains O(K) durable ref installation work. If implemented as K services that each prepare and vote, it becomes the already rejected two-phase-commit design.

### 3. Structure

- **Commit point:** the durable, validated descriptor decision, gated so readers cannot acknowledge new values before that decision is durable.
- **Bytes:** Git objects and ref-version records remain per repository; immutable descriptors and decisions live in a workspace-owned directory. No descriptor may commit until all referenced objects and ref-version entries are durable.
- **Worktrees:** ordinary private working directories. W1 reads captured indexes and changes logical refs; W2 uses a separate draft checkout. Checkout is not inside the ref transaction.
- **Concurrency:** sorted acquisition of affected ref locks, expected-version checks, and helping or safe abort of abandoned operations. Readers follow decisions; consistent cross-repository reads pin/validate a version vector and retry if it changes. No ABA: versions and descriptors have unique identities.

The format must cover deletions, symbolic refs, reflogs, root refs, and GC reachability—not just `refs/heads/main`. A multi-repository read cannot get a snapshot merely by reading K individually atomic refs. It must validate both ref versions and descriptor decisions. Under sustained mutation optimistic snapshot acquisition can starve.

### 4. W1 protocol

1. Capture staged inputs and compute objects outside the ref locks. Build the intended K-member update and identify the root update dependency.
2. Lock selected ref slots in a canonical order. Validate expected member versions and actual manifest dependencies. Acquire the root publication lock late and construct the root commit against its current predecessor.
3. Write a complete immutable descriptor and install candidate versions containing old/new values and descriptor identity. Before commitment, missing or incomplete descriptor data must resolve to old state or temporarily block, never expose new state.
4. Flush all new objects, descriptor data, ref-version files and required directory entries. These calls can overlap across repositories, but all must finish before the decision.
5. Write and flush the commit decision while holding the visibility gate. Only then allow transaction-aware readers to return new values and acknowledge W1. A helper seeing an undecided or not-yet-durable decision must perform the required verification/flush before exposing it.
6. Release ref locks. Compact old ref versions only after readers and retention policies release them; the commit does not depend on compaction.

An application-level version read plus fingerprint check cannot replace step 2. All writers, including raw Git, must implement this format's locking and decision rules.

### 5. W2: conflict, continue, abort

Build all merge results in a private draft; persist its conflict bytes, index stages, frozen parents, and session identity. Shared accepted refs and the root lock remain unchanged. The draft can itself be represented by retained Git objects and a private session ref, avoiding a second authoritative stash index.

Continue prepares one descriptor for the 50 member results and root. A changed member version fails validation before publication and preserves the draft for an automatic replan. Abort removes only the session's active status through a small transaction; keep resolution objects reachable. It never needs to reconstruct the user's pre-merge worktree through filters. A compatibility command that insists on merging in place would reintroduce that restoration problem.

### 6. Cost and W3

Let q be the implementation-dependent number of object, ref, and directory flush requests per changed repository after packaging. Do not assume q = 1. The root and descriptor add constant work with respect to K.

| K | Durability work | Ordered phases | Critical section |
| --- | --- | --- | --- |
| 5 | About 5q + O(1) flush requests | Payload/ref preparation, then decision | O(5) install/check plus flush completion and root publication |
| 50 | About 50q + O(1) | Same logical phases, more bounded-pool waves | O(50) plus flush completion |
| 500 | About 500q + O(1) | Same logical phases, substantial device work | O(500) plus flush completion |

Bytes are D + O(M) + ref/reflog records; W1 need not copy the worktree. No network RTT is required locally. A rough latency model is construction + O(K) ref work + T_flush(qK) + F_decision. It is **not** construction + 2F independent of K.

Twenty disjoint agents can prepare independently, but one root branch serializes closing publication, including its durability wait. Overlapping transactions wait or fail expected-version checks; there is no throughput miracle at K = 500. The strongest gain over today's saga is parallel preparation and removal of per-action recovery ambiguity, not constant flush count.

Five machines can submit requests to one machine owning the storage; that adds a client request/reply and payload upload. Independently writable repositories on five hosts do not participate atomically under this protocol. A network filesystem is not a substitute for a specified multi-host lock/durability protocol.

### 7. W4 and W5

On restart, a complete committed descriptor selects new values. A descriptor with no valid decision selects old values; candidate records can be retired automatically after exclusive ownership is established. A completed decision with missing referenced data is a violated durability invariant, not an invitation to guess. The old values remain available until safe reclamation.

Use kernel-released locks plus descriptor identities for process-failure handoff; do not infer safety from a reused PID or stale pathname. GC understands both prepared and committed reachability. Cross-volume operation needs flushing every relevant volume and stable access to all of them; the cost table assumes one volume.

An **upgraded** raw Git commit during W2 is an ordinary one-member ref transaction. It succeeds while the draft remains open, and causes later continue to revalidate. Abort leaves that commit intact. Unmodified Git must reject the repository's unknown required extension, rather than interpret candidate records as loose refs. Allowing old Git to write anyway invalidates the design.

### 8. Git compatibility

Git object identity, packs, commit signatures, and wire exports survive. **Stock Git on the authoritative repositories does not.** A suitably extended Git CLI would still feel like raw Git, but GUIs, libraries, maintenance tools, and alternative implementations need support or must refuse access. Conventional exported clones remain ordinary. Standard hosts need no extension for independent exports; atomic cross-host publication still requires an outbox.

### 9. Changes to today's design

Eliminates branch-ref rollback classification and central ref projections. Adds a Git format, descriptor resolution, crash-safe decision gating, snapshot reads, GC rules, and mandatory ecosystem coordination. It does not solve transactional editing of arbitrary worktree files. Drafts and captured work remain essential.

### 10. Costs and risks

Implementable in user space on macOS/Linux/Windows if durable replacement and locking are correctly specified for each platform. Fully offline on one host. Operational burden is high because every ref consumer must understand the format. **Effort: XL**, including upstream adoption or maintaining a Git fork. This is credible as a Git ecosystem project, weak as a speed-first GWZ-only investment.

### 11. Prior art

Multiword CAS uses descriptors and allows nonoverlapping updates to proceed concurrently; that is an algorithmic analogy, not a disk durability implementation. [Harris et al., A Practical Multi-Word Compare-and-Swap Operation](https://www.microsoft.com/en-us/research/publication/a-practical-multi-word-compare-and-swap-operation/)

Current Git documentation explicitly warns that readers may see only a subset of a multi-ref update. Reftable provides a consistent reference-space snapshot within one repository; it does not supply this cross-repository descriptor mechanism. [Git update-ref](https://git-scm.com/docs/git-update-ref.html), [Git reftable format](https://git-scm.com/docs/reftable)

## A9. Operating-system transactions over ordinary Git processes and files

### 1. Name and shape

Put the transaction boundary below Git, in a kernel providing transactions over filesystem data, metadata, and relevant process-visible resources. GWZ performs a short coordinated operation inside one OS transaction spanning all K repositories and its runtime state. The filesystem commits the complete write set through one journal decision, with isolation from other processes. The member repositories remain real Git repositories rather than projections of a separate logical store.

### 2. Why this is new

The changed axis is **the layer providing atomicity**. A5 supplies a Git-aware daemon and virtual worktrees; A9 requires a general OS transaction facility handling ordinary files, indexes, refs, and external process interference. This is not a filesystem snapshot, directory-generation swap, or shadow-paged workspace. Open descriptors remain kernel objects whose transactional behavior must be defined, rather than secretly pointing at an old generation.

This is a plausible research architecture, **not an available portable deployment option**. Treating ordinary ext4 journaling or a Windows file API as an already complete solution would be incorrect.

### 3. Structure

- **Commit point:** the filesystem's durable transaction commit record covering every modified byte and namespace entry.
- **Bytes:** ordinary member Git files and working files, plus journal payload and isolated uncommitted buffers. One supported transaction-capable volume.
- **Worktrees:** real files on that volume, observed through the transaction-aware kernel. No FUSE or Git projection service is required.
- **Concurrency:** kernel tracking of read/write sets, validation and strong isolation against nontransactional system calls. Conflicting operations wait, abort, or retry under a defined policy; unrelated file sets can proceed.

The required kernel must handle page-cache writes, `mmap`, directory operations, file descriptors, child processes, and crash recovery. A multi-call raw reader still needs a read transaction for a coherent snapshot across a commit, just as a database client does. This is much larger than adding an atomic multi-file rename.

### 4. W1 protocol

1. Outside the OS transaction, capture index inputs, perform expensive hashing/compression, and capture nondeterministic hook/filter/signature outputs. Preflight space and journal capacity.
2. Begin a short OS transaction, conceptually enclosing GWZ and the child operations it owns. This is a required capability, not the name of an existing portable API.
3. Revalidate input versions inside the transaction. Install Git objects, index changes if required, member refs/reflogs, and the root commit/lock/marker. Runtime outcome and export intent belong to this same write set.
4. The kernel validates conflicts and serializes with external accesses. A conflict returns a clean retry before visibility. Nontransactional external side effects are prohibited inside this phase.
5. Flush journal payload then the commit record under the filesystem's ordering contract. Return success only after durability. Home-location writes/checkpointing can follow.

Immutable objects may be preflushed outside the OS transaction to shorten it; they then become harmless unreachable objects on abort. Alternatively journal them with everything else, at the cost of more journal traffic.

### 5. W2: conflict, continue, abort

Do not keep a kernel transaction open while a human resolves conflicts. Create a durable private draft containing the frozen inputs, clean outputs, three conflict indexes and worktree bytes. Accepted refs and lock stay unchanged. Save resolution checkpoints using short OS transactions.

Continue executes a short transaction that validates accepted bases and applies the completed merge. If applying a checkout would overwrite intervening user edits, validation fails and those edits remain; automatically preserve the draft and replan. Abort transactionally closes the draft and retains captured work. Since accepted working files were not tentatively replaced, there is nothing to restore during the human interval.

### 6. Cost and W3

| K | Target durability cost | Critical section | Traffic |
| --- | --- | --- | --- |
| 5 | About 2 journal phases | Validate/install 5 members plus journal flush | Journal payload + home writes |
| 50 | About 2 journal phases if write set fits | Validate/install 50 members plus flush | Proportional to touched pages and metadata |
| 500 | About 2 phases only if capacity suffices | Potentially long validation and journal reservation | May exceed capacity; reject before application or reserve a larger transaction |

The two-phase target is a proposed full-data journal design, not a measured TxOS guarantee. Journal rotation, metadata logging and preflushed objects can add barriers. A rough traffic bound is about twice the journaled changed-page bytes, plus prewritten objects and filesystem overhead; page granularity and index rewrites can make that far larger than D. Splitting an oversized write set into separately visible commits would violate atomicity.

Local W1 requires no RTT. Twenty disjoint agents can execute concurrently until they encounter root-branch and journal bottlenecks. Shared directories, allocation structures, or coarse conflict tracking can create false conflicts. Overlapping operations retry or serialize; root work remains serial. Kernel transactions do not eliminate the K Git computations.

For five machines, the defensible mode is remote execution on one transaction-capable host, adding one client request/reply plus byte transfer. Atomicity over five independently owned kernels is unsupported. Distributed transactions added above them would bring back the rejected participant protocol.

### 7. W4 and W5

Recovery replays committed journal transactions and discards or undoes uncommitted ones according to the selected journal design. This includes GWZ outcomes and draft records, so no filesystem-handle heuristic is needed to decide what GWZ committed. The guarantee requires transactional **file data**, not merely a metadata-consistent filesystem.

A raw Git commit during the human phase of W2 operates on ordinary accepted files and survives draft abort. Continue notices its ref/index version changes inside the closing OS transaction. If raw Git overlaps that closing phase, kernel isolation protects the accesses, but a stock multi-syscall Git command is not automatically one whole OS transaction unless launched/enrolled that way. Specify enrollment and retry behavior; do not claim arbitrary applications acquire command-level atomicity by magic.

### 8. Git compatibility

Git on-disk formats, raw Git, existing tooling, object identity and ordinary hosting survive **inside the supported kernel environment**. Hooks that send messages or write to another device cannot be rolled back by this transaction. Network actions remain outside it. On macOS/Windows, a dedicated Linux VM is a possible product configuration; repositories then live inside its virtual disk and tools run inside the guest. A shared host folder does not acquire transactional semantics.

### 9. Changes to today's design

Eliminates the application-level ref/index/runtime saga for short local operations. Adds kernel ownership, a full-data transactional filesystem, syscall coverage, journal sizing, and transaction-aware process launching. Long interactive merges still need explicit durable drafts. Safe deletion of user-owned directories still requires ownership policy even if deletion itself is atomic.

### 10. Costs and risks

No native cross-platform solution is established by this proposal. Linux requires substantial kernel/filesystem engineering; a VM adds deployment and tooling friction on macOS/Windows; Raspberry Pi requires an ARM-capable implementation with tight memory limits. Offline use is good. Operational burden is very high. **Effort: XL, effectively a separate systems project.** Exclude it from a near-term production shortlist despite the attractive atomicity boundary.

### 11. Prior art

TxOS demonstrated system transactions and strong isolation in a modified Linux kernel; its published implementation is a Linux 2.6.22.6 proof of concept, not a modern supported kernel. [TxOS source and description](https://github.com/ut-osa/txos), [Operating Systems Should Provide Transactions](https://www.cs.utexas.edu/~witchel/pubs/porter09hotos.html)

Transactional NTFS is real, but Microsoft recommends investigating alternatives rather than creating a dependency on it. It is not a portable NTFS/ReFS solution for GWZ. [Microsoft's TxF guidance](https://learn.microsoft.com/en-us/windows/win32/fileio/deprecation-of-txf)

Ordinary ext4 journals metadata by default; that does not make K arbitrary Git operations one application transaction. [Linux ext4 journal documentation](https://www.kernel.org/doc./html/next/filesystems/ext4/journal.html)

## A10. Sealed lane capsules: transact on immutable deliverables

### 1. Name and shape

Change the transaction unit from “advance the live workspace” to **“seal and deliver this exact lane result.”** A capsule contains a complete composition description, K member commit payloads, the root commit, captured draft/stash state when requested, and explicit prerequisites. Publication of one complete capsule is atomic. Consumers select a capsule ID and materialize it into a new ordinary workspace. Existing workspaces continue to belong to their users and are never implicitly moved to the capsule's version.

### 2. Why this is new

The changed axes are **transaction unit and worktree ownership**. There is no A1–A3 mutable authority mapping all current refs, no A4 combined member history, and no A5 virtual working copy. There is also no CRDT convergence rule as in A7: independent capsules are deliverables, not competing updates to one logical current workspace. Integration is a separately requested operation producing another capsule.

This is not the rejected symlink-generation or filesystem-snapshot scheme. No path is switched beneath a process; an old working directory intentionally stays old. If a “current capsule” pointer is later introduced as authoritative workspace state and existing worktrees become automatic projections, this collapses back into A1. The distinction therefore requires a real product change, not new terminology for a view ledger.

### 3. Structure

- **Commit point:** durable publication of a complete, verified capsule under its immutable identity.
- **Bytes:** Git-format pack/bundle sections and captured non-Git bytes inline in a framed archive. An incremental capsule names retained base capsules; a portable self-contained capsule includes all necessary history.
- **Worktrees:** independent ordinary workspaces, explicitly materialized at new paths. Existing files and open handles remain untouched.
- **Concurrency:** independent producers seal independent artifacts. No global root branch, global ordering, or compare-and-swap on a “latest” workspace exists. A single recipient can serialize integration if desired, but that is additional work.

The capsule's root commit describes its exact composition and preserves each member's original Git history. It is not one monorepo commit replacing those histories. A capsule receipt certifies storage of the entire deliverable and prerequisites, not acceptance onto a host's main branches.

### 4. W1 protocol

1. Select an explicit baseline capsule or capture a new baseline. Freeze selected indexes and immutable Git inputs in the producer lane. Revalidate capture if another writer changes them.
2. Construct K member commits and the root commit in scratch storage. Do not advance the producer's live refs as part of capsule publication.
3. Stream a private capsule with repository identities, full composition, operation ID, exact commit IDs, pack sections, prerequisite IDs, and checksums. Verify the dependency closure before acknowledging; a merely named remote prerequisite is insufficient.
4. Flush the completed capsule, publish its immutable name, and durably publish the directory entry using the platform's supported protocol. Readers go through a visibility gate until completion, or verify and finish the required durability before returning success. A checksum alone is not a durability barrier.
5. Return the capsule ID and exact per-member outcomes. Receipt is idempotent by operation ID and content identity. Delivery to another machine is complete only after that machine durably verifies the capsule and its prerequisites.
6. If requested, create a new workspace from that capsule and report it ready after materialization. This is explicitly a separate completion stage; the existing workspace does not change in place.

This preserves all-or-none artifact publication but intentionally gives up all-or-none mutation of the current checkout. It is a good candidate only if lane handoff is an acceptable primary workflow.

### 5. W2: conflict, continue, abort

Merging two capsule IDs creates a private draft workspace with 47 clean results and three conflicts. Save draft checkpoints as capsules containing frozen inputs, index stages and exact captured bytes. Continue seals a new merged capsule. Abort closes the draft and retains its last acknowledged checkpoint; no accepted workspace was partially merged.

If an integration target advanced, the completed capsule still exists. Integrating it against the new target is another merge; report the distinction rather than quietly claiming the target was updated. This can increase user-visible integration work while keeping storage recovery simple.

### 6. Cost and W3

| K | Incremental capsule barriers | Bytes written to seal | Critical section |
| --- | --- | --- | --- |
| 5 | About 2 file/name durability phases | D + M, about 0.1 MiB of illustrative object payload | Immutable-name publication only |
| 50 | Same | D + M, about 1 MiB object payload | Same; hashing/packing stays outside |
| 500 | Same | D + M, about 10 MiB object payload | Same; full composition validation remains O(K) |

These counts assume already durable retained prerequisites and a supported file-publication protocol. First capture costs H + D + M; a self-contained capsule repeats H unless existing archive sections can be reused without copying. No O(1) lane-copy claim is justified. Materializing a fresh workspace adds object extraction, index construction, and potentially the full selected worktree byte size, not merely the diff. Retaining a cache can reduce reads but does not automatically remove file creation cost.

Twenty agents can seal independently, including overlapping repository sets, because they do not compete for one destination branch. Local bandwidth and packing dominate. On five machines there are zero required network RTTs for local sealing, and roughly one transfer/receipt RTT plus byte transfer for a known-prerequisite receiver; prerequisite negotiation can add a round. Replicating to four other machines copies the missing archive bytes four times.

This is high **deliverable throughput**. The throughput of integrating twenty overlapping capsules onto one accepted branch is unproven and can be dominated by merge conflicts. Do not compare it to A2 serializable branch commits as though it delivered the same result.

### 7. W4 and W5

Recovery verifies complete published capsules and their prerequisite closure. Incomplete private files are ignored; complete artifacts with a lost reply are found by operation ID. Retain or automatically remove only producer-owned temporary files. A materialization interrupted by power loss can be restarted at a fresh path from the capsule; the old workspace is still available. Partially materialized paths are not advertised as ready.

A raw Git commit while W2 is open belongs to the workspace where it was made. If it is in the input lane, it does not retroactively alter frozen inputs. If it is in the draft lane, continue captures that branch/index state as an explicit resolution input after validation. Abort preserves the lane or its captured checkpoint and cannot reset an unrelated commit. As elsewhere, arbitrary edits after the last acknowledged checkpoint are not promised power-loss durability.

### 8. Git compatibility

Ordinary repositories and stock Git survive completely in producer and consumer workspaces. A capsule wrapper is new; its per-repository Git sections can be imported using conventional tooling. Standard hosts receive ordinary member commits and root commits through asynchronous export. Global atomic advancement of their independent branches remains unsupported.

The product concession is substantial: `gwz commit` would return an immutable deliverable and possibly a new workspace, rather than atomically advance the live member branches. Expose that as a distinct operation unless the product deliberately changes its contract.

### 9. Changes to today's design

Eliminates live-workspace rollback, external-writer reconciliation during publication, and filesystem-identity proofs for newly produced artifacts. Adds capsule framing, verification, prerequisite retention, explicit materialization, draft checkpoints, and durable recipient receipts. Lane disposal requires a receipt proving all required history and captured work exist elsewhere; sending bytes or possessing an unverified capsule name is not enough.

### 10. Costs and risks

User-space implementation is portable across macOS/Linux/Windows, subject to validated durable-file-publication primitives. Good offline and Raspberry Pi operation with streaming and bounded packing memory, although storage capacity may dominate. Operational burden is low locally, medium for artifact distribution and retention. **Effort: L.** Largest risks are duplicate history storage, materialization latency, prerequisite-chain management, and a workflow users may find too different.

### 11. Prior art

Git bundles transfer objects and refs offline, may be full or incremental, and can carry prerequisite commits. They do not capture the index, working tree or repository configuration, and one native bundle is not a multi-repository workspace transaction. The capsule adds those structures and its own atomic publication/receipt contract. [Git bundle documentation](https://git-scm.com/docs/git-bundle)

## Candidates discarded during selection

- **LMDB/RocksDB/another embedded store for authoritative refs:** A1 with a substituted storage product.
- **A distributed SQL service or FoundationDB replacing PostgreSQL:** A2/A3 with a substituted transaction engine.
- **Shared packs plus a consensus metadata service:** A3; consensus does not change the split-plane architecture.
- **A reftable/gitlink repository containing current workspace views:** explicitly the excluded A1 variant.
- **A monorepo with clever per-member export:** A4 regardless of the export proxy's name.
- **A Git daemon with a different virtual filesystem:** A5 regardless of its kernel bridge or remote transport.
- **A GitHub/Gerrit-style queue that merely serializes root-last pushes:** still the existing saga, or A1/A2 if the queue's ledger becomes authoritative; no cross-host atomicity is created.
- **K independent reftable prepare/commit participants:** the rejected two-phase protocol with K durable preparations.
- **ZFS/btrfs/APFS snapshots, symlink-flipped workspaces, or forked VM disk generations:** the rejected generation-switch boundary; existing handles do not change identity.
- **NVMe atomic-write commands for the whole operation:** a bounded block-write facility does not atomically cover arbitrary K-repository namespace, cache, index and worktree updates.
- **Persistent-memory metadata in place of a database:** merely changes A1/A3's storage medium unless the application consistency model changes too; hardware scope is poor.
- **Hardware transactional memory around Git:** CPU transaction mechanisms do not persist arbitrary filesystem effects and cannot cover long Git operations.
- **Independent per-member CRDT refs with last-writer-wins:** can discard part of a coordinated change and hide conflicts; A7 retains whole bundles instead.
- **Transactional NTFS as a ready-made cross-platform substrate:** API scope and platform dependence fail the requirement; A9 is explicitly a new OS effort, not a claim that TxF solves it today.

## Comparison with A1–A5

Barrier counts below include new authoritative input/object durability, not just the final metadata commit. `O(K) calls` means parallelization may reduce dependency depth but not total durability work. Async projections, checkpoints and remote mirrors add work outside these counts. Effort is relative to adopting the complete architecture in GWZ, not installing its database package.

| Alternative | Commit point | Barriers per W1 | Critical section | Members stay ordinary Git on disk | Multi-host transactions | Operational burden | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A1 Local ledger | Durable local metadata transaction | 1 metadata flush after object durability; roughly 2 phases, but per-member O(K) object flush calls can remain | O(K) validation/update plus serialized writer flush; root serialization | Yes as projections; raw writes are foreign operations | No shared atomic authority across hosts as specified | Low–medium | L |
| A2 PostgreSQL everything | Durable database WAL commit | Usually 1 WAL flush carrying new bytes, amortized by group commit; checkpoint traffic extra | Row validation/updates and root row/ref lock through commit | No; clients/checkouts of server state | Yes through one service, not atomic writes to arbitrary independent hosts | High | XL |
| A3 DB metadata + CAS | Metadata commit after durable object publication | Object publication phase + metadata phase; actual calls depend on object packaging | Metadata validation/update and root lock | Optional projections, not authoritative raw writers | Yes through chosen distributed service | Medium locally, high distributed | XL |
| A4 One Git history | One authoritative ref update | Object durability + ref publication, about 2 logical phases; backend-specific actual flushes | Single root-ref publication | No; member histories are filtered views | Central owner only as specified; no independent multi-host authority | Medium | XL |
| A5 Daemon + virtual worktrees | Durable daemon WAL boundary | 1 if payload is in WAL; otherwise payload phase + WAL phase | O(K) validation/version publication and batch flush | No ordinary autonomous worktrees | Single host as stated; replication would be additional design | High | XL |
| A6 Deterministic stream | Durable ordered input fixes outcome; success waits for execution | 1 inline-input flush, or 2 phases with external payload; quorum writes when replicated | Brief sequencing; conflicting execution dependencies last longer | Private ordinary sandboxes; authoritative state is service-owned | Yes, replicated order and replayable inputs | Medium–high | XL |
| A7 Causal bundles | Local durable acceptance of whole event | 1 integrated event flush, or 2 payload/event phases | Local append; no global lock | Yes, pinned private views | Causal atomic bundles only; no global serializable head | Medium–high | XL |
| A8 Git descriptors | Durable shared decision over prepared ref versions | O(K) object/ref flush calls then decision; about 2 logical phases | Affected refs and root held through preparation/decision | Git objects yes; stock Git ref compatibility no | One storage owner only | High | XL |
| A9 OS transactions | Full-data filesystem journal decision | Target about 2 phases if write set fits; research design | Kernel write-set validation/install/flush; root still shared | Yes, inside the supported kernel | One kernel/storage owner only | Very high | XL |
| A10 Sealed capsules | Durable immutable artifact publication | About 2 file/name phases with durable prerequisites | Immutable-name publication; no shared current root | Yes; workspaces remain independent | Artifact transfer only; no shared mutable transaction | Low–medium | L |

SQLite WAL permits concurrent readers but one writer, and its durability settings matter; A1 needs durable commits rather than accepting power-loss loss of recent transactions. PostgreSQL asynchronous commit likewise must not be mistaken for durable acknowledgement. These are configuration requirements, not architecture speedups. [SQLite WAL](https://www.sqlite.org/wal.html), [PostgreSQL asynchronous commit](https://www.postgresql.org/docs/current/wal-async-commit.html)

## Speed rankings

These are **conditional engineering rankings**, not measured league tables. Lower numbered tiers are preferable; architectures in a tier have no justified strict order. Use the cost formulas to rerank for actual F, R, K, object sizes and root-lock work. “Ready input” means staged bytes and merge resolutions are known; it does not mean new authoritative bytes have already been durably written for free.

### Commit-point latency

For one machine, warm metadata, small W1 payloads, and a locally colocated service:

| Tier | Alternatives | Reason |
| --- | --- | --- |
| 1 | A5; A2 locally hosted | One integrated durable WAL boundary can include new bytes; group commit available |
| 2 | A1, A3 local, A4 | Object-before-metadata/ref publication; usually two logical phases; A1 may retain many object flushes |
| 3 | A6 | Admission can match tier 1, but successful commit also waits for ordered execution; cheap execution can bring it close to tier 1 |
| 4 | A8 | K ref/object installation calls and root publication lock remain |
| Conditional research | A9 | Could approach tier 2 for small journal-fitting write sets; journal amplification/capacity prevents a dependable production rank |
| Different contract | A7, A10 | A7 local acceptance can match tier 1; A10 sealing is roughly tier 2 plus packing. Neither certifies advancement of one shared mutable workspace |

As K moves 5 → 50 → 500, byte processing and worker waves increasingly dominate every tier. A1 can drop behind A6 if per-member object durability is expensive. A2 can drop behind local alternatives as soon as a network upload or RTT dominates. A4's publication is small, but preparing monorepo objects is not guaranteed to be.

Across five machines requiring one serializable accepted workspace, A2/A3 are the leading established mechanisms; A6 is the strongest new contender. For tiny payloads, colocated client/server routing and quorum RTT dominate their relative latency. A1/A4/A5/A8/A9 can serve remote clients through one owner but have no independently replicated authority as specified. A7/A10 win offline local acknowledgement by answering a weaker question.

### End-to-end latency

For W1 alone, W is usually zero; do not charge A1 for an imaginary full checkout or credit A5 for avoiding one. For W2 continue/materialize, W can dominate.

| Tier | Alternatives | Scope and reason |
| --- | --- | --- |
| 1 | A5 | Managed virtual view can become ready without eagerly copying W; later file reads still pay fetch/materialization costs |
| 2 | A1; A4 when filtered exports are not required | Local warm object access; W1 principally needs small ref/index projection work |
| 3 | A2, A3, A6 | Strong results, but client transfer, replay/execution and ordinary checkout readiness can add time; colocated A2/A3 may move up |
| 4 | A8 | Ordinary checkout plus K durable ref installations; W1 may overlap tier 2 at small K |
| Conditional research | A9 | W1 could be tier 2, but large W2 write sets can amplify journal traffic or exceed capacity |
| Different contract | A7, A10 | Existing local views are ready immediately because they did not move. A7 must resolve alternatives for a unique integrated workspace; A10 may create an entire new workspace. Neither has a finite general rank for today's in-place contract |

If end to end requires all K members pushed to ordinary hosts, filtering/export cost can move A4 much lower, and remote service time can dominate every candidate. If it requires all worktree bytes eagerly available, remove A5's lazy-materialization advantage. These are changes in the measured endpoint, not implementation regressions.

### Concurrent throughput

| Workload | Ranking / qualification |
| --- | --- |
| One machine, disjoint useful work | Tier 1: A5 and A2 local. Tier 2: A3 local and A6. Tier 3: A1 and A4. Tier 4: A8. A9 is unproven and may suffer false conflicts. A7/A10 can have very high proposal/deliverable throughput but are outside the strong-commit ranking. |
| One machine, overlapping branch/read sets | A6 is the strongest candidate for avoiding wasted speculative execution; A2/A3/A5 follow with established locking or validation approaches. A1/A4/A8 remain constrained by shared publication. Exact-base W1 operations still reject stale proposals in every strong design; no design makes incompatible commits simultaneously valid. A9 is unproven. A7/A10 defer integration cost rather than eliminate it. |
| Five machines, disjoint useful work | A2/A3 lead on maturity; A6 can lead if deterministic scheduling and partitioned execution pay off. A1/A4/A5/A8/A9 are limited by their single owner unless redesigned. A7/A10 lead in independently durable proposals, not globally serialized commits. |
| Five machines, overlapping branch/read sets | A6 is the best research bet for reduced wasted work; A2/A3 are the practical choices. All still serialize the common destination/root. A7 accumulates alternatives; A10 accumulates integration candidates. Neither guarantees greater resolved-merge throughput. |

All ten appear in each ranking, including explicit “different contract” and “unproven” placements. A total numeric ordering across those categories would manufacture precision. In particular, a submillisecond proposal receipt is not evidence of a faster completed 500-repository merge.

For any architecture with one root publication section of duration t_root, sustained coordinated commits are bounded by approximately 1/t_root before considering data bandwidth, CPU, or retries. Group commit can amortize flushes over a batch, but building each root commit's parent-dependent contents still imposes ordered work. Report successful committed transactions per second, not attempted or admitted requests.

## Recommendation and deciding facts

**For GWZ as described, select A1 as the practical first architecture, and treat A6 as the most promising genuinely new direction for a future shared multi-machine service.** The five additional alternatives expose useful tradeoffs, but none is an unqualified improvement over all A1–A5.

A1 buys one authoritative decision, straightforward recovery of managed state, preservation of member Git identities, and moderate operational burden. For transaction speed, use warm repository catalogs, parallel object construction, bounded parallel durability, short validation/publication, and explicit projection completion. Its object durability can remain O(K); measure that before promising two cheap barriers. A SQLite transaction alone does not fix arbitrary worktree writers.

To meet the no-manual-repair goal, adopt a corresponding worktree contract: each mutable lane has an owner, merges use durable private drafts, captured raw bytes are retained, and a dirty or externally changed checkout is preserved while a clean managed view is created at a new explicit path. Foreign Git activity is imported as an operation; never use polling as if it excluded concurrent writes. This is automatic preservation with a visible reconciliation state, not a claim that an arbitrary editor can be transactionally rolled back. If “every existing path must already match the new version when commit returns” is mandatory, A1's projection contract is insufficient.

For a product centered on many machines and one shared accepted branch, choose between A2/A3 and A6 using workload evidence. A6 is attractive when deterministic execution removes expensive repeated speculative work and a replicated input stream simplifies sharded execution. A2/A3 are safer when operations have broad dynamic read sets, arbitrary plugins, or complex external dependencies. Do not undertake a deterministic engine solely to save a presumed millisecond of local flush time.

Among the other new candidates, A10 is the most approachable separate feature: reliable agent handoff and preserved lane results. A7 is appropriate only if concurrent accepted versions are a desired product feature. A8 requires a Git ecosystem commitment with a weak speed payoff; A9 requires an OS product and should remain a research boundary case.

Facts that would change the decision:

- **Stock Git must freely mutate one shared accepted worktree while GWZ merges it, on all current platforms:** none of these proposals satisfies the full requirement. Change writer ownership, scope atomicity to logical state, or accept weaker guarantees; do not conceal the incompatibility.
- **A virtual filesystem is acceptable and checkout readiness dominates:** favor A5, particularly for many lanes and large W. Measure file-read and tooling behavior, not only view-switch latency.
- **All authoritative work already occurs on a server with reliable connectivity:** favor A2/A3; consider A6 after profiling overlapping transactions and deterministic-input feasibility.
- **Offline agents must independently accept work and multiple heads are acceptable:** favor A7. If only durable handoff matters, A10 is simpler.
- **One Linux appliance/VM is the whole supported product and kernel maintenance is funded:** A9 becomes investigable, though not automatically faster than a user-space service.
- **Upstream Git adopts cross-repository decision descriptors:** reevaluate A8. Without broad support, its ecosystem cost dominates.
- **Per-member history identity, signatures, and isolation can be sacrificed:** A4 becomes more attractive than any elaborate attempt to preserve independent authorities.
- **The measured bottleneck is cold scanning, hashing, index size or filters rather than durability:** fix that bottleneck first; changing the commit point may barely affect end-to-end performance.

Before selecting a store, measure W1 at K = 5/50/500 with separately reported capture, object construction, actual flush-call count and wait time, root publication, and checkout readiness. For W3 distinguish attempted requests, accepted proposals, successful commits, and resolved integrations. Exercise W4 at each durability boundary, including acknowledgement loss and repeated recovery crashes; exercise W5 with the actual supported writer contract. These are proposed acceptance experiments, not results produced for this document.
