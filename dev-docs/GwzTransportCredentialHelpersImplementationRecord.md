# Credential helper/context/code75/password implementation receipt — in progress

2026-10-03. Working source package, not implementation acceptance. Accepted
context authority is core `aedfa862d7ce6c121ecbe4d65ac4900e2017fdde`;
configuration-view adoption is core `edcab346745b896c14112bc8efdace87d91c50f1`.
The v8 parser environment correction is preserved at core `646e3e1c`/evidence
`662d8982` and remains subject to full implementation review. The actual
hasconfig regression is GREEN. Root owns settlement and reviews; the drafter
has performed no Git/GWZ mutation.

Both original round2 reviewers returned GO with no open clock findings at
core `fde5878a`. Root adopted the mechanism at core
`a53e1f1004a157d0bdc3adb2a038fcf52cb25ddc` and explicitly relayed implementation
authority. The working package now implements the shared clock and password-only
helper route. Final source settlement and fresh implementation reviews remain
open; mechanism acceptance does not constitute source acceptance.

## Behavior and preserved boundaries

The two accepted optional internal fields carry the captured helper allowance
and encoded HTTPS account selector. Timeout producers capture integer milliseconds
once; zero/submillisecond allocation refuses before helper admission and launch.
Both endpoint and host admissions share one absolute allocation deadline; both
permits remain owned by pending helper cleanup. Started helper interaction is
separate, positive and capped at the accepted **120 seconds**. Public code **75**
is the error enum value, not a 75-second duration; the initial brief's contrary
reading is corrected here.

All Failure carriers validate the timing provenance/limits, fixed cause and
bounded diagnostic alternatives. M4/M8/M10 render the complete accepted catalog
text, including exact fractional seconds, causal-neutral zero-allocation wording,
and native Git's repository/conditional helper-scope caveat. General timeouts
without helper provenance retain the general renderer. A typed helper Timeout
crosses the HTTPS/libgit2 seam as Timeout+Http and the fixed M4/M10 stem; the
central model conversion assigns `credential_helper_timeout` (75). Other
libgit2 codes/classes/messages retain `git_command_failed`. This has focused
boundary tests; it does not claim complete native callback integration proof.

HTTPS Open carries a separate encoded username, leaves SSH username meaning
unchanged, validates decoded controls before Open/endpoint effects, forwards it
to the helper URL, removes HTTP userinfo and redacts Destination Debug. Retry
budget/challenge identity includes this private selector, keeping accounts apart.
The fixed MalformedOutput cause covers malformed/duplicate recognized fields;
unknown helper lines remain tolerated. Failure/log messages contain no username,
password, helper answer or free-form helper diagnostic.

The runner uses supervised `git -c core.askPass= credential fill`, URL-only
input, captured environment, process-group kill/reap and the adopted disk-free
configuration view. It preserves ordered unconditional includes and repeated
scope occurrences while excluding every includeIf. Stdin parsing runs with an
empty controlled configuration; the initial native discovery can preread an
original FIFO and remains deadline/output bounded, not source-size bounded.
Owned answer/header/config buffers wipe capacity. Pending children/file workers
retain both quotas. The two distinct FIFO regressions preserve native timeout
and controlled-parse/core-file refusal semantics; the six view tests passed.

A route owns its answer once per canonical credential URL. Its opaque pool
scope retires authenticated physical connections when the answer owner dies;
another route/account/operation cannot reuse that identity. Discovery for both
services starts anonymously; only a supported 401 challenge admits helper work.
Anonymous receive-pack discovery followed by a POST challenge does not run a
helper. Redirect targets start anonymously and obtain their own answer. A
rejected answer is not resent; missing/unstartable Git latches only within the
operation, while a later operation can look it up afresh. Fixed scheme tokens
never carry a realm or challenge payload into diagnostics.

The stream retains the first ADMITTED terminal Failure by move, including its
typed detail/facts. Admission failure and ignored/late terminals do not allocate
or replace a retained copy. The RPC adapter and private TransportAttempt bridge
project typed Open/terminal outcomes without interpreting generic error text.
Fetch/push narrowly preserve external_tool_missing and credential_helper_timeout;
other handler behavior is unchanged.

| Obligation | Current implementation/gate state |
|---|---|
| TR2.2 configured-gh absolute/PATH discovery and URL-only helper input | Implemented; broader candidate HTTPS suite 161/0 |
| TR2.22 captured context/username, strict recognized fields, fixed causes/code75 | Implemented; focused producer/validator/renderer/generator checks green |
| TR2.22 supervised cleanup, secret buffers, both admissions, native scope view | Implemented; six view tests and broader HTTPS suite green |
| TR2.22 route-owned reuse, account/operation isolation, missing-Git latch, challenge-only receive-pack | Implemented; corresponding HTTPS regressions included in 161/0 |
| TR2.22 terminal detail ownership and adapter classification | Implemented; retained-failure focused regression green, transport full suite green |
| TR2.22 real fetch/push/private-clone M1/75 and E2BIG/M2 projection | GREEN actual operations: M1/75 remain visible; native E2BIG M2 projects fetch/push remote_rejected, private clone quiet; same-operation repaired route succeeds |
| TR2.23 password-only helper parity and cancellation/clock provenance | Native endpoint and public typed consumer implemented; real SSH M1/75/M4/M10 backend and handler rows GREEN (final focused/affected gates below) |
| Normal final gates and secret Code/State + combined Surface/aggregate acceptance | Normal gate GREEN; secret/Surface/aggregate acceptance OPEN; no final implementation GO |

## Generator and split provenance

One connection-scoped, transport-neutral SetupClock arbitrates network and local
phases. Prepared tokens do not pause clocks; publication, acknowledgment, expiry,
cancellation and completion share its mutex and immutable first terminal cause.
Network aggregate and stall remainder pause during helper admission/interaction
and rebase on resume; progress alone resets stall. Control, PoolHost and
post-result classification consume that same authority. Disabled network clocks
and inactive stall remain distinct. Valid expired preparation terminates its own
authority; stale/foreign tokens cannot replace another connection's terminal.
Core owns bounded phase witnesses and exact helper timing/error projection.
Pending physical cleanup retains the job, phase context, pool charge and both
helper permits until children/file workers retire. Wake/drop callbacks run outside
the authority and runtime locks; a safe custom-Wake regression checks ownership.

Ambient SSH uses helpers only when the server offers password without publickey
and helpers are enabled. Trust still precedes authentication; selected keys,
agents, combined/publickey offers and explicit URL passwords preserve their
existing selection. The private opening path carries captured configuration,
shared host slots and retained allocation. Each started helper receives its full
positive effective interaction allowance after both admissions, independently of
earlier connect/network deadlines. A 350 ms native helper succeeds past an
original 250 ms connect deadline under its 1,500 ms helper allowance. No public
wire/schema field, global state, dependency binding or transport secret owner is
introduced by this clock/password integration.

Core's normal protocol generator and message-catalog generator produced code75.
The catalog also repairs its pre-existing omissions of codes73/74 and Sync planned;
no additional schema change is authored. Python has generated api/IR and its
normal exact-additive drift guard only; no handwritten driver/settings or CLI
source changed. Both core and Python remove precisely enum75 for historical
projection checks. The historical core projection hash remains
`4d377a496c8905293b5e9b53392b70867cf6dafccbb623841a623dbd2d555f14`.

Candidate generator metadata now separately pins current-core-schema-sha256.
Retained-old-schema `423cb73b8c17a6779155431aebe40949f90a429ef69625683b449ed7fe072adb`
and retained Rust source/revision/hash remain byte-for-byte; the current owner IR
pin changes only for accepted context fields/cause. Current candidate artifacts
are regenerated, not a rewritten historical corpus/reader.

The split-files skill and installed rust-split were used for declaration moves.
Explode reconstructed auth/model/request/HTTPS endpoint sources byte-identically
in manifest order. Auth separates ownership, executable resolution, lookup,
secret parsing and tests. Model error declarations keep public type identity
through re-export; HTTPS failure rendering is cohesive. Endpoint polling moves
step/take_outbound into a child impl, preserving the original transport_host
visibility and changing one session sibling qualifier. rust-split explicitly
reported its 475-line impl requires manual extraction; that bounded extraction
was reviewed rather than applying its oversized 511-line proposal. Additional syntax-aware splits separate host policy fixtures by admission,
cancellation, receipts and route invariants, split authentication fixtures into
challenge/route/policy leaves, and move stream state declarations with unchanged
re-export type identity. Final split verification includes 161 HTTPS tests and
the full transport suite. All new leaves are below500 lines.

Pre-existing oversized handle_fetch.rs, push_member.rs and gitbackend/transport.rs
receive only the authorized narrow projection changes. Root explicitly deferred
moving these whole owners until the post-integration refactor checkpoint to
preserve the active feature's scope. This is recorded size debt, not a completed
migration; generated/schema source remains under its existing exception.
Other pre-existing large owners retain narrow constructor/admission edits:
core https_tests.rs623 unchanged lines; transport admission.rs501→513,
tests/mux.rs896→897 and tests/sequenced.rs549→550. New cohesive leaves remain
below500; this records remaining size debt without claiming a broader migration.

Local syntax-aware move receipts are in
`/Volumes/projects/limbo/gwz-tr222-cohesive-splits-20261003/receipt.json` and
`https-endpoint-move-receipt.json`, with frozen source/chunks/proposals; auth's
original receipt is `/Volumes/projects/limbo/gwz-tr222-helper-split-20261003/`.
The latest move receipt is
`/Volumes/projects/limbo/gwz-tr222-final-splits-20261003/receipt.json`;
its three exploded sources reconstruct byte-identically, with reviewed module
wiring and conditional scopes. These are local work receipts, not newly archived private campaign runs. The
configuration campaign contains only its separately authorized mechanism evidence.
Clock integration moves use
`/Volumes/projects/limbo/gwz-tr222-clock-splits-20261003/receipt.json`:
agent Control/tests, SSH setup tests and worker request/endpoint/runner declarations
were extracted with their owning attributes/scopes. Re-exported type identity is
unchanged. NEXT_WORKER remains the existing recorded debt; its allowlist path
follows the endpoint declaration move, with unchanged reason/disposition.

## Validation

Rust1.95.0, retained ordinary profiles/debug/incremental. Normal core
`scripts/run_tests.py --lib --offline` passes its full source guards and targeted
fake/native phases; final native phase: 2,103 passed, one ignored. Transport full
unit/integration tests and doctest pass, strict library Clippy passes, and four
normal transport artifacts regenerate exactly. Core normal regen --check and
historical additive proof pass; candidate regen checks and nine provenance tests
pass. Python normal regen --check and drift guard pass against this lane's exact
schema, without exercising its copied editable runtime against main.

Focused RED preceded renderer/code75 implementation and helper timing producer
implementation. Their corrected focused cases pass. The completed broader candidate HTTPS run is
`/Volumes/projects/limbo/gwz-tr222-route-https-v6.log`: 161 passed, zero failed,
exit0 in 13.78s under Rust1.95.0, own-lane candidate manifest and
`target/candidate-transport`. Transport full suite/doc tests exit0 at
`/Volumes/projects/limbo/gwz-tr222-transport-final.log`; first admitted Failure
regressions pass at `gwz-tr222-retained-failure-test-v2.log`. No repeated green
run is required absent relevant changes. Final full source acceptance is open.
The initial broad run was 134 green/14 failures, including the preserved scope
counterexample, obsolete gh argv/input fixtures and one scheduling-dependent
host-slot fixture. Updated fixtures now assert Git argv/URL input and retain
actual credential comparisons; a host-slot fixture holds until cancellation
rather than relying on a five-second process exit during a loaded parallel run.
Fixture corrections cite TR1.6 §2.1/§5/§6/§11: both discovery services start
anonymous; each route asks once; fresh operations perform fresh lookup and do
not inherit another operation's pool scope; a supported challenge with missing
Git is M1. Assertions retain actual credential comparisons, anonymous/authenticated
request sequences, physical scope isolation and clean cancellation.

The latest normal gate (`gwz-tr222-core-normal-final-v2.log`) exits0 with
2,103 native tests, one ignored and all source/fake/native phases. The real
operation test (`gwz-tr222-helper-projection-v4.log`) exits0 in 3.48s after a
RED exposed advertised_refs connect_auth/list dropping typed M2 classification;
both branches now use owned attempt.error with their existing git_error fallback.
Its assertions cover M1/75 unsuppressed private materialization, M2/E2BIG quiet
private clone, no helper execution and repaired different route in the SAME
operation. Current candidate Clippy (`gwz-tr222-candidate-clippy-final-v2.log`) exits0
with51 warnings: inherited lint debt is not
claimed strict-green. One newly unused production checkout wrapper was restricted
to its actual test owners; no blanket warning suppression was added.

The new normal gate initially failed only its candidate-switch inventory for
TransportAttempt::failed; that declaration was added to the exact inventory and
the gate restarted. No failing test is hidden or claimed as an accepted exception.
Generator checks retain exact historical projection pins. A bare system-Python
additive invocation lacked taut, and an initial candidate check incorrectly
supplied the taut source instead of owner IR; corrected pinned-interpreter/IR
checks are the authoritative commands, not those invocation errors.

Clock/password TDD logs preserve missing-API REDs, the consumed-Job Drop
counterexample and its fix, and the interaction fixture's initial unsupported
assertion that credential fill must start within a 75 ms total interaction
allowance. The final fixture keeps exact timeout timing/cause and no password
offer; native cancellation separately waits for the real fill PID before
cancelling and proves descendant retirement and restored slots. No timer was
extended to make that fixture pass. Final SSH log
`gwz-tr222-ssh-final-v9.log` has 194 passes, zero failures and three existing
ignored tests; moved agent-job tests pass 10/0/1 ignored in
`gwz-tr222-agent-job-final-v9.log`.
Six native password rows also pass after final lint cleanup in
`gwz-tr222-password-final-v11.log`. Candidate Clippy v12 passes with50 retained
warnings; the seven newly introduced integration warnings are removed, alongside
one older redundant closure. This remains a debt-bearing core gate, not strict
Clippy acceptance.

The final public SSH closure is explicitly authorized by the coordinator under
adopted typed retry/error ownership. SshOpenFailure projects only admitted
helper timing detail or Gh Unavailable facts into the same first-error
TransportAttempt bridge; generic SSH timeouts/refusals retain their fallback.
A host-private formatter shares the complete accepted M4/M10 body with HTTPS,
including its existing troubleshooting reference and exact milliseconds-to-
seconds conversion. Existing supplied retry counts retain their suffix; first
non-retriable helper outcomes have none. SSH M1 uses the truthful SSH noun.
Runtime's in-process SSH producer now carries its validated existing pool
allocation/interaction values instead of widening smaller limits to hardcoded
30000/120000; allocation queue wait only shortens one deadline. Default ceilings,
HTTPS defaults and the supplied-carrier driver contract remain unchanged.
Focused v7 passes two tests across missing/unexecutable Git, M4 and M10: actual
backend clone/fetch/push, public fetch/push/private-lock materialize, direct typed
Runtime/Open budget and metadata, exact common text, retry/redaction, no password
offer and retained cleanup/slot restoration. Initial v1 captures policy refusal
of the old widening producer; v2 captures real M1 lost to GitCommandFailed.
Subsequent failures were fixture expectation/registration/type-wiring repairs,
not waived product gates. Final union affected gate v8 passes442/0/4 ignored in38.31s; candidate
Clippy v9 exits0 with50 retained warnings. Exact cwd/argv/env and log hashes
are recorded in the external receipt. Root-owned core CredentialHelpers.md/
README and CLI Troubleshooting.md are separate Surface inputs. Full normal and
both candidate phase gates below predate this final bounded consumer patch.

`gwz-tr222-clock-transport-final-v8.log` passes every transport integration,
17 deterministic clock rows, its safe callback ownership unit test and doctest;
strict library Clippy passes in `gwz-tr222-clock-transport-clippy-v9.log`.
The prior v7 compile failure attempted unavailable cfg_if in the dependency-free
transport crate. It was corrected with an explicit braced test module and safe
standard-library Wake, without dependencies or unsafe exceptions. Core clock
tests cover actual Pool/Control/final-result timing provenance for all three
winners, prepared expiry/refusal, generic network errors without provenance,
zero allocation/no launch and immutable terminal ownership.

The ordinary final runner (`gwz-tr222-core-normal-clock-v4.log`) passes all phases
and its 2,103 native tests, one ignored. A direct full candidate cargo invocation
(`gwz-tr222-candidate-full-clock-v6.log`) omitted the normal runner's required Git
and filesystem phase environment. Its V1 native-repository fixtures then used
the default fake backend and reported repository missing. The exact store case
is RED without phase settings and GREEN with native settings in
`gwz-tr222-candidate-v1-counterexample-v1.log` and `v2.log`. That invalid full
invocation was interrupted and is not a gate result or inherited-debt waiver;
the correct candidate phase runner is recorded separately. Its independent
runner/environment/throughput failures are retained for comparison with the
correct final run, without claiming they share the V1 cause.

Final corrected whole candidate phase gates are GREEN under both
`gwz_transport_candidate` alone and with `gwz_session_candidate`:
`gwz-tr222-candidate-normal-clock-v12.log` and
`gwz-tr222-candidate-both-clock-v10.log`. Each passes fake-FS18, migrated129,
native-crosscheck1 and native2,652/0/7 ignored; native phases take424.63/424.37s.
Both use the own-lane prepared manifest, Rust1.95.0 and the ordinary retained
profiles, with `target/candidate-transport` or `target/candidate-both` respectively.
The corrected snapshot fixture now expects accepted `git`, preserving complete
captured-environment equality. The descendant heartbeat fixture keeps its typed
timeout/start assertions and checks no further writes after existing cleanup
grace; product deadlines/cleanup ownership are unchanged. Both exact regressions
pass in the final full gates. All source, pinned generator, exact historical
additive/drift and nine provenance checks also pass. A system-Python transport
regen lacked the pinned release, and manual transport-global invocations used
the wrong allowlist; corrected pinned-interpreter/exact-allowlist checks are
authoritative and pass without source/allowlist policy changes.

## Root settlement verification

The first unpublished source checkpoint `5e32ee9f` failed the exact-commit
boundary gate: four new test-only source-loading edges were missing from its
inventory. Root verified regular in-crate targets and enclosing test boundaries
and added only those four entries, without weakening the checker. Before any
review or push, root rebuilt that unpublished checkpoint through GWZ's `forall`
soft reset to the accepted base and normal `gwz add`/`gwz commit`. No accepted
history, tests, protected Rust tree or gate floor was rewritten or waived.

Root's staged diff check includes newly tracked split leaves, unlike the earlier
working-tree diff check. It found EOF-only blank/trailing whitespace in five
core test leaves and transport stream/state.rs. Root removed only those trailing
bytes in a separate settlement correction; no declaration, string, test assertion
or behavior changed. The final review tuple includes this correction. Gate source
fingerprints above precede it; root's settlement manifest names the final bytes.

## Touched paths and source receipt

Fresh implementation review inputs are this receipt, its exact owned-path/hash
inventory, the accepted HelpersDesign/context/configuration-view/SSH-clock
authorities, and the named raw gate logs. Code and State must inspect the complete
helper/secret/clock call graph and the v8 parser-environment correction together;
earlier mechanism reviews do not cover these working source bytes. Surface also
consumes the accepted M4/M8/M10/URL-username behavior and MAIN's external Python
help-only input `a0773afa0cb3ae09be742dadd216dfd13527e692`; those three help files
are not copied or edited in this lane. Final acceptance/aggregate and platform
release campaigns remain coordinator-owned and open.

Deterministic clock/arbitration cases are in transport `tests/setup_clock.rs`;
safe wake/drop ownership is in `src/pool/setup_clock/tests.rs`. Core timing
witness/admitted terminal cases are in `ssh_setup_context/tests.rs`; actual
native offer/admission/expiry/cancellation/rejection cases are in
`ssh_tests/password_helpers.rs`. These complement the full native key/agent,
pooling and HTTPS route/account/operation regressions; no test is skipped or
waived for this feature. Existing ignored tests and lint/size debt stay labeled.

The current working-source SHA256/line inventory and gate commands are at
`/Volumes/projects/limbo/gwz-tr222-implementation-source-receipt-20261003.json`.
This retrospective local receipt excludes the inherited private BugReport and
old context proposal, and is not campaign evidence. It labels later source
deltas rather than claiming an exact earlier-run snapshot.

### gwz-core

- `dev-docs/GwzTransportCredentialHelpersImplementationRecord.md`
- `docs/ErrorCatalog.md`
- `docs/MessageCatalog.md`
- `protocol/candidate/candidate-generator.json`
- `protocol/candidate/candidate-regenerator.py`
- `protocol/candidate/candidate_generated.py`
- `protocol/candidate/corpus/golden.json`
- `protocol/candidate/corpus/rust/vectors.rs`
- `protocol/candidate/test_candidate_regen.py`
- `protocol/check_log_additive.py`
- `protocol/gwz.taut.py`
- `scripts/candidate_switch_inventory.txt`
- `scripts/checks/cfg_boundaries_allowlist.json`
- `scripts/checks/process_globals_allowlist.json`
- `src/git/endpoint/agent_job.rs`
- `src/git/endpoint/agent_job/control.rs`
- `src/git/endpoint/agent_job/control/shared.rs`
- `src/git/endpoint/agent_job/tests.rs`
- `src/git/endpoint/budget_wait_tests.rs`
- `src/git/endpoint/helper_script.rs`
- `src/git/endpoint/https_auth.rs`
- `src/git/endpoint/https_auth/executable.rs`
- `src/git/endpoint/https_auth/file_worker.rs`
- `src/git/endpoint/https_auth/lookup.rs`
- `src/git/endpoint/https_auth/owner.rs`
- `src/git/endpoint/https_auth/runner.rs`
- `src/git/endpoint/https_auth/runner_tests.rs`
- `src/git/endpoint/https_auth/secret.rs`
- `src/git/endpoint/https_auth/test_support.rs`
- `src/git/endpoint/https_auth/view.rs`
- `src/git/endpoint/https_auth/view/framing.rs`
- `src/git/endpoint/https_auth/view/tests.rs`
- `src/git/endpoint/https_auth_integration_tests.rs`
- `src/git/endpoint/https_auth_integration_tests/challenge.rs`
- `src/git/endpoint/https_auth_integration_tests/policy.rs`
- `src/git/endpoint/https_auth_integration_tests/route.rs`
- `src/git/endpoint/https_budget_tests.rs`
- `src/git/endpoint/https_connection.rs`
- `src/git/endpoint/https_destination.rs`
- `src/git/endpoint/https_destination/control.rs`
- `src/git/endpoint/https_fixture.rs`
- `src/git/endpoint/https_opening.rs`
- `src/git/endpoint/https_opening_tests.rs`
- `src/git/endpoint/https_policy.rs`
- `src/git/endpoint/https_pool.rs`
- `src/git/endpoint/https_remote.rs`
- `src/git/endpoint/https_remote_tests.rs`
- `src/git/endpoint/https_worker.rs`
- `src/git/endpoint/https_worker/budget.rs`
- `src/git/endpoint/https_worker/challenges.rs`
- `src/git/endpoint/https_worker/credential_tests.rs`
- `src/git/endpoint/https_worker/credentials.rs`
- `src/git/endpoint/https_worker/helper_budget_tests.rs`
- `src/git/endpoint/https_worker/prepare.rs`
- `src/git/endpoint/https_worker/serve.rs`
- `src/git/endpoint/https_worker_tests.rs`
- `src/git/endpoint/https_worker_tests/discovery.rs`
- `src/git/endpoint/mod.rs`
- `src/git/endpoint/placement_endpoint/retry_tests.rs`
- `src/git/endpoint/placement_endpoint_tests.rs`
- `src/git/endpoint/setup_retry.rs`
- `src/git/endpoint/shared_reservation.rs`
- `src/git/endpoint/ssh_local.rs`
- `src/git/endpoint/ssh_password.rs`
- `src/git/endpoint/ssh_password_helpers.rs`
- `src/git/endpoint/ssh_pool.rs`
- `src/git/endpoint/ssh_setup.rs`
- `src/git/endpoint/ssh_setup/tests.rs`
- `src/git/endpoint/ssh_setup_context.rs`
- `src/git/endpoint/ssh_setup_context/tests.rs`
- `src/git/endpoint/ssh_tests/mod.rs`
- `src/git/endpoint/ssh_tests/password_helpers.rs`
- `src/git/endpoint/ssh_tests/placement_endpoint.rs`
- `src/git/endpoint/ssh_tests/retry.rs`
- `src/git/endpoint/ssh_worker.rs`
- `src/git/endpoint/ssh_worker/endpoint.rs`
- `src/git/endpoint/ssh_worker/open_request.rs`
- `src/git/endpoint/ssh_worker/runner.rs`
- `src/git/endpoint/ssh_worker_tests.rs`
- `src/git/endpoint/stream_io.rs`
- `src/git/gitbackend.rs`
- `src/git/gitbackend/transport.rs`
- `src/git/gitbackend/transport_binding.rs`
- `src/git/gitbackend/transport_observations.rs`
- `src/git/verify_checkout_state.rs`
- `src/model/error_code.rs`
- `src/model/mod.rs`
- `src/protocol/candidate_generated.rs`
- `src/protocol/convert.rs`
- `src/protocol/generated.rs`
- `src/transport_host/cancellation_tests.rs`
- `src/transport_host/cleanup_tests.rs`
- `src/transport_host/endpoint_environment.rs`
- `src/transport_host/endpoint_environment_tests.rs`
- `src/transport_host/https_cancel_mux_tests.rs`
- `src/transport_host/helper_failure.rs`
- `src/transport_host/https_endpoint.rs`
- `src/transport_host/https_endpoint/poll.rs`
- `src/transport_host/https_endpoint/retry.rs`
- `src/transport_host/https_endpoint/retry_tests.rs`
- `src/transport_host/https_helper_projection_tests.rs`
- `src/transport_host/https_policy_tests.rs`
- `src/transport_host/https_policy_tests/admission.rs`
- `src/transport_host/https_policy_tests/auth_receipts.rs`
- `src/transport_host/https_policy_tests/cancellation.rs`
- `src/transport_host/https_policy_tests/route_policy.rs`
- `src/transport_host/https_tests.rs`
- `src/transport_host/mod.rs`
- `src/transport_host/request.rs`
- `src/transport_host/request/https_failure.rs`
- `src/transport_host/session.rs`
- `src/transport_host/session/driver.rs`
- `src/transport_host/session/driver/opening.rs`
- `src/transport_host/ssh_helper_projection_tests.rs`
- `src/transport_host/ssh_helper_projection_tests/workspace.rs`
- `src/workspace_ops/handle_fetch.rs`
- `src/workspace_ops/push_member.rs`

### gwz-transport

- `protocol/transport.ir.json`
- `protocol/transport.taut.py`
- `scripts/regen.py`
- `src/admission.rs`
- `src/codec.rs`
- `src/codec/failure_detail.rs`
- `src/codec/https_shape.rs`
- `src/codec/validate.rs`
- `src/pool/allocation.rs`
- `src/pool/asynchronous.rs`
- `src/pool/clock.rs`
- `src/pool/lifecycle.rs`
- `src/pool/machine.rs`
- `src/pool/mod.rs`
- `src/pool/setup.rs`
- `src/pool/setup_clock.rs`
- `src/pool/setup_clock/state.rs`
- `src/pool/setup_clock/tests.rs`
- `src/pool/setup_clock/transitions.rs`
- `src/protocol.rs`
- `src/stream/asynchronous.rs`
- `src/stream/incoming.rs`
- `src/stream/machine.rs`
- `src/stream/mod.rs`
- `src/stream/state.rs`
- `tests/failure_detail.rs`
- `tests/https_reuse.rs`
- `tests/https_scope.rs`
- `tests/https_selector.rs`
- `tests/mux.rs`
- `tests/placement_v2.rs`
- `tests/policy.rs`
- `tests/sequenced.rs`
- `tests/sequenced_random.rs`
- `tests/setup_clock.rs`

### gwz-py

- `scripts/check_protocol_drift.py`
- `src/gwz/protocol/generated/api.py`
- `src/gwz/protocol/generated/gwz.ir.json`
