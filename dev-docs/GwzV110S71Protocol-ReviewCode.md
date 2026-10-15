# S7.1 protocol fields (C2.1/C2.2) — CODE-AXIS REVIEW

**Review object:** gwz-core `158c628c1ea6fd003b568f3064024a8bd95ea60d` (one commit over `1c16373f`) and gwz-py `0c8bcc7003b837f090c851285387d89f0ada56a2`. Reference only: gwz-transport `a27817b9`, gwz-cli `8d909ec4`. Lane root `63e4076b` is out of scope. Workspace: `/Volumes/projects/limbo/gwz-dev-c21`.
**Baseline:** gwz-core `1c16373f` (pre-C2 schema sha256 `76455b40…22c5`); the released baseline is gwz-core and gwz-py `v1.0.17`.
**Date:** 2026-10-16
**Axis:** CODE. It covers interfaces, call graphs, wire compatibility with retained readers and older writers, and error paths.

**Verdict: GO.** There are 0 P0, 0 P1, 0 P2 and 2 P3 findings.

## 0. Evidence base

I checked the tuple at the start and again at the end, and it was identical both times. All four repositories are at the stated heads. The untracked files are the same as at the start: gwz-core `dev-docs/GwzRemoteTransportBugReport.md`, and five root `dev-docs/` files. I wrote nothing in the lane. Builds and venvs went under `/Volumes/projects/limbo/build-targets/reviewc2-code-*`. Python ran with `PYTHONDONTWRITEBYTECODE=1` and pytest ran with `-p no:cacheprovider`.

Commands I ran myself:

| Check | Result |
|---|---|
| `python3.13 protocol/regen.py --check` (gwz-core) | `OK -- committed protocol artifacts are current` |
| `candidate-regenerator.py --core-schema protocol/gwz.taut.py --owner-schema ../gwz-transport/protocol/transport.ir.json --check` (regen venv, taut-proto 0.10.0) | `placement candidate regeneration verified` |
| `git diff 1c16373f 158c628c -- src/protocol/candidate_generated.rs protocol/candidate/candidate_generated.py protocol/candidate/corpus` | empty, so the candidate artifacts are byte-identical |
| `shasum -a 256 protocol/gwz.taut.py` | `a7866259…107d`, equal to the moved pin in `candidate-generator.json:12` and `test_candidate_regen.py:124-126`. The pre-C2 file hashes to `76455b40…`. |
| `protocol/check_log_additive.py` | `OK sha256:4d377a49…5f14`, so the historical pin holds |
| gwz-py `scripts/check_protocol_drift.py` | OK |
| gwz-py `scripts/regen_protocol.py --check` (taut 0.10.0) | `regen_protocol: OK` |
| `cargo +1.95.0 test --test protocol`, ordinary build, external target | 44 passed, including `ordinary_operation_policy_and_observation_carry_the_1_1_0_fields` |
| Candidate tree through `prepare.py` → `build-targets/reviewc2-code-cand`, `RUSTFLAGS='--cfg gwz_transport_candidate'`, `--test protocol` | 44 passed |
| Same candidate tree, `--lib -- max_retries transport_observations` | 19 passed: the merge refusal, the transport_host retry and range rows, and the receipt `https_tests` |
| pytest `tests/transport_consumer/candidate/test_candidate.py` and `protocol/candidate/test_candidate_regen.py` (own venv with taut-proto 0.10.0) | 21 passed |
| pytest gwz-py `test_codec`, `test_log_protocol`, `test_transport_reporting`, `test_cli_push_render`, `test_cli_url_scheme` | 30 passed |
| `docs/generate_message_catalog.py` rendered to the scratchpad and compared with `cmp` | identical to the committed `docs/MessageCatalog.md` |
| Schema history scan: every one of the 55 revisions of `protocol/gwz.taut.py`, tags of `OperationPolicy` and `TransportObservation` | each tag ever had one occupant, and tags 9–12 had none before this commit |
| Pre-C2 IR (`git show 1c16373f:protocol/gwz.taut.py`) compared with test_candidate's `OLD` (production minus the five members) | `OLD == pre-C2 IR: True` |
| Released gwz-py `v1.0.17` `gwz.ir.json` used to decode all 155 vectors of the new `protocol/corpus/golden.json` | 155 decoded, 0 failed. The new tags land in `__unknown__`: `{9}` on every `meta.policy` and `{9,10,11,12}` on every `meta.transport[]` row. |
| Production struct codegen compared with candidate (`OperationPolicy` and `TransportObservation`, struct through `from_cbor`) | identical sha256 for both. The whole-file diff is 86 `>` lines, as the proposal says. |

Evidence I read but did not re-run, from `gwz-core-evidence/.../runs/2026-10-16-c21`:

- gwz-cli ordinary build: 376 passed. Candidate build: 401 passed.
- gwz-py suites: ordinary 1037 passed; candidate 1059 passed; candidate with session 1059 passed.
- Core gates r1: all exits 0 except leg 1. Leg 1 failed on a harness error, "cannot update the lock file … because --locked was passed", not on a test. It was rerun green in r2.
- Windows receipt `c21-20261016-a` is PASS. It was built from the worktree on top of `1c16373f`.

## Problem, requirement provenance, and scope

- **`OperationPolicy.max_retries` (tag 9).** This is a 1.1.0 need traced to code. It is written by gwz-cli `globalargs/retry.rs:24-26` through `clirequest/invocation.rs:129`. It is read by `transport_host/request.rs:454-486` and `session/requests.rs`. It is the S7.2 `--max-retries` feature.
- **The four `TransportObservation` receipts (tags 9–12).** They are written only by `transport_observations.rs:114-120` (`opened`) on every transport-route open. No production code reads them. They are the evidence for the pooling tests and the gwz-py route assertion. D1 = B is the lane owner's disposition, and its outcome is out of scope here.
- **Authority.** Amendment 2 line 280 says any candidate field the 1.1.0 transport needs is named in the ordinary protocol, regenerated in gwz-py and placed under S7.5. This object does the first two. Its Surface entry is C2.3.
- **Scope.** The changed files are exactly the proposal §0 and §5 C2.1/C2.2 lists. Nothing outside them changed, and gwz-transport, gwz-cli and taut are untouched. The gwz-py IR diff adds exactly five field objects. The production protocol adds nothing beyond the five fields: no placement projection, no `transport_message`, no capability extras.
- **D-dispositions as implemented:**
  - D2: `MISSING_OK`, generated as `try_get_opt`.
  - D3: the old reader is built by stripping the five members in the test, and I proved it equals the pre-C2 IR.
  - D4: gwz-py `--json` rows gain four null keys. This is asserted at `test_transport_reporting.py:43-44`.
  - D5: `client.py:338` passes `max_retries=None`, and all six test constructors were updated.

## 1. Findings

### F1 (P3): in the ordinary build, merge silently accepts `policy.max_retries`, and the comment guarding the refusal is now false

- **Root cause.** `gwz-core/src/workspace_ops/merge/validate.rs:186-195` still gates the `max_retries` refusal on `cfg(gwz_transport_candidate)`, and its comment says "Only the transport build's policy has the field". Since C2.1, the ordinary `OperationPolicy` has the field (`src/protocol/generated.rs:2423`). The test that pins the refusal (`validate.rs:537`, `max_retries_is_refused_like_every_policy_field_merge_does_not_act_on`) is gated in the same way.
- **Violated invariant.** Merge refuses every policy field it does not act on (`validate.rs:175-185`: sync, remote, concurrency, progress interval, per-host limit). The ordinary build now decodes a field that is in that class and does not refuse it.
- **Reproduction.** In an ordinary build, call merge start with `RequestMeta { policy: Some(OperationPolicy { max_retries: Some(5), ..Default::default() }), .. }`. The request validates and the field is ignored. The candidate build returns `invalid("policy.max_retries (--max-retries)")`; that candidate test passed in my run.
- **Impact.** The gap is bounded. No ordinary-build front end writes the field: gwz-cli's ordinary `RetryArgs::apply` is a no-op (`retry.rs:51`), and gwz-py always sends `None`. Only a direct Rust or raw-protocol caller sees a silent accept until C3b removes the switch. Before C2.1 the tag was simply unknown, so on the wire this is not a regression. The problem is the false comment and the missing parity.
- **Correction.** In C3b, which owns inventory rows 20 and 21, make the refusal and its test unconditional. Proposal row 20 already says "unconditional once D1 puts the field in the ordinary protocol", and that condition is now met. If the switch inventory should stay untouched until then, record the now-false comment and the ordinary-build acceptance against C3b's row list in the C2.3 checkpoint.
- **Closure test.** `max_retries_is_refused_like_every_policy_field_merge_does_not_act_on` runs and passes in the ordinary build.

### F2 (P3): MessageCatalog.md cannot tell MISSING_OK fields from required-but-nullable fields

- **Root cause.** `gwz-core/docs/generate_message_catalog.py:135` renders `"yes" if field.optional else "no"`. `MISSING_OK` and `optional=True` both come out as `yes`. C2.1 adds the first `MISSING_OK` fields to the production schema (the import at `gwz.taut.py:4` is new), so the catalog now lists two different presence contracts the same way. For example, `docs/MessageCatalog.md` shows `max_connections_per_host | 8 | int | yes` next to `max_retries | 9 | int | yes`.
- **Violated invariant.** The catalog is the mechanical description of the wire. D2's whole rationale is that the two contracts behave differently when the key is absent: `try_get` returns a `MissingKey` error, while `try_get_opt` reads absence as `None`.
- **Reproduction.** Read `docs/MessageCatalog.md` around lines 1002-1005 and 1074-1080. Then encode an `OperationPolicy` map that omits tag 8 and decode it: you get `MissingKey(8)`. Omitting tag 9 decodes to `None`. Both fields are documented as "Optional: yes".
- **Impact.** A writer or reviewer who relies on the catalog may omit a key that is required. This is a documentation defect only; generated code is correct.
- **Correction.** Render `missing_ok` distinctly, for example `yes (absent ok)`, and regenerate the catalog.
- **Closure test.** A generator assertion, or a catalog grep, showing that tags 9–12 and `max_retries` render differently from `optional=True` rows.

## 2. Invariant analysis (attacks that failed)

**Old writer to new reader.**
- Rust: the generated `from_cbor` uses `try_get_opt` for tags 9–12, so a map without those keys reads as `None`, and so does a map with null values. The ordinary test covers both cases.
- Python: taut's `_from_wire` with `strict=True` lets a `MISSING_OK` key be absent (`codec.py:182`). `test_pre_c2_bytes_decode_with_the_new_members_absent` decodes genuine 8-entry (`a8…`) pre-C2 bytes.
- test_candidate's `OLD` writer equals the pre-C2 schema exactly, which I verified.

**New writer to old reader.**
- The released Rust reader looks fields up by key (v1.0.17 `generated.rs:2395-2404`, `cbor.rs` `try_get`) and never rejects an unknown key.
- Released gwz-py 1.0.17 iterates only its own fields (`codec.py:246-250`), so `__unknown__` is dropped. All 155 new corpus vectors decode under the 1.0.17 IR.
- The retained Python reader at taut `733e8a78` reports `__unknown__ == {9..12}` and `{9}` (`test_candidate.py`).
- The Rust consumer-proof crate could not run offline, because the `gwz-transport =0.1.0` registry package is not cached. Its inputs (`candidate_generated.rs`, `retained_old_generated.rs`) are byte-identical, and the two structs match production exactly, so its result cannot have changed.

**Candidate shape against ordinary shape.**
- `FieldDef` equality is asserted in `test_production_carries_the_1_1_0_fields_as_missing_ok`.
- The struct codegen hashes are identical.
- The candidate is regenerated byte-identical, and `_add_fields` would have skipped identical fields anyway.

**Malformed values are rejected.**
- Rust: text at tag 9 gives `Err` (ordinary test).
- Python: `"seven"` at tag 11 gives `DecodeError` (candidate test).
- The generated `try_int`, `try_text` and `try_bool` checks are uniform across all five fields.

**Tag reuse.** Tag 9 is used in two different messages, which is legal because tags are scoped to a message. Within each message, tags 1–12 are unique. The history scan shows tags 9–12 were never occupied before, and neither message has a reserved or retired tag.

**Projections strip exactly five members.** `check_log_additive.py:289-315` and `check_protocol_drift.py` C2_ADDITIONS each require the exact field object (name, tag, scalar, `missing_ok`, not transient, no merge) before removing it. A sixth field, a wrong tag or a wrong kind would raise or break the pin. The gwz-py `test_log_protocol.py:370-386` copy checks only tag and presence, not scalar kind. That is weaker, but the drift script catches a kind change, so it has no concrete consequence.

**gwz-cli JSON is unchanged.** gwz-cli is unchanged at `8d909ec4`. `transport_observation_json` (`response_meta_json.rs:16-43`) builds a fixed `json!` literal of eight keys, and no other emitter serializes generated structs (no serde derives in `generated.rs`). Its evidence run passed 376 ordinary and 401 candidate tests.

**gwz-py `--json` (D4).** `dataclasses.asdict` (`cli_render_parts/machine.py:13`) adds four keys, which are null off the transport route. This is asserted. No row dedupe or equality logic consumes the rows.

**gwz-py API break (D5).** Keyword or positional constructors without the new arguments raise `TypeError`. Every in-tree constructor is updated: `client.py:329` and six test sites, found by grep across `.py` and `.md`. `_merge_policy` uses `fields(policy)`, so `max_retries` is covered.

**Rust struct literals.** Every in-tree literal uses `..Default::default()` and compiles in both builds (gwz-core here; gwz-cli and gwz-py in the evidence). The `needless_update` attributes are `allow`, not `expect`, so no unfulfilled-lint failure can appear.

**Ordinary-build writers.**
- The ordinary build always writes tags 9–12 as null; the corpus diff is `09 f6` per policy plus four entries per observation.
- Nothing in the ordinary build writes a non-null value. `opened()` is candidate-only (`transport_observations.rs:112-120`), the gwz-cli ordinary `apply` is a no-op, and gwz-py sends `None`.
- The meaning matches the candidate field for field.

**gwz-py candidate route assertion.** `test_client_host_transport.py` reads raw CBOR keys 9 and 10, which are unaffected by the new named fields.

**Durable or cross-process wire.** No non-generated code persists `OperationPolicy`, `TransportObservation`, `RequestMeta` or `ResponseMeta`. The decodes in `client_host.rs:205` and `dispatch/mod.rs:26` are in-process, within one wheel.

## 3. Follow-ups (outside the object; none blocks)

1. **Stale "candidate adds" text, now false for these two messages:**
   - `src/protocol/convert.rs:209` ("gwz_transport_candidate adds max_retries")
   - `transport_observations.rs:73`
   - gwz-py `test_client_host_transport.py:56-57` ("Candidate wire keys")
   - `tests/transport_backend/prepare.py:13-14` ("step C2 … retires the pair"), which contradicts D3

   The first two are already in proposal finding 2 for C3/C4. The other two should be added to that list.
2. **No gwz-cli test pins the exact key set of a `meta.transport` row.** The fixed literal is correct today, but nothing would catch a future addition there. Consider an exact-keys assertion in `src/tests/g02.rs:609` when C5 touches MachineOutput.md.

## 4. Risks and next action

- **Residual risk.** It is low. The only cross-version wire paths are theoretical, because the extension and Python ship in one wheel and gwz-cli runs in process, and every direction was attacked and holds.
- **Next actions:**
  - Close F1 in C3b, or record it in the C2.3 checkpoint against rows 20 and 21.
  - Fix F2 in the catalog generator at any convenient step before S7.5's Surface review.
  - C2.3 records the five fields, the D4 JSON effect and the D5 constructor effect under S7.5's Surface.
