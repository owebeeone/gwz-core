# Interruptible SSH agent helper

Date: 2026-09-21. Status: DRAFT for dual design review; no implementation or
production activation accepted by this document.

## 1. Decision and authority

The operator selected a helper thread with agent I/O owned by GWZ and capable
of cancellation. Use synchronous control flow on that helper, with bounded OS
waits underneath. Do not wrap opaque `ssh2::Agent` blocking calls and call the
result cancellable. A receiver timeout, dropped JoinHandle, or thread-per-retry
replacement does not terminate the underlying work.

This refines [the transport design](GwzRemoteTransportDesign.md) §§7–8 and 10,
[requirements](GwzRemoteTransportRequirements.md) G1/G2, P6–P9, and the next-work
section of [the accepted worker](GwzRemoteTransportSshWorker.md). It supersedes
only the worker document's candidate of making the shared worker itself service
agent I/O: agent exchange and signing now belong to a setup helper. The shared
worker remains nonblocking. Existing messages, CLI/core API, per-remote binding,
pool keys, eligibility rules and native Git stream semantics are unchanged.

This is an architectural design review, not a Track-P physical capability
freeze. The operator's deferred platform/selected-source qualification batch
remains outstanding; no platform capability is advertised before its primitive
and cancellation behavior pass. Local implementation tests proceed first.

## 2. Boundary and staging

A short-lived helper owns one connection-setup attempt. In the first executable
slice it receives an already connected, handshaken, fixture-trusted owned
`SshConnection`, a username, and an endpoint-local agent address. It authenticates
and returns the whole connection, or destroys it on failure. This isolates the
agent/signing problem without pretending DNS, key-file I/O or trust lookup has
already become bounded. No helper opens a Git command channel.

Later production setup must perform connect, handshake and host-trust validation
before authentication and transfer using the same ownership discipline. Blocking
DNS, filesystem calls, passphrase prompts and platform discovery must not be
smuggled into this qualified slice. They require their own bounded adapter or
explicit admission contract before activation. Existing explicit file identity
selection remains fail-closed: it must never fall back to an ambient agent.
Encrypted file-key/exact-agent support remains unavailable until separately
qualified; ambient agent support does not implicitly enable that feature.

GWZ owns only the agent operations needed for authentication: list public keys
and sign. It does not implement agent key storage, add/remove/lock operations,
agent forwarding, a new identity namespace or user credential management.
Native SSH negotiation and server communication remain libssh2's responsibility.

## 3. Ownership and internal interface

`SetupJob` is a private resource owned by the endpoint's physical pool entry.
Its conceptual operations are start, poll-result, cancel and poll-disposed.
Exact Rust names may follow existing Connector/Resource conventions; this does
not freeze a new public API. Starting it must not wait for DNS, agent connection,
a reply, authentication or join. Thread creation failure releases its reservation
and returns a setup error before any authentication is offered.

| Owner | Exclusive responsibility |
|---|---|
| Shared endpoint worker | Pool ledger, request deadlines, readiness polling, other streams and setup cancellation |
| Setup helper | Native SSH session during setup, its TCP owner, agent handle, codec and signing callback state |
| Cancellation control | Monotonic cancellation flag and platform wake primitive; no native session access |
| Completion cell | At most one owned result, atomically claimed or discarded; never clones a session |
| Setup supervisor | JoinHandles, helper permits, abandoned/unclaimed results and exceptional cleanup overruns |

The supervisor is process-local infrastructure, not a service/daemon or transport
registry. It imposes a fixed process-wide cap of 64 setup helpers, additional to
the endpoint's physical pool and per-host limits. Every live, completed-unjoined
or quarantined helper consumes one permit. No automatic pool expansion or
replacement of a still-live helper is allowed. Cap exhaustion refuses setup
with capacity failure; it must not spawn another thread or fall back to Git.
Use one bounded supervisor control loop for all endpoint instances, not one
reaper thread per job. At most 64 job/result/control records are retained.

Reserve pool capacity and a helper permit before spawn. The helper drops its
agent handle and all signing state before publishing a result. The worker takes
success only after helper termination and join, and before that treats it as
connecting. A failed join/panic makes the result unusable. Cancellation and
shutdown win over unclaimed success; the whole connection is terminated rather
than promoted to ready. Join is called only once the thread reports finished.
The native session is never accessed concurrently or cloned across the handoff.

A generation-tagged job ID prevents a stale completion/cancel from affecting a
replacement. Completion publication and claim use one state lock; cancellation
sets a monotonic flag under that same arbitration. Wakers/condition notifications
occur after releasing the lock. The slot contains one result, not a blocking
send that can trap the helper when its requester disappears.

## 4. Interruptible agent I/O

The public-key agent client exposes ordinary sequential connect/send/read calls
to the helper, but each implementation checks cancellation and an absolute
monotonic deadline before and after progress and before accepting completion.
Use nonblocking handles plus bounded readiness waits; never rely on a native
agent library's hidden socket or an unbounded `read_exact`/`write_all`.

A readiness wait lasts no longer than min(remaining deadline, 20 ms). A cancel
wake should end it earlier, but bounded polling remains the fallback. Retry
Interrupted/WouldBlock only after rechecking cancellation and remaining budget.
Readiness is not data and does not extend a deadline. Connect, partial writes,
partial frame-header reads and partial body reads all obey this rule. All work
per iteration and all buffers are bounded. Spurious wakes do not cause tight
retry loops. Agent closure, truncated response and unexpected response type fail
the attempt; there is no reconnect/replay of an outstanding signing operation.

For Unix sockets, use an owned nonblocking connection from creation, including
connect-in-progress and its completion-error check. For Windows named pipes,
the candidate adapter must use cancellable overlapped operations and retain
operation buffers/OVERLAPPED ownership until cancellation completion is observed.
Do not free buffers merely because CancelIoEx was requested. No synchronous
named-pipe or Pageant helper is admitted by this design without an equally bounded
proof. Unsupported agent transports refuse explicitly on the selected endpoint.
The platform API spelling is a proposal until the deferred native checks pass.

Cancellation is a state transition, not dependent on socket shutdown succeeding.
The helper's bounded wait returns to inspect the flag and close its owned agent
handle. A supported shutdown/cancel syscall accelerates it; plain close of a
second handle is not the sole interruption mechanism. Check cancellation before
publishing a successful signature and again before authenticated handoff.

## 5. Agent protocol and signing bridge

Reuse the agent protocol understood by the pinned libssh2 implementation:
network-order u32 frame length, message byte, and bounded SSH strings. Support
identity request/answer (11/12), sign request/response (13/14), and failure (5).
These are SSH-agent messages on the endpoint, not a new GWZ/taut wire protocol.

Limits for the first slice: one in-flight request per helper, 1 MiB maximum
frame, 256 identities, 64 KiB per public-key blob, signing input and signature.
Check lengths before allocation, use checked offset arithmetic and reject
trailing bytes, truncated fields, count/size overflow and invalid response types.
Discard agent key comments rather than retaining or logging them. Stop after
one bounded identity enumeration and at most one authentication attempt per
listed key in order; no recursive retry or repeated enumeration. All attempts
share one remaining setup deadline. A fresh operation may make a fresh attempt.

Add a narrowly scoped private binding to libssh2_userauth_publickey rather than
patching libgit2 or emulating SSH authentication. Its callback executes solely
on the setup helper, so it may synchronously await the bounded agent exchange
without blocking the shared worker. The SSH session/TCP remain nonblocking;
libssh2 network EAGAIN returns to the helper's bounded socket-wait loop. No
WouldBlock is exposed to libgit2's blocking Git stream adapter.

The binding must prove callback userdata lifetime across all native retries,
no Rust unwind across FFI, checked signature size and native allocator ownership
on every success/failure path. Signature memory must use the allocator paired
with that session, and be transferred exactly once. Do not assume Rust Vec
allocation is acceptable. The pinned ssh2 constructor uses libssh2's default
allocator; matching allocator/CRT behavior remains a native binding gate,
particularly on Windows. Failure to establish that pairing blocks that adapter.

Follow libssh2's negotiated signing algorithm, including RSA SHA-256/SHA-512
agent flags; do not infer algorithm from a filename, silently downgrade to
SHA-1, or pass the complete agent signature envelope where native code expects
raw signature bytes. Validate the returned algorithm and nested string shape
against the requested method before handing it to libssh2. Ed25519 and RSA
modern-signature fixtures are required; unsupported algorithms fail explicitly.
A repeated native callback for the same pending authentication must preserve
stable callback input/state and must not duplicate an in-flight agent request.

## 6. Time, cancellation and disposal

For the first slice no interactive prompting capability is advertised. Agent
list/sign waits spend the same remaining connect/auth budget as SSH setup;
absence of bytes is not evidence of user interaction and cannot pause that clock.
Zero disables that normal network deadline only: cancellation and bounded cleanup
still work. Interactive consent support, if later added, needs the existing
visible/cancellable interaction accounting and cumulative allowance propagated
from pool setup to stream; it is not silently introduced by this design.

The worker advances its clock before processing helper completion. At an exact
expired setup deadline, cancellation wins even if authentication just succeeded.
The helper checks the same deadline before every local/network operation and
publication. Neither side may reset it when moving between agent and SSH I/O.
Completion received after cancellation or owner loss is discarded, never reused.

Normal state progression:

`Reserved -> Running -> Finished -> Joined -> Transferred | Disposed`

Cancellation from Reserved/Running/Finished enters Cancelling, requests wake,
terminates owned I/O, then reaches Finished/Joined/Disposed. An unclaimed success
in Finished is still owned by the job and must be destroyed on cancellation.
Pool disposal acknowledgment and helper permit release require thread exit,
join, destruction of all native/agent owners and removal of unclaimed results.
Successful transfer instead releases the helper permit after join while keeping
the pool's physical connection charged to the new owner.

`poll_dispose(force)` only requests cancellation and polls completion; it must
never perform a blocking join in the shared worker. At the cleanup deadline
(default pool cleanup budget 5 seconds), a still-live helper is a cleanup failure,
not successful disposal. Refuse new opens on that endpoint and report cleanup
failure. The supervisor retains the job, cancel control, result and handle;
its global permit remains occupied and cannot be bypassed by recreating endpoints.
When it eventually finishes, destroy results and join before releasing the permit.
It never becomes eligible for reuse. Never kill a thread, detach and forget it,
claim an OS real-time scheduling guarantee, or report shutdown complete while
such work remains. Process termination is the final OS cleanup boundary.

Endpoint Drop requests cancellation and transfers any unfinished jobs to that
already-existing supervisor; it does not wait indefinitely or acknowledge pool
capacity as physically freed. Normal explicit shutdown waits/polls for joined
cleanup; on an overrun it returns failure with pending cleanup, not success.
This requires refining the private Resource Drop contract, currently phrased as
synchronous socket termination, to permit retained supervised setup ownership.
It does not weaken ready/active SshConnection disposal. The shared worker's
current fire-and-signal shutdown API needs an internal observable cleanup result
before production activation; no new CLI flag or taut message is required here.

## 7. Security and authentication facts

Host trust must pass before signing/authentication. All agent addresses, public
key lists, private-key inputs and signatures remain endpoint-local. Bounded
errors identify phase/code, never raw agent replies, comments, signature bytes,
private data or credential-bearing URLs. Endpoint-local authority is checked
before each allocation; setup success records proven authentication only after
libssh2 reports authenticated. A queued thread or accepted sign request is not
proof. Reuse does not falsely record a new credential offer. Ambient and explicit
pool eligibility remain separate under the same username/host/effective-port key.

No helper threads, callbacks or OS handles belong in gwz-transport. Its message
stream and pool ledger stay executor- and wire-independent. Concrete ownership
and cancellation live in gwz-core's endpoint adapter; a reusable private agent
module may be extracted later if justified, not made a new repository now.

## 8. Implementation and review gates

1. A1: isolated bounded agent codec/client and helper lifecycle, fake agent tests.
   Budget <=500 production lines across <=3 files; <=700 focused test/support
   lines. No native signing binding or production setup activation in this slice.
2. A2: minimal private signing binding and helper-to-connection handoff using
   fixture-trusted native sessions. Budget <=350 production lines across <=2
   files; <=500 tests. No broad key-format parser, DNS or backend routing change.
3. A3: integrate supervised setup resource, failure/cleanup observation and shared
   endpoint ownership. Refine limits before adding full production discovery,
   identity/trust I/O or call-site activation. Those remain required, not waived.

TDD gates: fragmented headers/bodies, partial writes, malformed/oversized replies,
EOF mid-frame, unavailable agent, connect pending, peer never reading/replying,
exact deadlines, disabled network timeout plus explicit cancellation, cancellation
at every publication/claim/join boundary, repeated cancellation, callback panic,
late success after owner loss, thread creation failure, global cap across recreated
endpoints, forced cleanup overrun/quarantine and eventual reap. Seeded randomized
chunking uses a printed replayable seed. No user agent or key is used in tests.

A2 adds trusted loopback authentication, wrong host rejection before signing,
Ed25519/RSA algorithm and signature-shape cases, rejection across multiple keys,
sign callback/native retry ownership, shutdown during a sign wait and successful
join/connection transfer. A stalled helper must not stop another active Git
stream or the worker's timers. Test cleanup asserts helpers/handles are gone;
a timeout returned to the test caller alone is not passing evidence.

A1/A2 local gates precede aggregate implementation acceptance. Platform-specific
handle cancellation, allocator ABI, agent discovery and source distribution stay
in the operator-deferred batch and gate capability activation. Design review uses
retained Consistency/Safety reviewers on an exact committed tuple, peer blind;
P0–P2 block; at most two merged remediation rounds. No user-facing surface added.

## 9. Evidence and limitations

The existing public agent_wait fixture and private worker-a record demonstrate
only that ssh2 0.9.6/libssh2-sys 0.3.3 agent-list waiting is not bounded by session
nonblocking mode or its timeout. Native agent.c implements blocking local-agent
I/O; userauth.c provides the public-key callback and its retry behavior. Those
source observations support this design but do not qualify the new client or FFI.
The current task produces a reviewed design only; all new implementation gates
above remain unexecuted. Raw future experiments follow EVIDENCE.md and stay in
the private evidence member with external build/runtime outputs.
