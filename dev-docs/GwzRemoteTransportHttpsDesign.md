# HTTPS endpoint adapter design

Status: **DRAFT — pending retained Consistency/Safety review**, 2026-09-22.
This admits candidate implementation only. It does not activate production
routes, freeze a new public constructor/command, or qualify physical CLI/core wire.
Authority: [Requirements G4/C8/P4](GwzRemoteTransportRequirements.md),
[Design §§3,9,10](GwzRemoteTransportDesign.md), and
[Plan Phase5](GwzRemoteTransportPlan.md). Accepted placement A/B/C remains intact.

## 1. Outcome and ownership

A Git HTTPS remote uses the existing per-remote git2 callback, a gwz-transport
stream, and an HTTP adapter at the selected endpoint. Core owns Git negotiation
and repository effects. The endpoint owns DNS, TCP, TLS, proxies, gh, HTTP headers,
connection health and disposal. The same adapter serves local and CLI placement;
CLI placement continues to use optional attachments on existing operation messages.
No token, Authorization header, socket or HTTP-client handle crosses those messages.

`gwz-transport` remains physical-I/O-free: shared Taut protocol, byte-stream
semantics, credit, deadlines, terminal facts and generic pool accounting. HTTPS
network implementation stays in `gwz-core/src/git/endpoint`, beside SSH. No new
repo, Git executable fallback, parallel schema, transport service or carrier.
The existing `Scheme::Https`, `AuthPolicy::{Anonymous,Gh}`, four `GitService`
values, `Data`, `EndWrite`, `Closed.facts` and HTTPS pool key cover this design.

## 2. Concrete implementation choice

Use Hyper 1's **per-connection HTTP/1 client** (`client::conn::http1`), Tokio I/O,
and tokio-native-tls/native-tls for endpoint-owned TLS. Use hyper-util only for
runtime I/O adaptation, not its pooled client. HTTP/1.1 with one request per
exclusive lease matches the accepted pool; HTTP/2 multiplexing is not required.
Pin exact dependency versions and features in the external candidate manifest
and lock during H1, including required buffer controls; do not change production
Cargo inputs in this design package. A failed library qualification returns to
this design; it does not authorize substituting a client with hidden semantics.

Reasons: the low-level sender is attached to one physical connection, while its
connection future is independently driven. This lets GWZ own retry decisions,
physical connection IDs, bounded admission and disposal. High-level reqwest or
Hyper legacy clients would add a second pool and implicit policy to qualify.
Native TLS is chosen for platform trust integration; platform equivalence is a
later qualification result, not inferred from using that library.

One bounded endpoint runtime drives sockets, connection futures, stream pumps,
helper pipes, cancellation and pool timers. No thread per request or socket.
Blocking host facilities (DNS/system trust access) run in bounded owned jobs;
a timeout does not imply the underlying call stopped. Retain their accounting
until completion; saturation refuses admission. Runtime teardown never waits
unboundedly for these jobs. Use the existing SSH job-ownership discipline,
without putting HTTP work on the SSH agent worker.

## 3. Git RPC mapping and message boundaries

| git2 service | Existing Taut service | Endpoint HTTP request |
| --- | --- | --- |
| UploadPackLs | UploadPackAdvertisement | GET `<base>/info/refs?service=git-upload-pack` |
| UploadPack | UploadPackExchange | POST `<base>/git-upload-pack` |
| ReceivePackLs | ReceivePackAdvertisement | GET `<base>/info/refs?service=git-receive-pack` |
| ReceivePack | ReceivePackExchange | POST `<base>/git-receive-pack` |

HTTPS gets a separate RPC smart subtransport (`smart_transport(true, ...)`).
Do not reuse SSH's stateful adapter, which collapses advertisement and exchange.
Every RPC action creates a fresh logical stream, including successive negotiation
POSTs; the endpoint may lease the same physical connection sequentially.
Per-remote state owns cancellation and the current RPC, not an authenticated
socket. libgit2 frees/reset streams between RPC rounds without necessarily
calling subtransport close. The RPC stream destructor therefore cancels unfinished
work, while successfully read EOF explicitly closes and accounts for its lease.

The Git-facing wrapper has shared state `Writing -> Ending -> Reading -> Done`.
On the first **nonempty** response read, call `end_write()` exactly once, await
its completion, then read. A zero-length read is a no-op. Further writes fail;
flush only flushes buffered bytes. Advertisement streams have no request body:
end their write side before the first response read and reject nonempty writes.
Endpoint body EOF is produced only on received EndWrite after all prior Data.
Do not scan Git pkt-lines for body end; a pkt-line flush may precede a push pack.

Send Opened after validating policy, acquiring a live connection and preparing
request execution, **before awaiting POST body or HTTP response**. Waiting for
an HTTP result before Opened would deadlock a caller waiting to write its body.
For GET likewise use normal Opened followed by response bytes or typed failure.
Before handing any response body to Git, validate status and content type;
advertisements retain the service pkt-line prefix expected by libgit2's RPC
parser. No second Git parser is introduced. Use the appropriate
`application/x-git-<service>-request/result/advertisement` types. Smart protocol
only: a dumb response is UnsupportedOperation, not an alternate native fetch.
No response cache, cookies, arbitrary extra headers or new Git protocol version
negotiation is introduced. Existing libgit2 negotiation remains authoritative.

Unknown-length POST bodies use HTTP/1.1 chunking, never pack-sized buffering or
temporary spool files. Request body polling pulls at most a bounded chunk from
the transport receive window. Response polling stops when reverse credit is
exhausted. Header parsing, TLS buffering and the HTTP body adapter have separate
explicit bounds; credited transport bytes are not permission for unbounded
HTTP allocations. Early error responses cancel a still-writing body and wake its
writer instead of waiting for EndWrite. EndWrite on the response means validated
HTTP body EOF, not TCP EOF or a truncated Content-Length/chunked response.

## 4. gh authentication and anonymous access

Keep `Anonymous` and `Gh` distinct. Anonymous opens never spawn gh and send no
Authorization. Gh opens require a successful credential lookup before issuing
HTTP; missing executable/login/usable credentials fails Authentication with an
actionable, redacted message. There is no fallback after explicit Gh failure.

To preserve anonymous public access in the ordinary helper-enabled backend,
start discovery anonymously. If discovery returns 401, or 404 (a host may hide
private repositories), the core RPC adapter may retry that **GET only**, once
with a new stream carrying Gh. This transition is allowed only when the caller's
credential-helper policy permits gh and the negotiated endpoint supports it.
It retains request/operation/placement and cumulative budgets. If disabled,
return the original refusal and never invoke gh. No fallback to another provider.
A successful authenticated discovery pins Gh for subsequent RPCs on that remote.
A successful anonymous discovery keeps Anonymous. A POST challenge never triggers
automatic replay; report the refusal so the user can resolve authorization.
This is intentionally conservative for public-read/private-write repositories:
H1 must establish a pre-POST Gh selection for ReceivePack when helpers are allowed
(discovery uses Gh from the start for that service), so authenticated push does
not rely on retrying a body. Explicit Anonymous remains possible and unchanged.

Invoke a resolved endpoint-local executable directly, without a shell:
`gh auth git-credential get`. Feed bounded stdin with `protocol=https`, canonical
`host` authority (including nondefault port), repository `path`, then a blank
line. Do not pass a username selector or a token argument. Parse a complete,
bounded credential response, reject duplicate/malformed username/password fields,
require nonempty values, reject control characters and username colons, and
construct HTTP Basic credentials only inside the endpoint. Never invoke store,
erase, setup-git, login, refresh or an arbitrary configured helper.

gh owns active account selection, its keychain/configuration and documented
endpoint environment-token precedence. GWZ does not read the store or add an
account namespace. Snapshot the endpoint environment at runtime construction,
allow gh to use that environment, and do not import core's environment when
placement is CLI. Persistent gh account changes are observed at the next lookup;
no claim of account pinning across separate RPCs. gh's own host/gist mapping
remains its policy; GWZ must not strip ports or invent hostname aliases to find a
token. Enterprise host/port and environment-token cases require qualification.

Look up credentials afresh for each Gh HTTP request, including a redirected GET
on a new origin. Hold them only for that request; no application-wide token cache.
An authentication rejection never silently retries with the same or refreshed
token. The user changes gh state and retries the operation. Retain no raw helper
output or stderr in events/errors/evidence. Typed failure distinguishes missing
gh, missing credentials, malformed output, timeout and cancellation using existing
codes plus fixed redacted text. Credential material has no Debug representation;
clear owned buffers on disposal where supported, without claiming erasure of all
library copies.

Helper stdin/stdout/stderr progress concurrently under a shared interaction
budget; enforce 16KiB stdout and 16KiB stderr maxima, bounded to smaller negotiated
metadata limits where applicable. Disable terminal prompting, close stdin after
the request, cancel/kill and reap the owned child on deadline, and retain a bounded
cleanup entry if reaping or inherited pipes outlive the logical deadline. Do not
block the message pump or create unlimited replacement jobs while entries remain.

## 5. Destinations, redirects, trust and proxies

Validate the effective remote URL after Git URL rewriting and again at endpoint
admission: HTTPS, canonical host/port, repository path, no userinfo (even username
only), query, fragment, controls or ambiguous escaped authority. Reject before
Open/helper/network effects with redacted errors. Preserve path escaping exactly;
append the service suffix via a structured URL builder without double decoding.
The fixed **adapter-generated** discovery `service=` query is allowed: the ban
is on caller-supplied credential-bearing destination forms, not this Git-required
query. Transport Destination still contains only the repository base path.

Redirects are explicit, maximum five GET hops and bounded by the same operation
budgets. Only 301/302/303/307/308 with one valid Location may redirect discovery.
Resolve relative Location against the actual request; allow no userinfo, fragment
or arbitrary query. An advertisement redirect may retain exactly the expected
single `service=` pair; remove that generated suffix/query to obtain a new valid
repository base, then rebuild and compare the resulting request exactly. Other
shapes refuse. Reject HTTPS downgrade before any new host/helper access.

Keep redirect resolution endpoint-local. Cache only the validated effective
repository base for this registered operation + original destination + service
family, so later POSTs use the redirected base without first publishing to the
old location. This bounded route record holds no credentials, expires on request
finish, and is never shared across operations. Different service families discover
separately. Each new origin gets a new pool key and gh lookup under the selected
policy. Never copy Authorization across origins. Release/discard the old lease
before acquiring the next; no two-origin hold-and-wait. **Never follow a POST
redirect or retry a POST**, even 307/308, whether or not a body was fully sent.
Return an actionable typed error with conservative effect accounting.

TLS verification and hostname checks are on. Load platform roots and explicit
endpoint-local CA policy; core never resolves a CLI endpoint's CA paths. Keep
trust/proxy configuration immutable for a runtime so a pool key cannot reuse a
connection under a changed trust route. Configuration reload recreates the endpoint
and disposes old connections. Certificate/client-key auth is outside gh-only
origin authentication; unsupported requested policy must refuse explicitly.

Before activation, inventory the current native backend's effective TLS/proxy
settings and cover each used setting with support or an approved, documented
compatibility disposition. H1 candidate supports system trust and injected CA,
direct TCP and endpoint-configured HTTP/HTTPS CONNECT proxies with NO_PROXY
selection. Proxy CONNECT carries no Git Authorization. Proxy credentials, if
configured, remain endpoint-local and are scoped to Proxy-Authorization only;
origin gh credentials never satisfy proxy auth. No implicit core-side proxy.
Unsupported proxy schemes/auth methods or trust bypass settings fail before
origin credential access; they are not silently ignored. Exact config precedence
and additional native parity belong to the deferred qualification batch before
advertising support. No blanket native parity is claimed by this design.

## 6. Pooling, bounded work and lifecycle

Use one endpoint-owned gwz-transport pool for HTTPS; retain its 60-second idle
default and existing per-host/total ceilings. Key is HTTPS host/port, no username;
TLS identity is not an authenticated GitHub account. Every request applies its
own gh/anonymous policy. With immutable endpoint trust/proxy policy, one physical
connection record owns one TCP/TLS tunnel, Hyper sender and driven connection task.
No HTTP library internal pool, concurrent lease or HTTP/2 coalescing.

Pool Connect acknowledges only after DNS/TCP/proxy/TLS and HTTP setup succeed;
CancelConnect/Abort/Close acknowledge actual completion/disposal. Idle peer close
invalidates the cached connection. Sender readiness is a hint, not proof of live
peer; any race to closed connection yields a typed failure. No transparent retry
is required, even for GET. Connection health is reusable only after request body
completion, validated response framing/EOF, no terminal failure and ready sender;
otherwise discard. An authenticated request can reuse an anonymous TLS connection,
but cannot inherit credentials or authenticated=true from it.

RPC close can drain a bounded residual response within cleanup budget to prove
reuse; discard on budget/cap exhaustion. Never report Git success merely because
HTTP EOF or cleanup succeeded. Any incomplete/truncated/aborted exchange discards
its connection. Cancellation wakes Git immediately, stops HTTP body polling,
drops the request future and closes its connection, then reports bounded cleanup.
All task handles remain owned until joined or explicitly reported pending.
Drop initiates cancellation without blocking or declaring successful retirement.

Initial construction defaults (within existing hard caps): 8 simultaneous
connector/helper jobs each, 64 admitted endpoint requests, 16 pending input
messages/request and existing negotiated stream windows/payload caps. HTTP header
buffer 64KiB/100 headers, request/response bridge chunks <=16KiB each, with at
most one pending chunk/direction per exchange. Include Hyper/TLS buffers and job
results in memory accounting. Bound DNS result lists to16 candidates and attempt
them sequentially under one connect budget. Apply existing allocation/connect/
interaction/network-idle/cleanup budgets; backpressure never replenishes them.
When backpressure intentionally suspends network-idle timing, retain its remaining
allowance and continue allocation/cleanup/helper deadlines. Redirects and auth
retry use remaining budgets, never fresh per-attempt allowances.

CleanupReport pending work counts cover HTTP connections/tasks, gh children and
noninterruptible jobs. Integrate these into request finish, endpoint shutdown and
runtime shutdown; do not just drop their reports. Close the accepted Placement C
State P3-1 during H2 by testing a retained job and eventual retirement explicitly.

## 7. Failures and observations

Opened carries connection identity/reuse and initial facts; it does not assert
HTTP authentication before a response. `credential_offered` becomes true only
when Authorization is handed to the HTTP send path. Final facts accompany Closed
or terminal Failure using the existing protocol. Keep authenticated unknown on
ordinary 2xx: a public resource's acceptance alone does not prove an account.
401 after offering credentials is false; 403/404 may hide repository authorization
and do not prove bad credentials. TLS reuse remains distinct from request auth.

Map invalid destinations to InvalidRequest, unavailable scheme/config to
UnsupportedOperation, TLS rejection to Trust, helper rejection/missing login/401
to Authentication, 403/404 to RepositoryRefused, timeout/cancel to their existing
codes, malformed HTTP or unexpected success body type to Protocol, and network
loss to Io. Retain sanitized status and service, never raw URLs, Location, body,
helper stderr, arbitrary headers or secrets. A 404 anonymous discovery may be
internally retried once under §4; only its final operation result is published.

For receive-pack POST, mark Effect::Possible before the first request byte is
handed to the network client, and preserve it through timeout/cancel/redirect or
late errors. Early local validation/helper failure remains None. Upload-pack and
discovery have no remote publication effect; local repository effects continue
to follow the operation's existing accounting. Never retry a push on ambiguous
outcome. Map terminal repository refusal into existing private-member suppression
without hiding trust, malformed input, transport loss or ambiguous publication.

## 8. Integration and implementation gates

Private modules: `https_destination` (URL/redirect route), `https_auth` (gh job),
`https_connection` (HTTP/TLS/proxy owner), `https_pool` (actions/disposal),
`https_worker` (body/credit bridge), `https_remote` (RPC stream), with tests by
cohesion. Reuse `stream_io::BlockingStream`, not SSH-specific error strings or its
stateful remote wrapper. Extend private host session/placement dispatch by Scheme;
share request registration, binding, cancellation and terminal routing. Each
endpoint advertises only actually enabled schemes/policies. SSH-only peers stay
valid; explicit unsupported HTTPS refuses before effects. Generalizing the public
SshEndpointConfig/constructors is a later Surface-reviewed activation change;
H1/H2 use private candidate injection and freeze no new public settings.

Two substantial implementation batches, with TDD then one aggregate Code/State
gate each. No review per helper or command:

1. **H1 complete endpoint/RPC candidate:** pinned client; policy/gh jobs; URL,
   proxy/TLS and redirect validation; exclusive pool; streaming RPC adapter;
   fake-gh and local TLS smart-Git fixture. Establish all state transitions and
   deterministic seeded chunk/read/cancellation tests, no real user credentials.
2. **H2 host and command integration:** local + in-process carried placement in
   existing Rust/Python messages; every N3 network funnel (clone/init, fetch/pull,
   advertisements/tags, manifest/ref reads, push and post-push reads); observations,
   capability mismatches, local HTTP/git compatibility and private-member cases;
   assert cleanup reports and close C State P3-1. Aggregate retained review.

Required H1 tests: multi-round fetch, large clone/push, partial reads/writes,
zero-length reads, flush/timer not EOF, early401 during blocked upload, truncated
response, never replayed POST, 256KiB+ random payload reassembly, bounded queues,
paused credit, helper floods/hang/cancel, absent gh/public anonymous success,
disabled helpers, no-login/auth rejection, token changes between RPCs, Enterprise
host/port, sentinel redaction, different-origin redirect isolation and redirected
GET->POST base continuity, five-hop limit, idle reap, idle-peer-close race,
capacity while cancelled jobs linger, clean disposal versus pending reports.
Raw evidence only in private evidence member; public tests are self-contained.

Platform and selected-source checks stay one operator-deferred batch. System
TLS/proxy parity, actual supported gh versions/host behavior, final dependency
selection, production constructors/capabilities, release and performance claims
remain gated. GitHub network/manual-account checks use explicit test credentials
later; unit tests must not consume the user's login. No physical carrier or iroh
qualification is part of H1/H2. An adapter/library incompatibility cannot be
papered over by falling back to native HTTPS or weakening the frozen protocol.

## 9. Source basis and limits

Checked 2026-09-22; moving upstream sources inform choices, not qualification:

- [Hyper per-connection API](https://docs.rs/hyper/latest/hyper/client/conn/http1/):
  separate sender and driven connection. [Sender](https://docs.rs/hyper/latest/hyper/client/conn/http1/struct.SendRequest.html)
  documents cancellation closing HTTP/1 and readiness races.
- [Hyper buffer controls](https://docs.rs/hyper/latest/hyper/client/conn/http1/struct.Builder.html)
  must be pinned/configured rather than relying on changing defaults.
- [native-tls builder](https://docs.rs/native-tls/latest/native_tls/struct.TlsConnectorBuilder.html)
  provides trust/verification configuration; this does not prove platform parity.
- [gh credential helper source](https://github.com/cli/cli/blob/trunk/pkg/cmd/auth/gitcredential/helper.go)
  implements get and active host/account/token selection.
  [gh environment](https://cli.github.com/manual/gh_help_environment) documents
  environment-token behavior. Qualification records the actual gh version.
- [Git HTTP specification](https://git-scm.com/docs/http-protocol) establishes
  service discovery and RPC body semantics; the design above selects an explicit
  supported policy, with libgit2 retaining Git protocol interpretation.

Local basis: `gwz-transport/src/{protocol.rs,policy.rs,pool/}`, core
`src/git/endpoint/{ssh_remote.rs,stream_io.rs,placement_endpoint.rs}`,
`src/transport_host/{mod.rs,session.rs,request.rs}`, and the pinned libgit2
`src/libgit2/transports/smart.c`. No implementation or passing-test claim is made
by this document-only gate.
