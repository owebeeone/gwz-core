# SSH N2 — selected identity admission

Date: 2026-09-21. Status: DRAFT for retained Consistency/Safety review.
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
- Exact byte equality with a live, previously native-proven entry for the same
  pool Key permits that entry's opaque token. Existing pool equality still
  requires both Key and Identity; reuse still requires an idle authenticated owner.
- Otherwise allocate a fresh unproven token. It cannot match an old pool entry.
  It can request a new connection, whose native authentication must succeed
  before the entry becomes proven or any stream receives an authenticated lease.
- Concurrent unproven candidates are not interned together. A failed/cancelled
  candidate never lends another request a usable proof. A proven entry is a
  record of native usability; each new physical connection still authenticates.

This explicitly refines ProductionSetup's “validated snapshot before allocation”:
current file availability/representation is admitted before lookup; fresh key
usability is established during new setup. Reuse requires the fresh bytes to
match already-proven material. A malformed new file cannot reuse old authority.
There is no handwritten cryptographic parser and no token derived from private
bytes. Public fingerprints remain absent unless derived from proven public data.

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

Reserve a slot and a maximum read buffer (1 MiB plus one detection byte) before
spawning file admission. Refuse unavailable capacity with WouldBlock, before
file I/O. Shrink the reservation only after storage capacity is actually released.
Successful matching discards the new buffer before releasing its reservation and
pins the existing entry. New entries take ownership of the same admitted bytes.
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
This byte/encoding cap is an explicit proposed G1 exception: native file loading
may accept a larger or differently encoded representation that N2 refuses.
Refusal is InvalidInput/InvalidRequest before pool lookup, DNS or credentials.
Native parsing otherwise decides key-format usability; do not normalize bytes.

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
The native API derives the public key. A terminal failure destroys the connection
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
  file admission under A1 Job, memory-key authentication under N1 trust, and exact
  proof promotion. At most 600 added production lines across three cohesive files
  and 800 focused test/support lines. Does not activate routes or replace worker
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
2. Fresh malformed and encrypted candidate has no old token and fails native
   auth. Zero agent factory calls and no extra address/file/credential attempt.
   Successful Ed25519 and RSA native auth followed by a real Git exchange.
3. Token lifetime across queued, connecting, idle, active, cleanup-overrun and
   endpoint-drop states; no token recycling or stale resurrection. Concurrent
   fresh candidates cannot turn unproven equality into reuse. Only worker-side
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
