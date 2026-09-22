# HTTPS H1 — endpoint and RPC candidate

Status: **correction 2 implemented; final retained re-verdict pending**.
Controlling contract: [accepted HTTPS design](GwzRemoteTransportHttpsDesign.md).
No production activation, public constructor freeze, release or wire carrier.

## Scope and composition

The candidate lives under `src/git/endpoint/https_*` and the existing
`gwz_transport_candidate` boundary. The external full-core preparation script
pins Hyper 1.11.1, Tokio 1.53.1, hyper-util 0.1.20, http-body-util 0.1.5,
bytes 1.11.1, native-tls 0.2.18, tokio-native-tls 0.3.1 and tokio-util 0.7.19.
Production Cargo inputs are unchanged.

`https_remote` implements per-remote HTTP RPC subtransport behavior for all four
Git services. Each action opens a fresh logical exchange. The first nonempty
read completes the request body; flush and empty reads do not. Unfinished RPC
Drop cancels. `https_local` composes the candidate with independently driven
in-memory envelopes and timers; it is private test/candidate injection, not a
new CLI/core API or physical GWZ carrier.

`https_worker` validates/adopts an operation route, performs discovery before
Opened and begins POST after Opened. It enforces explicit discovery redirects,
one anonymous-to-Gh discovery transition, pre-POST receive-pack Gh selection,
write-once route pinning, typed status/refusal behavior and no POST replay.
Allocation, connector, helper and pre-open network allowances carry across
redirects/auth transitions. Pool connection identity reflects the final
advertisement connection. POST effects become possible when the HTTP request
is handed to the sender. HTTP progress comes from the actual TLS stream, not
from moving bytes between message queues.

`https_auth` invokes an explicitly supplied endpoint executable as
`gh auth git-credential get`, using only its supplied environment snapshot.
Pipes, workers and pending cleanup are bounded. It kills/reaps cancelled work,
retains unreaped children and their admission permits, and clears owned parsed
credential buffers without claiming erasure of library copies. Credentials and
helper stderr are absent from protocol facts. Successful HTTP alone does not
establish an authenticated account.

`https_connection` owns TCP, TLS, HTTP CONNECT/HTTPS CONNECT, trust, the driven
HTTP/1 connection and physical disposal. `https_pool` uses the existing generic
pool/PoolHost. `shared_reservation` provides the physical admission wrapper for
one shared total/per-host authority across scheme-specific owners: SSH port22
and HTTPS port443 count against the same canonical host. HTTPS uses that wrapper
now. H2 must inject the same authority into the composed SSH+HTTPS endpoint;
constructing independent authorities is not aggregate capacity enforcement.
A premature wrapper Drop fails closed by retaining its reservation; only actual
disposal releases it. Native job/driver cleanup remains separately owned.

The owner protocol has no new fields or tags. The generic codec permits a reused
credential offer only with Gh method; mux route context additionally requires
the bound Open to be HTTPS/Gh. SSH and Anonymous cannot widen their authority by
claiming Gh. The initiator advertises existing HTTPS capabilities; each endpoint's
negotiated capabilities still determine availability. Pre-Opened discovery
failures use v2 OpenFailed with retained facts, not stream Failed.

## Validation and replay

Initial local gates: **45 endpoint tests passed** (37 HTTPS cases plus existing
endpoint/shared-reservation cases); the complete gwz-transport suite and default
core library check passed. Scoped formatting and evidence archive verification
also pass. This is not a whole-core or platform qualification claim.

Public self-contained tests are included by `https_worker_tests.rs`, with local
TLS fixtures and fake gh executables. Certificate/private-key material is created
in disposable test directories. Git subprocesses implement the fixture's remote
upload-pack/receive-pack server only; the client under test is native git2.
No real account or external GitHub service is used.

Covered: partial writes, zero-length reads/flush semantics; native large clone,
multiple fetch negotiation POSTs and large push; seeded 300KB reassembly with
random writes/reads; seeded cancellation with paused response credit; anonymous
public access with absent gh; disabled helpers; one challenge transition; Gh
rejection without retry; new credentials on reused TLS; cross-origin credential
isolation and exact host/port/path lookup; final redirected connection attribution
and GET-to-POST continuity; conflicting route pinning; status/refusal matrix;
redirect query grammar and five-hop cap; TLS trust failure; HTTP/HTTPS CONNECT,
proxy isolation and NO_PROXY; truncated response; informational-response bound;
early401 during upload; stalled POST after EndWrite; idle reap/peer-close race;
shared physical reservations through pending disposal and failed connection.

Replay from the workspace root with an external manifest/target:

```sh
python3 gwz-core/tests/transport_backend/prepare.py /tmp/gwz-h1-replay
RUSTFLAGS='--cfg gwz_transport_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-h1-replay/Cargo.toml \
  --target-dir /tmp/gwz-h1-replay-target --lib git::endpoint:: -- --test-threads=1
```

The first preparation copies the production lock; resolve the candidate's pinned
additions before using `--locked --offline`. The settled evidence retains the
resolved candidate manifest/lock. Set `GWZ_HTTPS_SEED=<u64>` to replay the payload
case; the cancellation test prints/asserts its fixed seed and iteration.

[Private raw evidence](../../gwz-core-evidence/campaigns/https-integration/runs/2026-09-22-h1/README.md)
retains passing, failing and compilation attempts separately. Early failed
attempts do not all have per-attempt source hashes. RPC, policy and worker
boundaries began with captured failing tests; connector/pool drafting preceded
some causal fixtures, a TDD process deviation. Do not describe this as universal
red-before-code coverage. Implementation-phase failures found certificate fixture
setup, premature endpoint stream Drop, missing pool identity proof, helper stdin
EOF and an early-rejection fixture ownership error; regressions are retained.

## Remaining gates

Aggregate retained Code/State review of the committed tuple precedes acceptance.
H2 owns existing host/all-command Rust/Python message embedding, observation and
private-member integration, one injected shared SSH+HTTPS authority, and the
Placement C cleanup-accounting P3. H1 introduces no production endpoint capability.

Platform and selected-source checks remain the operator-deferred single batch.
Actual gh versions/accounts/Enterprise behavior, environment precedence, platform
TLS/proxy parity, final dependency selection, production construction/activation,
physical wire/iroh, release and performance measurement remain separate gates.

## Aggregate correction 1

Initial [Code](../../dev-docs/GwzRemoteTransportHttpsH1-ReviewCode.md) and
[State](../../dev-docs/GwzRemoteTransportHttpsH1-ReviewState.md) reviews reported
NO-GO: six distinct P2 roots and two bounded P3 findings. Both axes independently
found operation route retirement attached to per-remote Drop. The
[consolidated correction plan](../../dev-docs/GwzRemoteTransportHttpsH1-RemPlan.md)
maps every finding and closure test. Findings await the raising reviewers.

Routes now have explicit whole-operation sealing and dependent guards. The
operation owner calls `finish_operation`; individual remote Drop only cancels
its work. Sealing prevents new preparations, and routes retire only after
remote and prepared-stream dependencies drain. Endpoint shutdown/Drop closes
admission and cancels owned helpers and pool work. Shutdown counts held
preparations and retained child/physical work; repeated shutdown can converge.
Aborting a lookup retains its child and admission permit until actual reap.

Allocation wait, connection setup, helper interaction, active I/O and cleanup
have separate budgets, carried over redirects and the allowed authentication
transition. Candidate construction captures an independent active-I/O setting
(default3000ms, zero disabled); Open can shorten it. The generic pool's existing
wait-only request deadline is used, with current-clock admission before dispatch.
No public pool configuration field was added.

`https_opening` admits actual Open messages and routes actual worker outcomes
through an in-process mux pair. Each local RPC uses a unique session; anonymous
failure crosses as OpenFailed before a distinct Gh Open under the same remaining
budgets. Successful Stream instances use the admitted session/stream identity,
and their subsequent messages pass through the same mux. This is private H1
composition; H2 still embeds into the existing host/session owner.

HTTP parse failures map to Protocol, transport loss to Io. Credential-offered
is cumulative across discovery redirects, while status/authentication describe
the current origin; pre-response failures retain unknown status/authentication.
No secret, raw URL or helper stderr is included in these receipts.

Corrected local gate: **64 endpoint tests passed**, default core check passed,
scoped formatting and token-aware conditional-boundary checks passed (including
disabled branches). Transport sources are unchanged since the initial passing
full-suite gate. Actual local Git clone, multi-round fetch and push now traverse
the mux relay as well as the HTTP worker. No platform or source-admission claim.

[Correction evidence (private access)](../../gwz-core-evidence/campaigns/https-integration/runs/2026-09-22-h1-rem1/README.md)
retains regression failures, build attempts and final gates. Route/shutdown
counterexamples were captured failing before their corrections. Auth/budget/mux
helpers were partly drafted before causal tests; universal TDD adherence is not
claimed. Intermediate attempts do not all have source fingerprints. Twelve
additional seeded replays belong to the unchanged initial H1 tuple, not the
corrected candidate. Retained reviewers judge closure on the corrected tuple.

## Child-reaper correction 2

Correction1 received [Code GO](../../dev-docs/GwzRemoteTransportHttpsH1-ReviewCode-1.md)
and [State GO](../../dev-docs/GwzRemoteTransportHttpsH1-ReviewState-1.md).
Before recording acceptance, owner audit found that cancellation during reaping
itself could lose child/permit ownership, and a reaper could omit concurrent
arrivals from its return count. Both new regressions reproduced the defect.
The [second correction](../../dev-docs/GwzRemoteTransportHttpsH1-RemPlan-2.md)
retains guarded batches through cancelled awaits, counts in-flight reaping and
queued arrivals, and orders the final shutdown snapshots so a preparation's
transfer to retained cleanup cannot disappear. The orphan fallback preserves
its original ownership tags through the same cancellation case.

Final endpoint gate: **66 passed**; scoped formatting and conditional-boundary
checks pass. [Private red/green evidence](../../gwz-core-evidence/campaigns/https-integration/runs/2026-09-22-h1-rem2/README.md)
includes exact red/final source fingerprints. This narrow case escaped the
correction1 review; H1 remains nonactivated and no released escape is claimed.
The original findings remain closed by their reviewers; correction2 acceptance
awaits their focused final verdicts. All H2 and qualification deferrals remain.
