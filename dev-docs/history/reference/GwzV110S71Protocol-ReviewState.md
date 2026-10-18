# S7.1 protocol fields (C2.1/C2.2): STATE-AXIS REVIEW

**Review object:** gwz-core `158c628c1ea6fd003b568f3064024a8bd95ea60d` (one commit over `1c16373f`) and gwz-py `0c8bcc7003b837f090c851285387d89f0ada56a2` (one commit over `da098f5`), in lane `/Volumes/projects/limbo/gwz-dev-c21`.
**Baseline:** gwz-core `1c16373f`, gwz-py `da098f5`. gwz-transport `a27817b9` and gwz-cli `8d909ec4` were used as unchanged references.
**Date:** 2026-10-16
**Axis:** STATE. This covers durable encoded state, defaults and absence, fail-closed direction (never invent a fact) and mixed versions.

**Verdict: GO.** P0 0, P1 0, P2 0, P3 2.

## 0. Evidence base

**Tuple.** I checked it at the start and again at the end, and it was identical both times:

| Repository | HEAD | Working tree |
|---|---|---|
| gwz-core | `158c628c…` | 1 untracked file, `dev-docs/GwzRemoteTransportBugReport.md`. It was already there at the start and is not mine. |
| gwz-py | `0c8bcc70…` | clean |
| gwz-transport | `a27817b9…` | clean |
| gwz-cli | `8d909ec4…` | clean |
| lane root | `63e4076b…` | — |

I wrote nothing in the lane. All of my output went to `/Volumes/projects/limbo/build-targets/reviewc2-state-*`.

**Commands I ran, with results:**
- **`python3.13 protocol/regen.py --check`** (gwz-core): exit 0, "committed protocol artifacts are current". This also runs `check_log_additive`.
- **`candidate-regenerator.py --check`** (core schema plus `gwz-transport/protocol/transport.ir.json`): exit 0, "placement candidate regeneration verified". The candidate artifacts stay byte-identical.
- **`check_log_additive.py protocol/gwz.taut.py`** (regen venv, taut-proto 0.10.0): OK, `sha256:4d377a49…`.
- **gwz-py `scripts/check_protocol_drift.py`:** OK. `scripts/regen_protocol.py --check` (taut-proto 0.10.0): OK.
- **`cargo +1.95.0 test --test protocol`, ordinary build:** 44 passed, including `ordinary_operation_policy_and_observation_carry_the_1_1_0_fields`.
- **Candidate build through `prepare.py`** into `build-targets/reviewc2-state-cand`, with `RUSTFLAGS='--cfg gwz_transport_candidate'`:
  - `--test protocol`: 44 passed.
  - `--lib` filtered to `max_retries transport_observations validate::tests reused pool`: 112 passed, 0 failed. This includes the `merge::validate` tests, the `transport_observations::https_tests`, `retry_tests::a_negative_max_retries_is_refused_before_the_request_registers` and the `max_retries_is_refused…` test.
- **Python tests in my own venv** (`build-targets/reviewc2-state-venv`, pytest plus taut-proto 0.10.0):
  - gwz-core `tests/transport_consumer/candidate/test_candidate.py` and `protocol/candidate/test_candidate_regen.py`: 21 passed.
  - gwz-py `test_codec.py`, `test_log_protocol.py`, `test_transport_reporting.py`, `test_cli_push_render.py`, `test_cli_url_scheme.py`: 30 passed, including the three new codec tests.
- **My own experiments** (scripts in `build-targets/reviewc2-state-logs/work`):
  - **Pre-C2 golden bytes, Python.** I decoded all 155 vectors of the pre-C2 `protocol/corpus/golden.json` (from `1c16373f`) with the post-C2 schema, strict struct decode included. 0 failures. Every one of the five members reads `None`, and every other value equals the pre-C2 reader's decode.
  - **Post-C2 golden bytes, old reader.** The pre-C2 schema decodes all 155 post-C2 vectors, 68 of which changed, with the same values outside the five members.
  - **Pre-C2 golden bytes, Rust.** A scratch crate linked the post-C2 ordinary gwz-core and ran the pre-C2 `vectors.rs` table through `try_decode` and each type's `from_cbor`. All 155 decoded, and `OperationPolicy.max_retries` and the four receipts are `None`.
  - **Stripped schema equals the pre-C2 schema.** Production minus the five members (the idiom `test_candidate.py` uses for D3) exports an IR equal to the pre-C2 `gwz.taut.py` (sha256 `76455b40…`). So the "old reader" in `test_candidate.py` really is the pre-C2 reader at this commit.
  - **Projection mutations.** I ran the core and gwz-py projections against the post-C2 IR. The unchanged IR passed both. Each of these failed both:
    - an accidental `TransportObservation.extra@13`
    - an accidental `OperationPolicy.extra@10`
    - `stream_id` retyped to `str`
    - `max_retries` as house-style `optional=True`
    - `reused` moved to tag 13
    - `reused` removed
    - a pre-existing field (`authenticated`) changed to `missing_ok`

    Reordering a stripped field within its message passes, which is correct because order carries no wire meaning.
- **Lane evidence** (`gwz-core-evidence/…/runs/2026-10-16-c21`):
  - gwz-py ordinary: 1037 passed, 23 skipped.
  - gwz-py candidate, and candidate with session: each 1059 passed, 1 skipped.
  - gwz-cli ordinary and candidate builds: exit 0.
  - Windows receipt `c21-20261016-a`: PASS.

## Problem, requirement provenance, and scope

- **Provenance.** Amendment 2, §3.11 S7.1 (1.1.0) "The protocol" (lines 276–284 as the brief cites them) says the ordinary build compiles the transport against the production schema, and that any candidate field the 1.1.0 transport needs is named, regenerated in gwz-py and placed under S7.5.
- **`OperationPolicy.max_retries`.** Traced to code: gwz-cli writes it (`globalargs/retry.rs:24-26`). The transport reads it in `transport_host/request.rs:450-489` and `mod.rs:479-496`. It is the S7.2 `--max-retries` feature. It is required.
- **The four receipts.** Traced to code: `transport_observations.rs:113-120` writes them on every transport-route open. They are also the S6.3/S7.3 route assertion's discriminator. Leaving them candidate-only would need a cfg inside `opened()` once C3b removes the switch. The lane owner disposed D1 as B. They are required.
- **Nothing beyond the five was added.** The rest of the placement projection stays candidate:
  - `TransportPlacement`
  - `TransportOptions.placement` and `.endpoint_path_base`
  - both `transport_message` fields
  - the five capability extras
  - the 42 re-exports

  The candidate overlay lost exactly the five entries, and the candidate output is byte-identical.
- **Disposition shape is respected.**
  - D2: all five are `MISSING_OK`, which generates `try_get_opt`.
  - D3: one production module (`generated.rs`), with the old reader built by stripping in the test.
  - D4: gwz-py JSON gains the four keys.
  - D5: generated dataclass constructors need the new arguments.
  - D8: `cli_hint` is untouched.
- **Scope.** Every changed file is inside the proposal's §0 boundary. gwz-transport and gwz-cli are unchanged.

## 1. Findings

### P3-1: In the ordinary build, merge's "refuse every policy field it does not act on" rule is reopened for `max_retries`

- **Root cause:** the switch was not moved with the field. C2.1 made `OperationPolicy.max_retries` an ordinary field. The refusal of that field in merge still sits inside a `gwz_transport_candidate` arm, whose stated premise is now false.
- **Location:** `gwz-core/src/workspace_ops/merge/validate.rs:187-196`. The arm says "Only the transport build's policy has the field…". Its test is at `:531-558` and is candidate-only. Inventory rows: `scripts/candidate_switch_inventory.txt:40-41`.
- **Violated invariant:** merge refuses every policy field it does not act on (`validate_common_meta`'s closed list). Commit `828093f2` established this to close State review P3-3 of `9a235acb`. That commit's text also states "the ordinary build's policy has no such field, and its validator is unchanged". Since C2.1 that statement is false.
- **Reproduction** (ordinary build at `158c628c`):
  1. Build `MergeRequest { meta.policy: Some(OperationPolicy { max_retries: Some(0), ..Default }) }`. This compiles in the ordinary build now, and the field round-trips per `tests/protocol.rs`.
  2. Call `validate_merge_request`. It returns Ok, because the `cfg_if` arm is compiled out.
  3. Through gwz-py's ordinary wheel, the same happens with a raw `bridge.call("merge", …)` carrying `OperationPolicy(..., max_retries=0)`. Any other policy field gets `MergeValidationFailed` naming the field.

  Before C2 the ordinary build could not express the field in typed code; on the wire an unknown tag 9 was dropped the same way.
- **Impact:** a budget the caller named is accepted and ignored instead of refused. Merge opens no connection, so nothing functional changes; the loss is honest feedback. The exposure is limited:
  - No in-tree writer sets the field in the ordinary build. gwz-cli's `RetryArgs` is candidate-only, and gwz-py's `Client` hard-codes `None`.
  - The gap lasts only until C3b, which classes site #20 as (a) and makes it unconditional.
  - It ships only if a release is cut from this line before C3b.
- **Correction:** either option closes it.
  - Remove the `cfg_if` around the `reject_policy_field("policy.max_retries (--max-retries)", …)` call and around its test, and drop inventory rows 40–41. The proposal's own §1.2 row 20 says the refusal becomes "unconditional once D1 puts the field in the ordinary protocol", and that condition is met now.
  - Or record the interim gap explicitly in the C2.3 checkpoint as accepted until C3b, and reword the arm's comment so it no longer claims the ordinary policy lacks the field.
- **Closure test:** `max_retries_is_refused_like_every_policy_field_merge_does_not_act_on` runs in the ordinary build (no cfg) and passes, and `check_candidate_switches.py` stays green with the updated inventory.

### P3-2: The receipts' absence rule in the production schema is incomplete, so null can be misread as "native route"

- **Root cause:** documentation. The new production schema comment states when the receipts are absent and leaves out the transport-route setup-failure case.
- **Location:**
  - `gwz-core/protocol/gwz.taut.py:1104-1107`: "Absent on the native route and in an older writer's map (MISSING_OK)."
  - gwz-py `src/tests/test_transport_reporting.py:43`: "null off the transport route".
  - The gwz-py commit message says the same: "(null off the transport route)".
- **Violated invariant:** never invent a fact. A null receipt means "no open was reported for this row", not "native route".
- **How the code behaves:**
  - `TransportAttempt::opened()` (`transport_observations.rs:113-120`) is the only writer, and it runs only from the `report` closure on `Opened` (`transport_binding.rs:250-255`, `:289-294`).
  - A transport-route row whose setup fails, is refused or is cancelled before `Opened` keeps all four receipts `None`. `failed()` records only the error, and the `facts` closure can still set `credential_offered`/`authenticated`.
  - Non-SSH/HTTPS remotes (file, git://, Windows `http://`) on the transport build are also null.
- **Reproduction** (reading the code; the paths above are deterministic): on a transport-route SSH fetch whose authentication fails, the row carries the credential facts and the error, while `endpoint_id`, `connection_id`, `stream_id` and `reused` are null. The gwz-py `--json` row (`dataclasses.asdict`, `cli_render_parts/machine.py:11-13`) then reads exactly like a native-route row.
- **Impact:** gwz-py's own route assertion (`test_client_host_transport.py:133-151`) uses receipt presence as the route discriminator. D4 makes the keys user-visible, and S7.5's Surface entry and the migration notes will be written from this text. A consumer who follows the comment would report "native route" for a failed transport attempt, which is an invented fact.
- **Correction:**
  - Reword the schema comment to say "Absent unless the transport route reported an open for this row: on the native route, on a transport-route row whose setup failed or was refused before the open, and in an older writer's map". Python comments do not enter the IR, so no regeneration is needed.
  - Use the same wording in the C2.3/S7.5 Surface entry and the gwz-py migration note (D4).
- **Closure test:** an existing candidate failure-path test, or a new one in `transport_host` (SSH auth refused), asserts the failed transport-route row has all four receipts `None` while `credential_offered` is set. The S7.5 text then cites that behaviour.

## 2. Invariant analysis (attacks that failed)

- **Durable encoded state.** These messages are not persisted anywhere:
  - In the IR, the transitive parents of `OperationPolicy` are only `*Request`/`RequestMeta`. Those of `TransportObservation` are only `*Response`/`ResponseMeta`/`OperationResult`/`ResponseEnvelope`.
  - No `Log*` message (the log output records), checked-artifact record or `DiffOutputRecord` reaches them.
  - No code outside generated code encodes them to disk.
  - gwz-py's operation records are in memory.
  - The `cli_parity` fixtures use dotted paths, not bytes.

  The only stored bytes are the golden corpus and the gwz-py test hex. Every pre-C2 golden vector decodes under the new Rust and Python readers with the five fields `None`, and the old reader decodes every new vector (§0).
- **Absent, null, 0, value.**
  - Absent and null both read as `None`, which means the default of 3: `request.rs:482-489` with `DEFAULT_MAX_RETRIES`.
  - `Some(0)` means no retries.
  - A negative value, or one above `u32::MAX`, is refused in `validate_meta` (`request.rs:450-468`) before registration, which is fail-closed.
  - The Cli placement refuses any budget (`mod.rs:442-451`).
  - gwz-py now writes tag 9 as null where before it wrote nothing. Both decode to `None`, so the effective budget is unchanged.
  - `_merge_policy`'s all-None test is unaffected, because `max_retries` is always `None` from `Client`.
  - A present but malformed value is an error (`tests/protocol.rs` text at tag 9, and `test_candidate.py` `11: "seven"`).
- **`MISSING_OK` versus `optional=True`.** The generated Rust uses `try_get_opt` for exactly the five fields, and the pre-existing fields keep `try_get`. Changing a stripped member to house style fails both projections.
- **Projection strips.**
  - Core and drift require the exact field object (name, tag, scalar, `missing_ok`, transient, merge) before removing it.
  - Any accidental new field or a change to a pre-existing field changes the hash (§0 mutations).
  - `test_log_protocol.py`'s copy checks only tag and `missing_ok`, not the scalar type. A retype is still caught by core's `check_log_additive`, which `regen.py --check` runs, and by gwz-py's drift check, which package-smoke, publish and release run.
  - No fourth copy exists: I searched gwz-core, gwz-py, gwz-cli, gwz-transport and taut for `4d377a49`, `dev-docs` excluded.
- **Receipts never claim a fact.** `opened()` writes all four together from `Opened`. `begin()` defaults them to `None`. Nothing ever writes `reused: Some(false)` for a row that did not open. gwz-cli's JSON comes from a fixed field list (`response_meta_json.rs:17-79`), so it is unchanged.
- **Mixed versions.**
  - gwz-py pins `taut-proto==0.10.0`, which supports `missing_ok`. This is the packaged IR's first `missing_ok` (5 occurrences).
  - The Python layer and the extension ship in one wheel. In either mismatch direction, absent tags read `None`, and unknown tags are ignored, as the pre-C2 production IR already did with the candidate extension's tags 9–12.
  - In a candidate-and-ordinary mix in one process (the gwz-py candidate extension with the production Python IR), the five definitions are now identical. The gwz-py candidate and session suites are green.
  - The raw-tag reader in `test_client_host_transport.py` is unaffected.
- **Old reader in `test_candidate.py`.** Production minus the five equals the pre-C2 schema's IR exactly. The byte snapshots in `test_old_production_request_and_capability_snapshots_are_stable` would catch later drift.
- **Native route and the ordinary build ignore `max_retries` on network operations.** This matches the candidate's native route (the retry plan scopes retries to the transport). C2 did not change it, and it is not a finding.

## 3. Follow-ups

- **(Pre-existing, outside C2's diff, a limitation)** On HTTPS, `opened()` runs once per libgit2 request (GET info/refs, then POST), so one row keeps only the last open's receipts. A fresh first connection followed by `reused: true` on the second request is reported as `reused: true`. This should be stated in S7.5's Surface entry alongside P3-2. It is not a defect of C2.
- **(Structural debt for the operator, pre-existing)** The pre-log projection is maintained in three hand-written copies: `check_log_additive.py`, `check_protocol_drift.py` and `test_log_protocol.py`, with different strictness. A shared projection module, or having the test call the drift script's `pre_log_projection`, would remove the drift risk. This is a decision for the operator, not something to fix in C2.
- **(Nit)** `tests/protocol.rs` (new test) says "an unset budget writes null there, never omits it", but it asserts `Some(0)` encodes as `Int(0)`. The null-not-omitted property is untested in Rust.
- **(Stale text, belongs to C3/C4 per proposal finding 2)** `convert.rs:207-210` and `transport_observations.rs:71-74` still give the `needless_update` reason as "gwz_transport_candidate adds max_retries/fields". So does the `ROW_ENDPOINT/ROW_CONNECTION` comment in `test_client_host_transport.py:56-58`, which says "Candidate wire keys".

## 4. Risks and next action

- **Risk:**
  - P3-1 leaves the ordinary build with one policy field that merge neither honours nor refuses until C3b.
  - P3-2 would carry an incorrect absence rule into S7.5's Surface wording and the gwz-py migration notes if it is not corrected first.

  Neither affects wire compatibility, durability or decoding.
- **Next action:**
  1. The lane owner chooses P3-1's closure: make the merge refusal unconditional now and drop inventory rows 40–41, or record the interim gap in C2.3.
  2. Fold P3-2's corrected absence rule, and the HTTPS last-open note, into the C2.3 S7.5 Surface entry.
  3. Neither blocks C2's acceptance, so proceed to C2.3.
