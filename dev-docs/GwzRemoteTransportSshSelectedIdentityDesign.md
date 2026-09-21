# SSH N2 — selected identity admission

Date: 2026-09-21. Status: accepted design after retained Consistency/Safety GO.
Authority: accepted GwzRemoteTransportSshProductionSetup.md, N1 and A1–A3,
requirements G1–G3, and the operator's explicit-no-agent-fallback direction.
This specifies N2 only. Production backend attachment is N3; platform and
selected-source qualification remain the deferred batch before activation.
No public CLI/core, taut, transport Identity, pool key or wire change.

## 1. Selection and proof

Preserve existing invocation per-remote, invocation default, then repository
configuration precedence and endpoint-local absolute path resolution. Selection
may freeze a pathname for an operation; it may not cache file admission. Every
stream open reads the selected file again before any pool lookup, including an
open likely to reuse a connection. Missing, unreadable or changed files cannot
select a session authenticated from different bytes. Changes after a completed
read do not revoke that request's immutable snapshot; the next open reads again.

The native in-memory authentication API requires a handshaken session. It cannot
prove a never-seen snapshot usable before pool lookup without another parser.
N2 therefore distinguishes **admitted bytes** from **proven authority**:

- A successful bounded file read admits an immutable candidate, not a usability
  or authentication claim. Do not infer validity from path, metadata or a digest.
- Exact byte equality with a live entry for the same pool Key returns that
  entry's token, whether it is still a candidate or already proven. Interning is
  atomic in the worker after joined live file admission. Matching a token alone
  never authenticates a request or creates a usable lease.
- Otherwise allocate a fresh unproven token. It cannot match an entry for older
  or different bytes. Concurrent requests for identical bytes pin the same entry
  and token, so they can reuse a connection after its authentication succeeds.
- Every new physical connection authenticates independently. Only an idle,
  authenticated, reusable pool resource with matching Key/token can be reused;
  a candidate token or a registry Proven bit alone cannot satisfy that rule.
- A request's failure/cancellation does not promote authority, revive its request,
  or cancel independent requests sharing the entry. Another request may prove
  the same material by its own joined live native authentication. Reference
  ownership, deadline and cancellation stay per request/connection.

This explicitly refines ProductionSetup's “validated snapshot before allocation”:
current file availability/representation is admitted before lookup; fresh key
usability is established during new setup. Reuse requires the fresh bytes to
match the authenticated physical resource's material. Interning unproven bytes
is only compatibility bookkeeping. A malformed new file cannot reuse old authority.
There is no handwritten cryptographic parser and no token derived from private
bytes. Public fingerprints remain absent unless derived from proven public data.

First-fan-out invariant: if many same-Key requests admit identical bytes before
any authentication completes, their identities still match. With pool capacity
one, they wait and reuse the first successfully authenticated connection instead
of assigning each request an incompatible token and reconnecting for every repo.
This preserves the transport program's central connection-reuse benefit.

## 2. Bounded snapshot registry

One private registry belongs to one shared endpoint and its connector. It holds
weak references indexed by monotonic opaque token, never path/mtime as authority.
A token is never recycled, including after eviction. Counter exhaustion refuses.
Tokens are meaningful only inside that endpoint; no persistence or cross-endpoint
import. Existing Identity::Explicit(String) carries the token, within its current
1024-byte bound. Caller-provided arbitrary tokens cannot enter the production
selected-file route. Ambient and explicit admission are distinct internal variants.

Concrete internal limits, equal for local-core and driver-hosted endpoints:

| Resource | Bound and accounting |
| --- | --- |
| Selected file | 1 MiB, nonempty UTF-8 and NUL-free; opened descriptor must be regular |
| Live registry slots | 64, including unproven candidates and in-flight read reservations |
| Snapshot storage | 16 MiB of charged buffer capacity, including in-flight reads and retained cleanup owners |
| Setup/admission helpers | Existing process-wide A1 limit of 64 shared by both uses |
| Pending requests | Existing endpoint max_requests, held across admission and pool wait |
| Endpoint cleanup owners | Existing A3 limit of 64 active-or-retained owners; no extra unreserved owner |

Reserve a slot, a maximum read buffer (1 MiB plus one detection byte), and
256 bytes of classifier scratch before spawning file admission. Refuse unavailable capacity with WouldBlock, before
file I/O. Shrink the reservation only after storage capacity is actually released.
Successful matching pins the existing exact-byte entry (candidate or proven),
then discards the new buffer before releasing its reservation. New entries take ownership of the same admitted bytes.
Never allocate an uncharged overflow buffer to perform comparison or transfer.
Stale weak entries are pruned on registry access; map entries remain bounded.
No global lock may span file I/O, native work, waiting or a destructor callback.

This storage budget covers N2-owned snapshot/read buffers, not libssh2's internal
key representation or the pinned ssh2 wrapper's temporary CString copy. A native
auth call can additionally copy at most 1 MiB plus terminator; simultaneous calls
remain bounded by A1's 64-helper limit. No unbounded secret clone collection.
Private storage is not Debug/display/serialized; errors contain sanitized kinds,
not path, key bytes, native parser details, or a private-key hash.

An admitted request, a connecting Job, and an authenticated idle/active resource
hold strong entry handles for their entire use. The Authenticated/NativeResource
state transitions preserve the handle, including pump/reclaim/disposal. The
registry alone cannot keep bytes alive indefinitely. The final owner releases
snapshot capacity and slot only after native/Job disposal; a path or token cannot
resurrect a dead entry. Incompatible keys never evict an active owner to make room.
Admission can refuse at capacity even if the selected bytes might match: it must
read them under a reservation to know. No pathname-cache bypass under pressure.

Open using O_NONBLOCK on admitted Unix paths, then verify the descriptor is
regular, following existing endpoint-local symlink semantics. Read at most cap+1;
check Control before/after OS calls and before returning. OS file stalls have
N1's admitted retained-owner semantics, not a kernel-preemption guarantee.
This byte/encoding cap is an explicit accepted G1 exception: native file loading
may accept a larger or differently encoded representation that N2 refuses.
Refusal is InvalidInput/InvalidRequest before pool lookup, DNS or credentials.
A bounded preflight below additionally refuses unsupported/encrypted or ambiguous
containers before native authentication can run an uninterruptible KDF. This is
also an explicit accepted G1 representation restriction. Native parsing decides
accepted unencrypted key usability; do not normalize bytes.

### Bounded container preflight

Every selected-file admission runs this classifier before interning or checkout,
even when the bytes could match an old entry. It proves only that the representation
belongs to an allowed unencrypted container; it does not parse key mathematics,
validate a signing key, decrypt, or perform a KDF. Native authentication remains
the usability authority. Reject without invoking native auth on any ambiguity.

Accept exactly one PEM armor block, with matching case-sensitive BEGIN/END labels,
only ASCII whitespace outside, and base64 plus ASCII whitespace in its body.
Do not accept auxiliary headers, concatenated blocks, garbage prefixes/suffixes,
unknown labels or an encrypted block hidden behind a first allowed block. Use a
maintained base64 decoder with bounded streaming input and fixed scratch, not a
new handwritten decoder; inspect framing fields only. Read-only slices/streaming
views retain the original snapshot unchanged. Check Control between bounded
chunks. No complete decoded-key copy, algorithm execution or parameter-sized
allocation. The 256-byte scratch reservation includes framing lookahead.

| Container label | Pre-native disposition |
| --- | --- |
| ENCRYPTED PRIVATE KEY | Refuse immediately, regardless of password or KDF parameters |
| RSA PRIVATE KEY, DSA PRIVATE KEY, EC PRIVATE KEY | Admit only the header-free base64 body form; Proc-Type/DEK-Info and all other auxiliary headers refuse |
| PRIVATE KEY | Decode a bounded prefix and require definite-length DER PrivateKeyInfo framing beginning with version INTEGER 0 or 1; reject encrypted/ambiguous framing; native validates the remaining unencrypted key |
| OPENSSH PRIVATE KEY | Decode magic and bounded cipher/KDF strings. Require openssh-key-v1 magic, ciphername=none, kdfname=none, and empty kdfoptions; reject every other combination without parsing/using KDF work factors |
| All other labels/forms | Refuse as unsupported representation |

Every SSH/DER declared length is checked against input bounds before advancing;
a framing field longer than fixed lookahead refuses, never triggers allocation.
Malformed/truncated/ambiguous containers return InvalidInput (InvalidRequest)
before pool lookup, DNS, agent access or native authentication. These restrictions
can reject representations accepted by native file loading and therefore belong
to G1's explicit exceptions. Normal unencrypted OpenSSH, traditional PEM and
PKCS#8 fixtures must remain accepted; no new passphrase or prompt surface.

A native call can otherwise spend unbounded wall time in PBKDF2/bcrypt before
reporting a bad empty password. Passing None is not an encryption classifier.
The preflight excludes that unsupported work before the non-preemptible call.
Non-preemptible parsing/signing of admitted unencrypted keys remains under the
explicit supervised retained-owner policy; this is not a hard CPU termination
claim for arbitrary native operations.

## 3. Request and cleanup ownership

Keep Route selection side-effect-free: it captures an owned selection plan,
either Ambient or Selected(path, selection provenance). No synchronous file
read in IdentityResolver::resolve, Route::open, or Connector::start. N3 will
connect existing backend precedence to this plan without duplicating policy.

Endpoint admission acquires the existing pending-request permit and creates one
absolute deadline before starting any selected-file read. Use the existing
connect/allocation/interaction budget policy, including disabled-timeout mode.
Carry that deadline unchanged through file admission, queueing, pool checkout,
new network/authentication setup and stream handoff. Caller receive waits use
remaining time, never the original duration again. Cancellation/shutdown is one
request scope that reaches both admission and connection setup.

The worker owns a new pre-checkout Admission state:

1. Queued: owns plan, request permit, original deadline, cancellation and reply.
2. Admitting: reserves snapshot capacity and starts an A1 Job. The worker polls
   only; it never performs file I/O or joins a live thread. No pool checkout yet.
3. Admitted: joined result passes cancellation/deadline arbitration; request pins
   entry/token, then and only then invokes existing pool.checkout.
4. Connecting/reusing: existing physical pool machinery. On a new explicit setup,
   Connector looks up and pins the admitted entry, without path reopening or I/O.
   Missing authority is an error, not ambient fallback. Authenticated handoff
   checks token equality and native authenticated state. Only a successful joined,
   live handoff may promote a candidate to proven in the worker.
5. Active/released: original request permit ends as today; the physical resource
   pins authority until actual disposal. A later request must admit its file anew.

All owned admission slots live outside the worker's catch_unwind boundary, beside
PoolHost, so panic cannot make their ownership disappear. Cancellation stops
checkout/publication immediately, requests Job cancellation and begins disposal.
The request's reservation and admission permit remain charged until its helper
is joined and its discarded result destroyed, even if a logical error is replied
earlier. This needs a reply path that does not prematurely drop those owners.

Extend A3's retained endpoint owner to contain both PoolHost and admissions.
On shutdown or panic it cancels all admission Jobs and shuts down the pool.
cleanup_complete means both domains are physically disposed. Add internal
pending_admissions reporting alongside pending_connections; do not count a file
reader as a physical SSH connection. Any disposal overrun is sticky failure and
stops new endpoint admission. At the existing shutdown deadline the entire
unfinished owner transfers into its pre-reserved Cleanup slot. Its bounded
reaper advances both domains, including when no pool entries exist. Keep the
owner until every Job is joined and every pool resource disposed. No detached
file reader, synchronous join of live work, or false cleanup-complete flag.

The worker processes bounded admission work each tick, preserving existing stream
pump/timeout progress. New admission cannot monopolize the worker under a full
queue. Exact/past deadline wins before consuming a completed admission, matching
the existing queued-request rule. Late results cannot intern/promote a token,
call checkout, reopen a path, authenticate or publish a stream.

## 4. Native explicit authentication

Add a Unix internal file-key bridge alongside agent_auth. It takes exclusive
N1 connection ownership, independently approved host-key bytes, username,
immutable snapshot handle and original Control. Recheck the host key before
credential work. Use Session::userauth_pubkey_memory(username, None, bytes, None)
with nonblocking native session and bounded waits under the same deadline.
Repeat only EAGAIN progress with identical bytes; no whole-auth/address replay.
The preflight-admitted unencrypted native API derives the public key. A terminal failure destroys the connection
and cannot enumerate an agent, try another file, reopen this path, prompt, call
Git, or enter the old native transport as fallback. Keys requiring a supplied
passphrase remain unsupported; a nonempty-passphrase encrypted fixture must fail.

Native parse/sign work is performed inside the supervised setup Job. It may be
non-preemptible CPU work; cancellation is checked at native call boundaries,
late success discarded, and unfinished ownership retained as in A1/N1. Do not
claim that nonblocking network mode makes private-key parsing preemptible.
On native success require authenticated()==true before constructing the selected
Authenticated value and promoting its token after joined live handoff. Facts
record explicit-key method, successful authentication and a new credential offer
for new setup. Reuse clears the new-offer flag and retains proven authentication.
Never describe the opaque token as a public-key fingerprint.

## 5. Implementation packages and gates

- **N2a snapshot authority and native bridge:** registry/reservation/entry lifetime,
  file admission and bounded container preflight under A1 Job, memory-key
  authentication under N1 trust, and exact
  proof promotion. At most 600 added production lines across three cohesive files
  and 900 focused test/support lines. Does not activate routes or replace worker
  request flow. Focused aggregate Code/State review on its settled tuple.
- **N2b worker admission integration:** queued/admitting/checkout ownership,
  unchanged absolute deadline, selected Route plans, resource authority pins and
  retained cleanup covering admissions plus pool. At most 500 added production
  lines across existing worker/endpoint/setup/shutdown boundaries and two cohesive
  modules, plus 900 test/support lines. Excludes backend wiring and platforms.
  Aggregate Code/State review; prove existing ambient behavior still works.
- N3 owns production backend selection/observation attachment and every network
  driver. It must remove any old path-validation/authentication route that would
  bypass N2's supervised admission. No capability activation in N2a or N2b.

Required causal tests, without a count-based acceptance gate:

1. Same bytes via same or alternate path can reuse a proven same-Key token;
   changed/deleted/unreadable/invalid file cannot reuse it. Change the pathname
   after admission and prove native auth receives the original snapshot.
2. Malformed/unsupported/encrypted containers refuse before native auth; fresh
   admissible but unusable key content cannot reuse old authority and native
   failure has no agent/address/file/credential fallback. Observe zero native-auth
   calls for encrypted PKCS#8 with extreme PBKDF2 parameters, traditional encrypted
   PEM, and OpenSSH with extreme bcrypt rounds; never actually run those KDFs.
   Saturate with these fixtures and prove bounded refusal, helper/byte recovery,
   truthful cleanup completion and continued progress of an active stream.
   Cover multiple/mislabelled/truncated blocks, length overflow, nonempty cipher
   or KDF fields, and auxiliary PEM headers. Accept unencrypted Ed25519 and RSA
   OpenSSH, traditional PEM and PKCS#8 through real auth/Git exchanges. Classifier
   loops and scratch stay input-bounded; native usability is not inferred from
   a successful classification.
3. Token lifetime across queued, connecting, idle, active, cleanup-overrun and
   endpoint-drop states; no token recycling or stale resurrection. Concurrent
   fresh candidates for the same bytes share one token, but cannot obtain a
   lease from a connecting/unauthenticated resource. Admit a same-Key batch
   before releasing a native authentication barrier, with physical capacity one;
   all successful Git exchanges must use one connection. Changed bytes remain
   incompatible. Cancel/fail one sharer while another authenticates and prove
   independent outcomes, no early lease and retained ownership. Only worker-side
   joined live success promotes authority; wrong-token/failed proof refuses.
4. Slot and byte reservation exhaustion/recovery, oversize exact/below/above,
   UTF-8/NUL refusal and FIFO/nonregular rejection before checkout/resolution.
   Model small injected limits deterministically; native representative cap
   differential records the intentional refusal where native accepts a fixture.
5. Cancellation/expiry before/during/after admission, stalled read with no kernel
   preemption claim, late success after cancellation, endpoint shutdown and panic.
   Assert no checkout/auth/stream effect and no early capacity/cleanup release.
6. Admission-only retained endpoint disposal, combined pool/admission cleanup,
   bounded caller/worker exit, sticky failure and eventual disposal. Exhaust the
   shared helper cap; neither admission nor network setup gets an extra allowance.
7. Original deadline consumed by admission leaves only remaining pool/setup wait;
   exact/past queued expiry and disabled-timeout explicit shutdown. Progress an
   existing active stream while another request's file reader is stalled.
8. Explicit/ambient and different-Key isolation, alternate selected keys under
   pool limits, fresh/reused Facts, and regression of accepted A1–A3/N1 behavior.

Use controlled native fixtures, never user keys/agents. Public tests remain in
core; raw campaigns and failed attempts in the private evidence member. Retained
Consistency/Safety review accepts this design and the stated G1 refinement first.
P0–P2 block; at most two merged remediation rounds per object. Budget increases
must state excluded scope first, with no split solely to conceal aggregate size.

## Owner remediation 1 — first-fan-out compatibility

The initial design required a distinct token for every concurrent unproven
snapshot. Against pool/allocation.rs, requests queued before the first native
authentication would all be incompatible: with capacity one, each released
connection would be evicted for the next token. That recreates one SSH connect
per repository during the first fan-out. Owner identified this before design
acceptance, independently of the initial reviewer verdicts.

Canonicalize all live identical same-Key snapshots, including candidates. The
existing pool/native-resource authenticated eligibility gate, not token uniqueness
per request, prevents unauthenticated reuse. This changes no public shape, pool
key or native-authentication requirement. The barrier/batch gate above makes the
performance contract testable along with cancellation and proof isolation.

Safety P2-1: the original absent-passphrase call could still enter an encrypted
container's attacker-sized KDF. The bounded unencrypted-container preflight above
now refuses this path before native authentication, with explicit G1 representation
exceptions and saturation/progress tests. No decryptor, key-math parser or process
supervisor is added. N3/backend/platform work remains excluded; N2a production
budget stays600 lines, and its test allowance800→900 covers the classifier
adversarial matrix. This is the same merged remediation as the fan-out correction.

## Design acceptance

Accepted at root `a9ad12dcafb51d77e7d0fbd28fac97934e070b09`, core
`35df881b7075d7031082f61e0b99b838341149e1`, evidence
`e842abf855e58de3c4381855fbc1b8374485a7cd`, transport
`28f5afb3938a2aa8af0e1e8d5b07779add6ab776`, git2-rs
`ce78628308e11b4e8901d5061602619109bce21a`, libgit2
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348`, after retained
[Consistency GO](../../dev-docs/GwzRemoteTransportSshSelectedIdentityDesign-ReviewConsistency-1.md)
and [Safety GO](../../dev-docs/GwzRemoteTransportSshSelectedIdentityDesign-ReviewSafety-1.md).

One initial dual review plus one merged remediation. Safety P2-1 encrypted-KDF
admission and owner P2-O1 first-fan-out token incompatibility are closed at the
design boundary. No independent dual-axis convergence; no implementation or
escaped-code defect claim. Reports are filed verbatim. This documentation-only
gate ran inspection and diff checks, not native or platform experiments.

Next is N2a implementation under §5: bounded snapshots/registry, framing preflight
and native in-memory authentication, TDD then retained aggregate Code/State review.
N2b worker admission and combined cleanup, N3 backend attachment, and the deferred
platform/source qualification remain separate required gates before activation.
