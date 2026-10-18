# TR2.2 / TR2.22 / TR2.23 implementation checkpoints

Date: 2026-10-03. Status: **implementation in progress; no acceptance review
or release qualification claimed**. The root coordinator owns settlement and
review dispatch. The implementation drafter performs no Git/GWZ mutations.

## Authority and baseline

Baseline: root `12f11c7949919834fe8858247dc4c0cd49a8134b`, core
`2e64e88a28c332ed422cc390adc76738dc701bb1`, transport
`35475977530171ab77ee2fbb1e8128f938acb5ae` in the `tr2-22` lane.
The root and core working drafts recorded in the handoff remain out of scope.

Controlling behavior is [TR1.6 revision 4](GwzTransportCredentialHelpersDesign.md),
accepted after revision 3's Consistency/Safety/Surface GO, with the operator's
OQ1(b), OQ2(a), OQ3(b), OQ4(a), OQ5(a), OQ6(a), OQ7(1) and added retry count.
[Amendment 2 revision 6](GwzTransportReleasePlanAmendment-2.md) and root
`dev-docs/GwzTransportHandoff.md` §§6.1/6.4 set the execution and recording order.
This record does not rewrite frozen prose or invent another authentication policy.

The helper's default interaction allowance is **120 seconds**, shortened by
a positive Open interaction allowance. TR1.6 §3.3 and D10 state that duration.
The **75** in OQ5 and handoff §6.1 is the proposed `credential_helper_timeout`
error-code value, not a duration. A zero Open interaction deadline means no
caller-provided shortening; a retained zero budget remains exhausted.
OQ6(a) charges both helper admission waits to the retained allocation budget;
each started helper receives its own full interaction allowance.
Correction to the initial takeover brief: its reference to a 75-second helper
timeout misread that code value. Root confirmed the 120-second duration after
source verification. This is a precedence correction, not an amended behavior.

OQ1(b) permits an encoded username in the single serialized `url=` line. It
does not permit a separate decoded `username=` input line or a URL/username in
wire failure detail. Paths and usernames decoding to control characters refuse
before Open, including after redirects. OQ3(b) removes
`credential.interactive=false`; `core.askPass=` and `GIT_TERMINAL_PROMPT=0` remain.

## Ordered checkpoints

1. **TR2.2 characterization.** Keep the split HTTPS fixture and introduce a
   fake `gh` configured by absolute shell-helper path and by `PATH`. Exercise
   anonymous discovery, a Basic challenge, and authenticated discovery through
   the production transition wrapper. First record both failures on the old
   direct-gh spawn. The old C7 spawn/write correction is already replaced by
   the baseline's broken-pipe handling; do not recreate it. TR2.22's one
   configured-helper runner supplies the eventual green behavior.
2. **TR2.22 wire detail.** Define `Failure`'s optional detail in taut and
   regenerate transport artifacts. Its closed values cover M8's fixed causes,
   M6's maximum four HTTP scheme tokens of maximum 32 ASCII characters, and
   endpoint retry attempt/maximum counts. It carries no URL, helper output,
   stderr, credential, or free-form diagnostic text. Keep existing failure
   classification, effects, facts, setup causes and retry state transitions.
   Test absence, valid round trips, invalid bounds/tokens/counts, and same-build
   consumers. Populate retry counts from the endpoint machine's final result.
   **Per-step peer-blind Code/State review before passing this checkpoint.**
3. **TR2.22 secret runner and HTTPS route ownership.** Replace direct gh with
   resolved, supervised `git credential fill` under the accepted environment,
   process-group, admission, output and zeroization rules. Apply challenge-only
   lookup, effective URL, no POST replay, one lookup per route, retirement,
   rejection and credential pool scope. Preserve the TR2.2 regressions. Test
   timeout/cancel/drop and retained-child ownership, queue charging, lingering
   stderr, malformed output, empty values, redaction and route/pool isolation.
   **Per-step peer-blind Code/State review; Surface for the new messages.**
4. **TR2.23 SSH password-only parity.** Share the accepted helper runner when
   an ambient SSH server offers only password authentication, matching the
   released callback. Keep selected-key policy, agent/key algorithms, host
   trust, retry semantics and transport placement ownership. Test a real
   password-only fixture with helper success, refusal, policy-off and cleanup.
   **Secret-handling Code/State review before passing this checkpoint.**
5. **Aggregate package.** Complete ordinary core runner and candidate suites,
   format/source guards/inventories, and a settled aggregate review. Root owns
   amendment 2 revision 7 and the broader handoff/checkpoint authority records;
   this lane supplies precise replacement texts and an implementation report.

## Ownership and bounds

Mutable scope: core endpoint authentication, destination, worker, route/pool,
retry-count producers and adapters, host endpoint environment wiring, their
focused tests, exact source/checker inventories and corresponding core docs;
transport taut schema, generated artifacts, bounded validation and HTTPS pool
credential scope. Mechanical `Failure` literal updates may touch current
same-build callers. A generated transport file is exempt from the handwritten
500-line target; cohesive new handwritten modules stay below it. Existing
large auth code is split only as needed by this mechanism.

CLI/Python activation and generated public-protocol coordination remain the
root coordinator's integration responsibility. No edits to git2-rs, gwz-git,
private evidence, protected drafts, Windows implementation, statistics or
unrelated session interfaces. No push, merge, tag, installation, release,
compiler probes or source-mutation probes. Platform and performance campaigns
remain deferred. Any new semantic conflict is reported before an implementation
choice crosses an accepted ownership boundary.

Use Rust 1.95 default profiles with incremental/debug retained. Ordinary builds
use core/target; transport-only uses root target/candidate-transport; both
switches use root target/candidate-both; transport tests use transport/target.
External candidate preparation is generated by this lane's
`tests/transport_backend/prepare.py` and must resolve all members to this lane.
Focused failing tests precede code, then focused green gates, then the normal
`scripts/run_tests.py` and declared source gates. Evidence is executed outcomes,
not a fixture inventory. Reviewer inputs come from the canonical review-loop
template; no dirty-tree acceptance or implementer self-GO.

## Executed evidence

Takeover preserved the two configured-gh fixtures and their explicit Unix
module boundary. The initial transport-only command used this lane's candidate
prep and default test profile. Its inherited unqualified Cargo resolved to
Rust 1.96; that compiler is recorded for the characterization only. Final gates
explicitly select `cargo +1.95.0` as required by the lane brief. The command was:

```sh
RUSTFLAGS='--cfg gwz_transport_candidate' \
CARGO_TARGET_DIR=/Volumes/projects/limbo/gwz-dev-tr2-22/target/candidate-transport \
cargo test --offline \
  --manifest-path /Volumes/projects/limbo/gwz-tr222-candidate-core-20261003/Cargo.toml \
  --lib gh_configured -- --nocapture
```

Executed result: **0 passed, 2 failed**, both
`gh_configured_by_absolute_path_answers_https_challenge` and
`gh_configured_through_path_answers_https_challenge`. Each fails with
`Authentication`, method `Gh`, `credential_offered=false`, before the
authenticated discovery. This is the required TR2.2 red characterization;
it is not implementation acceptance. Boundary and process-global source guards
pass with the fixture's same-commit path edge.

## Wire checkpoint representation

The authored taut schema adds `Failure.detail` at tag 5 with missing-or-null
absence. `FailureDetail` has optional `helper_cause` (tag 1), `pipe_kind` (2),
`schemes` (3), and `retry_attempt` (4). The cause enum is exactly M8's seven
fixed alternatives. Pipe kinds are a closed whitelist of stable Rust error-kind
names; arbitrary error text is refused. Schemes are optional so `Some([])` can
represent M6's server naming no scheme while absence means no M6 diagnostic.
Helper-cause and scheme alternatives cannot coexist. A retry count is two
positive counts with `attempt <= attempts <= u32::MAX`.

Every Failure carrier is checked by both local codec admission and encoded
decode. Normal generic frame/allocation limits still apply before the generic
CBOR tree and generated typed values allocate. The added nested-map nodes are
charged by the generated admission visitor; its existing per-node conservative
allocation charge also covers the boxed diagnostic. No helper strings, URLs,
headers beyond scheme tokens, or usernames cross the field.

The release generator does not have a boxed-reference option. Transport's
regenerator therefore applies one exact, fail-closed Rust representation
projection to the generated declaration and decoder: `Option<Box<FailureDetail>>`.
Schema/CBOR tags and generated admission semantics are unchanged. This keeps
`Failure` below Clippy's large-error threshold; tests pin its common layout
below 128 bytes, test all diagnostic round trips, and check generation drift.
Generated Rust is never hand-edited.

Endpoint `Final::wire_failure` attaches its existing attempt/maximum counts
when the failure exhausted a retriable setup budget (including 1 of 1), or an
attempt after the first failed. A first authentication/trust refusal keeps its
existing display. The retry transitions, classification and counts themselves
are unchanged. Both SSH and HTTPS boundaries project the endpoint's final
result, including the key's retained result for queued members; their own facts
remain merged as before. Both drivers read the wire count. A missing count
produces no inferred suffix, and malformed internal fixture counts are refused.

Mechanical `detail: None` updates cover current same-build Failure literals,
including transport's README doctest and core's archive-consumer fixture. The
immutable retained transport reader is untouched. Public GWZ request/response
schema and CLI/Python activation are outside this checkpoint.

## Wire checkpoint gates and remaining boundary

Executed on Rust 1.95.0, with the target directories named above:

- Transport: default and `unstable-sequenced` test suites pass, including the
  README doctest; all seven focused detail tests pass. `cargo clippy --lib --
  -D warnings` passes. The installed pinned taut interpreter verifies all four
  generated artifacts, and the six regenerator tests pass.
- Core transport-only candidate: all 25 `retry_tests::` tests pass, including
  the endpoint first-attempt exhaustion regression that was red before the
  count producer changed. The budget-installation test retains a test-only
  accessor under an explicit `cfg_if!` boundary; production drivers have no
  budget-based diagnostic inference.
- Core transport-only setup-machine and host selections: 130 passed, zero
  failed, one ignored. The two known-red configured-gh tests are outside this
  selection and remain the next checkpoint's characterization.
- Ordinary core: `RUSTUP_TOOLCHAIN=1.95.0 python3 scripts/run_tests.py --lib
  --offline` completes successfully through all fake/native selections. The
  final real-backend selection has 2,103 passed, zero failed, one ignored.
  Its source/checker gates also pass.

Strict transport all-targets/all-features Clippy was executed and fails on two
unchanged baseline test lints: `tests/https_reuse.rs`'s bare
`endpoint.next_action().unwrap().1` (`unnecessary_operation`), and
`tests/pool.rs`'s default configuration followed by assignments
(`field_reassign_with_default`). `git show` at the baseline confirms both;
this checkpoint introduces no new Clippy diagnostic. This is recorded debt,
not a claim that the full all-targets lint gate passed.

The isolated archive-consumer proof is a **post-settlement coordinator gate**:
its runner verifies the exact transport revision and refuses a dirty archive.
The drafter has neither fabricated archive provenance nor changed that runner.
Root must package the settled transport and execute the consumer proof on
Rust 1.95 before dispatching the wire acceptance review. Existing generated
consumer wrappers and the immutable retained transport reader stay unchanged.

Canonical Code/State input drafts are prepared in `dev-docs/reviews/` from the
review-loop template. Their `{{SETTLED_*_SHA}}` tokens are deliberate pending
root settlement; they are not dispatched review objects. The wire checkpoint's
mandatory dual tier does not accept the next secret-runner checkpoint, and the
two configured-gh characterization tests remain intentionally red until that
runner is implemented.
