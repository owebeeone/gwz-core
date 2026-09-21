# Shared SSH endpoint worker and destination routing

Date: 2026-09-21. Status: implemented; aggregate review pending; production activation pending.
Authority: operator instruction to proceed with endpoint wiring after the accepted
[SshIntegration](GwzRemoteTransportSshIntegration.md) checkpoint. Platform and
selected-source qualification remain deferred as one integrated batch.

## Boundary and sequence

The next boundary replaces the fixture-only worker with an internal reusable
worker owned by a core endpoint. Endpoint clones share one worker and pool;
operation handles do not own the pool. Worker shutdown cancels requests, retains
physical capacity through disposal and wakes blocked Git calls. A failed exchange
must not kill unrelated active exchanges. Queue and per-turn work are bounded;
monotonic timers run independently of the blocking Git caller.

Destination admission handles SCP and ssh/git+ssh/ssh+git URLs, preserves the
repository operand, decodes URL escapes once, keeps IPv6 and effective ports,
and rejects credentials embedded in URLs before endpoint effects. Repository
path and remote name are not pool-key components. No URL spelling can silently
fall back after selecting this endpoint. This internal resolver is not a wire
protocol or new CLI surface.

The worker accepts the existing nonblocking Connector/Resource boundary. Setup
must establish host trust before authentication, then transfer exactly one
owned SshConnection. A channel-capable Resource owns the pump until cleanup and
restores the connection only after Closed is emitted. Native session clones must
not escape. Per-request explicit identity eligibility is checked before checkout.

Production activation additionally requires bounded DNS/connect/authentication,
existing identity selection and observations, and all network-entry routing.
Inspection found libssh2's Unix agent opens a blocking local socket: ssh2's
nonblocking session alone does not establish bounded helper cancellation. Do not
move that call into the shared worker or label it nonblocking. Setup cancellation
must be qualified before activation; any required binding change needs its own
bounded scope. The worker and destination tests can proceed independently.

## Package and gates

Internal worker plus channel-resource seam: <=650 production lines across <=3
files. Destination resolver and per-remote binding: <=250 lines across <=2 files. Added focused tests/support <=800 lines.
No production manifest/source-pin switch or endpoint capability advertisement
until the connection-setup boundary is qualified. This deliberately excludes
native credential implementation and production network-entry edits from the
first worker checkpoint; they remain required next steps of endpoint wiring.
Reuse the accepted pool/pump/per-remote bridge and frozen transport API.

Tests first: invalid destination refusal, aliases/escaping/path/port cases;
shared endpoint lifetime, bounded admission, cancellation/shutdown, fault
isolation and real native Git composition through the new worker. Existing
channel/pump/pool regressions remain gates. Internal aggregate checkpoint uses
retained Code/State reviewers and a settled tuple; no new public surface. P0–P2
block acceptance; at most two remediation rounds. No platform/source batch here.

## Connection-setup finding and follow-on route

The public `agent_wait` test uses an isolated Unix socket and reaped child test
process; it sends the five-byte identity-list request and withholds a reply.
`Session::set_blocking(false)` plus `set_timeout(25)` still leaves the child
waiting at 200 ms. This is a characterization of the existing native API,
not a passing bounded-authentication implementation. No user agent/keys are used.

A candidate remedy is libssh2's public-key signing callback: pinned
`libssh2/src/userauth.c` explicitly retains authentication state when the signer
returns `LIBSSH2_ERROR_EAGAIN`. ssh2 exposes guarded raw-session access but not
that high-level signing API. A separately reviewed minimal binding plus a bounded
endpoint-local agent client could therefore preserve native session ownership
and cancellation without changing libgit2 or running a Git subprocess. This is
an investigation result, not a qualified implementation. Signature allocation,
algorithm negotiation, helper cancellation and platform handles require tests.

## Production call-site map (activation still pending)

All remote Git work funnels through `git/gitbackend/transport_support.rs`:
`remote_callbacks`, `fetch_options_with_progress`, `remote_fetch_options`, and
`remote_push_options`. The direct advertisement path is `connect_auth` in
`git/gitbackend/transport.rs`; `read_remote_file` also uses fetch options.

| Required entry | Existing funnel / driver |
|---|---|
| Bootstrap / init-from-sources | `clone_repo_with_progress`, `handle_init_from_sources` |
| Workspace/member clone | `clone_repo_named`, `clone_workspace`, `handle_repo_lifecycle` |
| Materialize and snapshot materialize | `handle_materialize/apply.rs` clone funnel |
| Fetch / pull preparation | `backend.fetch`, `handle_fetch`, pull-head preflight |
| Tags | `handle_tag` fetch/push/advertisement funnels |
| Push and publication verification | `push_prepared`/`push`, `publication.rs` advertisement reads |
| Manifest discovery | `read_remote_file` before workspace clone |
| Local-family file operations | `fetch_anonymous`/`push_anonymous`, keep credential-free native routing |

Store the shared endpoint on the backend and preserve it across backend clones
and `with_transport`; do not create one per member. `handle_pull_snapshot` can
scope a backend then call `handle_materialize`, which scopes it again. Keep
per-invocation observations distinct from pool ownership and preserve the
operation's observation context through nested scopes. Reuse must report
`credential_offered = false` for that attempt and proven authentication rather
than attributing the cached session to a new credential offer. Activation must
exercise each driver row; a callback inventory alone is not coverage evidence.

## Local evidence and implementation checkpoint

The focused Rust 1.95 macOS fixture passed with 35 tests; its one ignored child
entry is invoked by the passing fake-agent parent. The new worker composes push,
clone, a second push and fetch using one injected authenticated connection.
Native service failure leaves another active stream readable; endpoint clones
retain the worker; bounded admission and shutdown with disabled connect timeouts
are exercised. Destination and eligibility regressions refuse malformed inputs
before endpoint effects and re-resolve identity authority on every open.

The scheduler uses a single monotonic origin and a real unpark waker, bounded
request/message turns and a 1 ms active polling fallback; idle turns sleep until
the next pool deadline (at most one second). This does not claim readiness-driven
socket scheduling. Admission permits cover queued/pending work and are released
before publishing replies. Physical capacity remains owned by the pool through
cleanup. Pump time advances before ingress so a late Close cannot hide expiry.

Production additions: 664 lines across three new internal files plus five added
pump lines. Tests: 842 lines across four new files plus the pump regression
(under the original test ceiling's 20% wiggle). No new public surface, wire
change or production dependency activation. Native setup remains excluded from
this checkpoint and required for the next one, as declared above.

Development corrections included constructor-time pool validation, shared clock
origin, disabled-connect timeout semantics, admission ownership, nonblocking
shutdown signalling and failure cleanup. Two new test assumptions were corrected:
terminal late frames are ignored, and stdout EOF can precede service failure
reported at close. These were test expectations, not product regressions.
No known escaped defect; formal review results are recorded after settlement.

Raw passing/failing logs and exact final source hashes are retained in
[worker-a](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-worker-a/README.md)
(private member access required). Platform/source checks remain batched later.
