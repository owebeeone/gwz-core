# S7.1 (1.1.0), step C2: the protocol pair, and how C3 and C4 follow from it — proposal (DRAFT)

Date: 2026-10-16. Status: **draft analysis, uncommitted; no code was changed.** Author: analyst for lane `c2protocol`, at gwz-core `0ba40f63`. Nothing here is decided: §6 lists what needs the operator or a review.

Controlling text: [Amendment 2](GwzTransportReleasePlanAmendment-2.md) lines 276–284 (S7.1 (1.1.0)), [the 1.1.0 amendment](GwzV110PlanAmendment.md) lines 112–120, [the 1.1.0 plan](GwzV110Plan.md) lines 308–315, and [the remaining plan](GwzV110RemainingPlan.md) lines 77–81 (C2 to C4).

## 0. Scope of this document

- **Deliverable:** an analysis and a proposed step plan for C2, with C3 and C4 re-cut as §5 proposes. No implementation.
- **Components a later implementation of C2 may change:** gwz-core (`protocol/gwz.taut.py`, `src/protocol/generated.rs`, `protocol/corpus/`, `protocol/check_log_additive.py`, `protocol/candidate/`, `tests/transport_consumer/candidate/`, `docs/MessageCatalog.md`), gwz-py (`src/gwz/protocol/generated/`, `scripts/check_protocol_drift.py`, and the Python call sites and tests in §5).
- **Reference only:** gwz-transport (its owner schema `protocol/transport.ir.json` and `transport.taut.py` do not change), gwz-sspi, taut, the rest of gwz-cli.
- **Evidence method.** Everything below is from `git grep` and file reads in the lane. I also ran four scratch experiments in the scratchpad (never in the lane): (1) the production schema with the five fields added, regenerated to Rust, to Python and to a corpus; (2) the candidate regenerator against that schema; (3) gwz-py's `check_protocol_drift.py` and gwz-core's `check_log_additive.py` against it; (4) `candidate-regenerator.py --check` and `protocol/regen.py --check` on the unchanged lane, both green. The lane's `.regen-venv` has taut-proto 0.10.0, so the regeneration route works here.

## 1. Every `gwz_transport_candidate` site, by class

Classes: **(a)** the 1.1.0 transport, made unconditional at C3/C4; **(b)** 1.2.0-gating, moves to `gwz_session_candidate`; **(c)** tooling, CI, harness, text.

### 1.1 Counts (lines matching `git grep -n gwz_transport_candidate`, `dev-docs` excluded)

| | gwz-core | gwz-cli | gwz-py | total |
|---|---|---|---|---|
| matching lines | 143 | 54 | 33 | 230 |
| inventory file (header text and rows) | 25 | 21 | 7 | 53 |
| **cfg sites (the inventory rows)** | **23** | **19** | **5** | **47** |
| of the cfg sites: **(a)** | 15 | 19 | 3 | **37** |
| of the cfg sites: **(b)** | 8 | 0 | 2 | **10** |
| `Cargo.toml` target tables and their comments (a, manifest) | 8 | 0 | 0 | 8 |
| `check-cfg` declarations (c) | 1 (`build.rs:8`) | 1 (`build.rs:6`) | 1 (`Cargo.toml:15`) | 3 |
| lint `reason = "gwz_transport_candidate adds ..."` strings (c, text) | 21 | 10 | 0 | 31 |
| comments in source (c, text) | 2 | 0 | 0 | 2 |
| CI workflows and checkout scripts (c) | 21 | 2 | 8 | 31 |
| other scripts, harness tests, READMEs (c) | 42 | 1 | 12 | 55 |

`dev-docs` holds 138 more matches in gwz-core and 1 in gwz-py. The completeness rule excludes `dev-docs`, so they are history, not work. The inventory rows equal the checker's `--list` output in all three repositories (`check_candidate_switches.py` passes on gwz-core, 23 sites).

**The audit's finding holds, and the reason is structural.** None of the 1.2.0 surfaces named in the amendment's list exists as code yet, except the placement projection and the Cli-placement entry:

- `rg SocketCoreBridge|"server"|reuse` finds nothing in gwz-core, gwz-cli or gwz-py source. There is no `server` command and no `--server` option in gwz-cli (`git grep -n` of its `src` for `server` finds only the unrelated hook code).
- `src/session_host` (gwz-core) has no cfg at all and says "Frozen interfaces; nothing calls them yet" (`src/lib.rs:62-68`; the module is declared at `:68`). Of it, 1.1.0 uses only `session_host::EnvironmentSnapshot` (gwz-cli `src/lib.rs:197`, `src/globalargs/transport.rs:172`, and gwz-core's `transport_host`). That is not a switch site.
- The "session host's transport entry and its arms" is, in code, the **Cli placement**: `TransportRuntime::install_cli`, `capabilities`, `CliEndpoint`, `require_cli_ssh`, and the `is_cli` arms. They sit inside `transport_host` with no cfg of their own, so a checker that lists cfg attributes cannot list them. Inventory rows for them can exist only after C3a adds `gwz_session_candidate` to them (§5).

### 1.2 The 23 cfg sites in gwz-core

| # | File:line | Symbol | Class | Why |
|---|---|---|---|---|
| 1 | `src/git/gitbackend.rs:46` | `mod transport_candidate_tests` | (a) test | 1.1.0 transport tests. `transport_candidate_tests/home_rows.rs:37` sets `placement: Some(Local)`; that line is dropped (None means Local). |
| 2 | `src/git/gitbackend.rs:67` | `use transport_support::server_timeout_ms as transport_timeout_ms` | (a) | `SshEndpointConfig::from_environment` (`transport_host/mod.rs:170`). |
| 3 | `src/git/gitbackend.rs:226` | `fn with_transport` | **(b)** | Refuses `placement == Cli` without a Cli host context. Cli placement is 1.2.0's. |
| 4 | `src/git/gitbackend.rs:241` | `fn with_transport` | **(b)** | The `is_cli_context()` identity arm (`from_options_cli`, `validate_endpoint_files`). The `else` arm (`from_options(start, ssh_home)`) is the 1.1.0 path. |
| 5 | `src/git/gitbackend/backend.rs:84` | `fn with_host_context` | (a) | How the 1.1.0 entry attaches a `RequestContext` (TR2.11). |
| 6 | `src/git/gitbackend/transport_binding.rs:9` | `impl Runtime` | (a), embeds (b) | The whole binding. Inside it, `is_cli_context` (`:56`, stub at `:323`) and `cli_hint: self.context.is_cli()` (`:104`) are Cli arms. |
| 7 | `src/git/gitbackend/transport_binding.rs:351` | `mod https_transport_binding_tests` | (a) test | |
| 8 | `src/git/gitbackend/transport_observations.rs:107` | `fn failed` | (a) | |
| 9 | `src/git/gitbackend/transport_observations.rs:113` | `fn opened` | (a) | Writes the four receipt fields (`:116-119`); see D1. |
| 10 | `src/git/gitbackend/transport_observations.rs:197` | `mod https_tests` | (a) test | |
| 11 | `src/git/gitbackend/transport_support.rs:283` | `fn server_timeout_ms` | (a) | |
| 12 | `src/git/gitbackend/transport_support/identity.rs:86` | `fn from_options_cli` | **(b)** | "Candidate CLI placement keeps identity paths opaque to core." |
| 13 | `src/git/gitbackend/transport_support/identity.rs:136` | `fn validate_endpoint_files` | **(b)** | "CLI placement validates every invocation supplied identity." |
| 14 | `src/git/mod.rs:5` | `mod endpoint` | (a) | The endpoint tree. `placement_endpoint.rs` is the mux endpoint of the in-process Local route (`Session::endpoint`), not the Cli placement; the name only collides with the projection. |
| 15 | `src/lib.rs:106` | `mod protocol_corpus` | **(b)** | Picks the candidate's corpus (`protocol/candidate/corpus`) in `test`. A "cfg site that selects the candidate protocol". |
| 16 | `src/lib.rs:57` | `mod transport_host` | (a), embeds (b) | The Local runtime, `with_local_transport*`, `with_cancellable_local_transport*`. The Cli placement arms are inside it (§1.4). |
| 17 | `src/lib.rs:13` | `use gwz_transport::cbor` | **(b)** | Selects the candidate protocol's Cbor type. Needed only so `RequestMeta.transport_message: Envelope` shares `Cbor` with the transport crate. Nothing else mixes the two (§2.5). |
| 18 | `src/lib.rs:81` | `mod transport_setting` | (a) | The off switch. |
| 19 | `src/protocol/mod.rs:5` | `mod generated` | **(b)** | `#[path = "candidate_generated.rs"]`. The amendment names it (`protocol/mod.rs:5-9`). |
| 20 | `src/workspace_ops/merge/validate.rs:188` | `fn validate_common_meta` | (a) | Merge refuses `policy.max_retries`; unconditional once D1 puts the field in the ordinary protocol. |
| 21 | `src/workspace_ops/merge/validate.rs:532` | its test | (a) test | |
| 22 | `src/workspace_ops/tests/g00/private_members.rs:356` | `private_materialize_...` | (a) test | Sets `placement: Some(Local)` at `:382`; drop the line. |
| 23 | `tests/protocol.rs:511` | `merge_request_and_response_round_trip_...` | **(b)** | The expected wire hex differs only by `RequestMeta.transport_message` (`aa` against `a9`). The ordinary arm already equals the post-C2 production bytes (policy is null there). |

### 1.3 gwz-cli (19 sites, all (a)) and gwz-py (5 sites)

gwz-cli, all (a), because every site is the in-process CLI transport, `--max-retries` or `--transport` (the off switch):

- `src/globalargs.rs:10`, `:22`, and `src/globalargs/dispatch.rs:4`, `:11`, `:57`, `:438`: the in-process entry, `note_sink`, `transport_meta`.
- `src/globalargs/retry.rs:8`, `:63`: `RetryArgs` and its pins. `apply` sets `policy.max_retries` (`retry.rs:24-26`); `invocation.rs:129` calls it.
- `src/globalargs/transport.rs:5` (`TransportArgs`), `transport_help.rs:3` (`JOBS_HELP`).
- `src/lib.rs:180`, `:193`, `:196`, `:303`, `:318`, `:330` (`fn run`, six arms).
- Tests: `src/tests/g00.rs:39`, `src/tests/g09.rs:232`, `tests/transport_workflows.rs:21`.
- Four of these are *inverted* arms, `not(all(candidate, any(unix, windows)))` (`globalargs.rs:10`, `dispatch.rs:4`, `g00.rs:39`, `g09.rs:232`). They are the native branch the plan keeps "for the paths 1.1.0 does not support". `execute_invocation` keeps its `not(any(unix, windows))` arm. `g09.rs:232` tests the ordinary build's help and goes. `g00.rs:39` (the `docs/CLI.md` doc test) becomes unconditional, which makes `docs/CLI.md` regenerate with `--max-retries`; that is C5's doc work, but C4's test run depends on it.

gwz-py: **(a) 3**: `native/src/client_host.rs:172` and `:182` (`fn network`, twice), `native/src/route.rs:18` (`mod transport`/`mod native`). **(b) 2**: `native/src/dispatch/mod.rs:468` (`submit_accepted`) and `native/src/dispatch/merge.rs:107` (`accepted_response`). The two (b) arms differ only by `transport_message: None` in a `ResponseMeta` literal. These are the amendment's "gwz-py's two schema arms".

### 1.4 Cli-placement code that is (b) but carries no cfg today

These move under `gwz_session_candidate` in C3a, and are the real "session host's transport entry and its arms". All paths are gwz-core `src/`.

- `transport_host/mod.rs`: `RuntimeState.cli` (`:251`, `:343`), `install_cli` (`:364`), `capabilities` (`:377`), the `is_cli` branch of `open_request` (`:438-476`), the `cli` handling in `close` (`:532` onwards), `CliEndpoint` (`:548`), `require_cli_ssh` (`:567`), and the `TransportPlacement` import (`:83`).
- `transport_host/request.rs`: `is_cli` (`:74`, `:441`), `validate_meta`'s `transport_message` and `endpoint_path_base` rules (`:445`, `:471-476`), `identity()`'s `path_base` (`:337-342`), and `cli_hint: self.is_cli()` (`:137`, `:187`, `:317`).
- `git/gitbackend/transport_support/identity.rs`: the `is_cli_context()` uses at `:227`, `:230`, `:263`, `:286`, `:303`.
- Public API used only by the placement guide: `docs/TransportPlacement.md` (405 lines), `tests/transport_backend/guide_example.rs`, `guide_test.rs`, and the `transport_placement_guide` target `prepare.py:84-86` adds.
- Tests that name the projection: `transport_host/tests.rs` (31 lines), `message_embedding_tests.rs` (10), `driver_tests.rs` (7), `qualification_tests.rs` (6), `retry_tests.rs`, `https_tests.rs`, `endpoint_environment_tests.rs`, `cleanup_tests.rs`, `agent_source_tests.rs`. Those that set `placement: Some(Local)` simply drop it; those that exercise `Cli`, `transport_message` or the five capability extras are session tests.

### 1.5 Unclear or worth an explicit ruling

- **U1.** `mod transport_host` is one site but two classes. The rule I propose: *an item is (b) iff it names `TransportPlacement::Cli`, `transport_message`, `endpoint_path_base`, or the five `TransportCapabilitiesResponse` extras, or exists only to serve them.* Everything else in the module stays (a).
- **U2.** `docs/TransportPlacement.md` and its guide test document the Cli placement. `prepare.py:6-12` says "step C3 turns the guide into an ordinary test". That cannot hold once the guide's types are session-only; it becomes a session test (D6).
- **U3.** `TransportPort` and `Attachment` are shared by the Local link (`session/local_link.rs:12`), so they stay (a) although `install_cli` returns them.
- **U4.** `Cargo.toml`'s six target tables are (a) in substance but are *manifest* edits, not cfg sites; C3b turns them into plain `[dependencies]` entries and is a Windows trigger path.

## 2. The protocol pair

### 2.1 Where the pair comes from

- Production schema: `gwz-core/protocol/gwz.taut.py`, digest `76455b40...22c5` (pinned as `current-core-schema-sha256` in `protocol/candidate/candidate-generator.json`).
- Owner schema: `gwz-transport/protocol/transport.ir.json`, digest `48837b7e...1e1`. It is composed whole; it does not change.
- `protocol/candidate/candidate.taut.py` composes the two, then adds the overlay with `_add_fields` (it skips a field already present and identical, `:41-45`). `candidate-regenerator.py` emits `src/protocol/candidate_generated.rs` (owner types become `pub use gwz_transport::protocol::X`, 42 of them), `protocol/candidate/candidate_generated.py`, and the candidate corpus. Production is `protocol/regen.py` (`tautc gen`), which also writes `src/cbor.rs`.
- `tests/transport_consumer/protocol/` (consumer.taut.py, generator.json) is a test-only crate proving the owner schema; it is not in the diff below and does not change. `tests/transport_consumer/candidate/` holds the retained old readers (Rust at revision `2514dc19`, Python taut `733e8a78`) and `test_candidate.py`, which *does* depend on the production schema (§2.6).

### 2.2 The difference: 101 added lines, 0 removed, in seven places

`diff generated.rs candidate_generated.rs` is 101 `>` lines. By type:

| Type | Candidate adds | Lines |
|---|---|---|
| (re-exports) | 42 `pub use gwz_transport::protocol::X as X` | 42 |
| `TransportPlacement` | the enum `{Local = 1, Cli = 2}` | 17 |
| `OperationPolicy` | `max_retries: Option<i64>`, tag 9 | 3 |
| `TransportObservation` | `endpoint_id` 9, `connection_id` 10, `stream_id` 11, `reused` 12 | 12 |
| `TransportCapabilitiesResponse` | `message_versions` 3, `placements` 4, `schemes` 5, `auth_policies` 6, `message_limits` 7 | 15 |
| `TransportOptions` | `placement` 4, `endpoint_path_base` 5 | 6 |
| `RequestMeta` / `ResponseMeta` | `transport_message: Option<Envelope>`, tags 10 / 9 | 3 + 3 |

All overlay fields are `MISSING_OK` (`try_get_opt`), where production's house style, `optional=True`, generates `try_get` and so *rejects a map that lacks the key*. This matters in §2.4.

### 2.3 Which differences does the 1.1.0 transport use at runtime?

"Needed to function" and "needed as evidence" differ, so both are shown.

| Field | Written at runtime | Read at runtime | Other readers | Class |
|---|---|---|---|---|
| `OperationPolicy.max_retries` | gwz-cli `RetryArgs::apply` (`globalargs/retry.rs:24-26`, via `clirequest/invocation.rs:129`). **gwz-py never writes it**: `client.py:329` builds the policy without it, and `test_client_host_transport.py:427` says "gwz-py has no `--max-retries` in 1.1.0". | `transport_host/request.rs:454-486` (range checks, `max_retries(&meta)`), `request.rs:44`, `mod.rs:479-496` (`endpoint.set_max_retries`), `session/requests.rs:15-24`; merge refuses it (`validate.rs:188-195`). `protocol/convert.rs:206-225` leaves it to `Default`. | `transport_host/tests.rs:42-57` | **1.1.0 needs it.** It is the S7.2 `--max-retries` feature. |
| `TransportObservation.endpoint_id`, `.connection_id`, `.stream_id`, `.reused` | `transport_observations.rs:113-120` (`opened`), called from `transport_binding.rs:252` (SSH) and `:291` (HTTPS) on every transport-route open. | **No production reader.** gwz-cli's `response_meta_json.rs:17-79` prints a fixed list that omits them. | Core unit tests asserting pooling (`driver_tests.rs:110`, `:606`, `ssh_helper_projection_tests/enablement.rs:143`, `url_password.rs:257`); gwz-py's route assertion reads raw tags 9 and 10 (`test_client_host_transport.py:60`, `:135`, asserted at `:151`, `:167`). | **Needed as evidence**, not to function. See D1. |
| `TransportOptions.placement`, `.endpoint_path_base` | Not written by gwz-cli (`invocation.rs:62-75` leaves them to `Default`) or gwz-py. Tests only (`Some(Local)` or `Some(Cli)`). | `request.rs:341`, `:442`, `:472`; `gitbackend.rs:228`. All Cli-placement logic; `None` means Local. | | **Projection (1.2.0).** |
| `RequestMeta.transport_message`, `ResponseMeta.transport_message` | gwz-py writes `None` in two literals (`dispatch/mod.rs:470`, `dispatch/merge.rs:109`); core tests embed Envelopes. | `request.rs:445` only *refuses* it. | `message_embedding_tests.rs` | **Projection (1.2.0).** |
| `TransportCapabilitiesResponse` five extras | Only `TransportRuntime::capabilities` (`mod.rs:377-402`), which has no production caller (§1.4). The static `transport_capabilities::handle` (`protocol/transport_capabilities.rs:8-20`) answers gwz-py and fills just `file_identity` and `exact_agent_identity`. | `require_cli_ssh` (`mod.rs:567`), also uncalled. | | **Projection (1.2.0).** S7.3 requires "`transport_capabilities` reports no session route". |
| 42 owner re-exports | | Nothing outside the candidate fields above names them through `crate::`. `git grep` over gwz-core, gwz-cli and gwz-py finds no `gwz_core::<owner type>`. | | **Projection.** They exist only to name the types the overlay fields use. |

### 2.4 What 1.1.0 needs in the ordinary protocol

- **Certain:** `OperationPolicy.max_retries` (tag 9, `MISSING_OK`).
- **Recommended (D1):** the four `TransportObservation` receipt fields, tags 9 to 12, `MISSING_OK`.
- **Stays candidate:** `TransportPlacement`, `TransportOptions.placement` and `.endpoint_path_base`, both `transport_message` fields, the five capability extras, and the 42 re-exports.

**MISSING_OK is the contract to use (D2).** I generated the production schema with the fields as plain `optional=True` and with `MISSING_OK`. With `optional=True`, `from_cbor` calls `c.try_get(9)?`, so an older writer's map, which lacks tag 9, fails with `MissingKey`. With `MISSING_OK` it calls `try_get_opt`, and the Rust output for the five fields is **exactly** the candidate's 15 lines (`generated.rs` +15 lines, nothing else, `cbor.rs` unchanged). The Python decoder treats an absent `MISSING_OK` key as null and an unknown tag as `__unknown__` (taut `wire/codec.py:172-193`). gwz-core already depends on this reading in the candidate (`candidate_compat.rs:46-135`, `test_candidate.py:126-200`). The cost is one import in `gwz.taut.py`: `from taut.ir.model import MISSING_OK` (the DSL's `F` accepts it, `dsl.py:145-170`; the candidate uses it with the same pinned 0.10.0).

Option A (smaller, D1): `max_retries` only. The receipt fields stay candidate. Then the unit tests above that assert `reused` and `connection_id` become session tests, or must count at the fixture instead, and gwz-py's route assertion (S6.3, S7.3 "asserting the transport route through its observations") needs another discriminator. The existing fields cannot tell the routes apart: the native route also records `credential_method`. Amendment 2 line 265 says "the transport rows do not show connections", which is true of gwz-cli's output under either option (D6 covers gwz-py's JSON).

### 2.5 `src/lib.rs` cbor selection

`gwz-core/src/cbor.rs` and `gwz-transport/src/cbor.rs` are the same taut 0.10.0 runtime, differing only by rustfmt (diff hunks at core `:243-247`, `:415-435`, `:541`). The candidate takes the transport's so `RequestMeta.transport_message: Envelope` has one `Cbor` type. In the 1.1.0 build nothing converts between them: `Cbor` appears under `src/transport_host`, `src/git/endpoint` and `src/git/gitbackend` only in `message_embedding_tests.rs` and `tests.rs:43`. So the selection is a pure projection site (#17) and the ordinary build keeps its own `cbor.rs`.

### 2.6 What the regeneration would be (measured in scratch)

| Artifact | Change |
|---|---|
| `protocol/gwz.taut.py` | +1 import, +1 field (`max_retries`), +4 fields (receipts) |
| `src/protocol/generated.rs` | +15 lines, exactly the candidate's lines for those five fields |
| `protocol/corpus/golden.json`, `rust/vectors.rs` | 136 changed lines each (every request vector's policy map grows by `09 f6`; the observation vector by four entries) |
| `docs/MessageCatalog.md` | regenerated by `docs/generate_message_catalog.py` |
| `protocol/candidate/candidate_generated.rs`, `.py`, corpus | **byte-identical** to today's after regeneration against the new production schema. Only the `current-core-schema-sha256` pin moves. After C2 the candidate differs from production by 86 lines: the placement projection and the 42 re-exports. |
| `protocol/candidate/candidate.taut.py` | the five now-redundant `_add_fields` entries are dead (the helper skips identical fields); remove them |
| `protocol/check_log_additive.py`, gwz-py `scripts/check_protocol_drift.py` | both currently **fail** against the new schema: `expected 4d377a49...` and `actual cbfdfbcb...`. Convention (every earlier additive change): extend `pre_log_projection` to strip exactly the new members and keep the pin, with a "MEASURED additive" comment. |
| `tests/transport_consumer/candidate/test_candidate.py` | `OLD = load_schema(CORE_SCHEMA)` (`:39`) *is the current production schema*, so after C2 it is no longer the old reader. Its `expected` overlay (`:81-97`) loses the `OperationPolicy` and `TransportObservation` entries, and its old-wire snapshots (`:126-133`) and `test_max_retries_crosses_the_retained_python_reader_and_absent_or_null_is_none` (`:193`) need a retained pre-C2 schema (see D3) |
| gwz-py `src/gwz/protocol/generated/api.py` | +5 lines (`max_retries: int | None`, four receipt fields) |
| gwz-py `gwz.ir.json` | about 60 added lines |
| gwz-py Python call sites | `client.py:329` (`OperationPolicy(...)`) gains `max_retries=None`; `tests/test_codec.py:38` and the five `TransportObservation(...)` test constructors (`test_cli_push_render.py:134`, `test_cli_url_scheme.py:164`, `test_transport_reporting.py:13`, `:32`, `:52`) gain the four receipt fields. The generated dataclasses have no defaults. |

The "one generated module" of C2's line in the remaining plan is **not reachable** while the placement projection stays candidate: the second module remains, with 86 differing lines, until S7.1 (1.2.0) removes `gwz_session_candidate`. See D3.

## 3. Compatibility

- **Rust, in process, either direction.** Evidence is already in the tree. A candidate writer's tags are unknown to the retained old reader and ignored (`candidate_compat.rs:46-56`, `:95-114`); an older writer's map without them reads as `None`, and null reads the same (`:57-93`, `:115-135`). The same holds for `max_retries` (`:95-135`) and in Python (`test_candidate.py:126-200`). A present but malformed value is an error. With D2 (`MISSING_OK`) the ordinary build gets the same behaviour; with `optional=True` it would not.
- **Older gwz-py talking to a newer core.** gwz-py ships its Python package and the extension (`gwz._gwz_core`, `pyproject.toml:42`) in one wheel built from one gwz-core path (`Cargo.toml:20`); there is no runtime version check between them, so a mix is a developer error. If one occurs: an older Python IR sends no tag 9 and the new core reads `None` (the default budget of 3); a newer Python sends tag 9 to an older core, which ignores it, so a budget would be dropped silently. gwz-py cannot set `max_retries` in 1.1.0 in any case.
- **Newer gwz-py, older core's responses.** Rows without tags 9 to 12 decode with the four fields `None`, so the Python route assertion would report "no receipt".
- **Rust library users.** The generated structs gain pub fields. A downstream struct literal without `..Default::default()` stops compiling. Every in-tree literal of `OperationPolicy` and `TransportObservation` already ends in `..Default::default()` (they compile in both builds today), so nothing in the three repositories breaks. Python callers constructing `OperationPolicy` or `TransportObservation` by keyword without the new arguments get `TypeError`. This is the shape every earlier additive change had (`url_scheme`, `invocation`). It is a Surface item (D5).
- **Machine-output JSON.** gwz-cli builds its transport rows from a fixed field list (`response_meta_json.rs:17-79`), so its JSON is unchanged, and `docs/MachineOutput.md:295-303` stays true. **gwz-py is different**: `cli_render_parts/machine.py:11-13` uses `dataclasses.asdict`, so with D1 every `meta.transport` row in its `--json` output gains `endpoint_id`, `connection_id`, `stream_id` and `reused` (null on the native route). With option A there is no change. This is the one user-visible effect of D1 (D4).
- **Is any GWZ wire visible across processes in 1.1.0? No.** The CLI runs the driver and the endpoint in one process (`local_command.rs`, `session/local_link.rs`); gwz-py is Python and the extension in one process (CBOR bytes over the pyo3 boundary); the SSH and HTTPS traffic is git's, not GWZ messages. The one separate process, the SSPI worker, speaks its own protocol (`gwz-sspi` has no reference to `gwz_core`). Nothing durable carries `RequestMeta`, `OperationPolicy` or `TransportObservation`: a search of `src` for decoding or encoding them outside generated code finds only `transport_host/tests.rs:48-56`. The cross-process wire of the Cli placement is 1.2.0's, and it stays under `gwz_session_candidate`.

## 4. Findings outside the scope of C2 (do not block it)

Recorded separately as the standing rule asks. None is implemented here.

1. **`cli_hint` is probably wrong for the in-process CLI** (hypothesis; read from code, not run). `transport_host/request.rs:137`, `:187`, `:317` and `transport_binding.rs:104` set `cli_hint` from `is_cli(meta)`, which is true only for `placement == Cli`. gwz-cli never sets a placement (`invocation.rs:62-75`), so its missing-git message names `GWZ_TRANSPORT=native` instead of `--transport native` (`session/driver/native_hint.rs:8-18`). Impact: a hint worded for the wrong surface. Related: three other sites hard-code `cli_hint: true` (`https_remote.rs:97`, `host_tunnel.rs:62`). Removing `placement` in C3a forces a choice at these four lines; I propose keeping today's behaviour (`false`) and filing this. Needs a runtime check first.
2. **The `needless_update` allow strings** (31 in gwz-core and gwz-cli) say "gwz_transport_candidate adds fields". After C2, those serving `OperationPolicy` and `TransportObservation` are obsolete and should be removed (`..Default::default()` is then needed in both builds); those serving `ResponseMeta`, `RequestMeta`, `TransportOptions` and `TransportCapabilitiesResponse` stay with the text `gwz_session_candidate`. Confirmed by reading `transport_observations.rs:70-92`, `protocol/convert.rs:206-225`, `handle_diff.rs:600-615`. It is part of C3/C4's completeness rule rather than a defect.
3. **`candidate-generator.json` has `retained-old-schema-sha256` `423cb73b...` with no file behind it.** `test_candidate_regen.py:117` compares the string to itself. The retained old *reader* is real (`retained_old_generated.rs`, digest pinned); the retained old *schema* is not checked in. This is why D3 is needed.
4. **gwz-core's published manifest.** `gwz-transport` and `gwz-sspi` are `path` dependencies without a version (`Cargo.toml:125-128`). Making them ordinary dependencies at C3b makes `cargo publish` of gwz-core depend on their release. Reference only here; TR3 owns publication.
5. **`docs/TransportPlacement.md`** (405 lines) describes an API that only the candidate build has. It must not ship as 1.1.0 documentation (D6).

## 5. Proposed plan

Each step is one goal, aspirational under 500 lines of hand-written change; generated output is counted separately. Red first means: write the test, run it, show it fails for the expected reason, then change the code. Steps C2.1 then C2.2 are sequential (gwz-py regenerates from gwz-core's schema); the rest are described after them.

### Phase C-protocol (milestone: the ordinary protocol carries what 1.1.0 needs)

**C2.1: gwz-core, name the fields and regenerate** *(hand about 140 lines; generated about 290)*

- Red tests, in this order:
  1. `tests/protocol.rs`, new `ordinary_operation_policy_and_observation_carry_the_1_1_0_fields`, built with **no** cfg: constructs `OperationPolicy { max_retries: Some(0), .. }` and a `TransportObservation` with the four receipts, round-trips both, decodes a map without tags 9 to 12 (and with them null) to `None`, and rejects a text value at tag 9. Fails to compile today (`cargo test --test protocol`, ordinary build).
  2. `protocol/candidate/test_candidate_regen.py::test_retained_reader_is_pinned_checked_in_baseline`: change it first to expect the *new* digest. Fails until the pin moves.
  3. `tests/transport_consumer/candidate/test_candidate.py`: remove the two entries from `expected`, and point `OLD` at the retained pre-C2 schema (D3). Fails today because `expected` names fields production already has.
  4. `protocol/check_log_additive.py` run against the new schema: red with the pin mismatch shown in §2.6.
- Change: `gwz.taut.py` (the five fields, `MISSING_OK`); regenerate with `python protocol/regen.py` and `docs/generate_message_catalog.py`; `check_log_additive.py` projection strips the five members, comment "MEASURED additive"; `candidate-generator.json` pin; `candidate.taut.py` loses the five entries; regenerate the candidate (expected byte-identical) and run `--check`.
- Green: the four tests; `tests/protocol.rs::generated_protocol_is_current`; `protocol/regen.py --check`; `candidate-regenerator.py --check`; the gwz-core gate (`PYTHON=python3.13 python3.13 scripts/run_tests.py`); `scripts/checks/`; both builds (`cargo check --tests` ordinary and with `--cfg gwz_transport_candidate`). Not a Windows trigger path (none of these files is in `windows_lane_check`'s patterns).
- **Review:** wire-format change, so a dual review (Code and State, two reviewers, no third) on this step's object, ahead of any further protocol step.

**C2.2: gwz-py, regenerate and adapt** *(hand about 90 lines; generated about 65)*

- Red tests: `src/tests/test_codec.py` gains a round trip of `OperationPolicy(..., max_retries=0)` and of a `TransportObservation` with receipts, plus a decode of the *pre-C2 golden bytes* of both (absent keys decode to `None`); `scripts/check_protocol_drift.py` against the new core schema fails with the pin message.
- Change: `scripts/regen_protocol.py` (writes `api.py` +5, `gwz.ir.json` about +60); `check_protocol_drift.py` projection strips the same members; `src/tests/test_log_protocol.py` if it holds a copy of the projection; `client.py:329` and the six test constructors (§2.6).
- Green: `regen_protocol.py --check`, `check_protocol_drift.py`, `run_tests.py` (Python suite), and, since the extension is rebuilt from the new core, the candidate extension suite.
- Review: with C2.1 as one object (the wire change spans both).

**C2.3: S7.5 Surface entry and the checkpoint** *(documents, under 60 lines)*: name the five fields in the S7.1 record and in S7.5's Surface list ("any ordinary-protocol field S7.1 (1.1.0) names"), including the gwz-py JSON effect and the dataclass-constructor effect. Records the digests.

### Phase C-switch (C3 and C4, re-cut)

The remaining plan's C3 (gwz-core) and C4 (cli and py) mix two kinds of change. Moving (b) sites is a behavioural cut with wire consequences; removing the switch from (a) sites should be mechanical. Separating them lets the first be reviewed and the second be checked by `rg`. Proposed:

**C3a: gwz-core, move the (b) sites to `gwz_session_candidate`** *(about 450 lines of hand change, mostly cfg and moves)*

- Red tests first (each fails before, in the stated build):
  1. Inventory test: `check_candidate_switches.py` against a gwz-core whose inventory file lists the new rows. Fails until the sites and rows agree.
  2. `cargo check --tests` in the build `RUSTFLAGS='--cfg gwz_transport_candidate'` (no session cfg): fails to compile once the test files that name `TransportPlacement`, `transport_message` or the five extras are session-gated, until their `Some(Local)` lines are dropped. Do this edit before the production gating, so the failure order is test-first.
  3. A test (new) that the transport-only build's `TransportCapabilitiesResponse` has only `file_identity` and `exact_agent_identity`: `transport_capabilities::handle` (`transport_capabilities.rs:8-20`) is untouched, and `TransportRuntime::capabilities` no longer exists in that build.
  4. The 1.1.0 behaviour pins that must not move: `transport_host/retry_tests.rs`, `driver_tests.rs` (Local rows), `tests/transport_workflows.rs` in gwz-cli. They stay green throughout.
- Change: the eight (b) cfg rows move; the §1.4 Cli items get `cfg_if! { if #[cfg(all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows)))] ... }` (no separate attribute on an item; the bodies in `with_transport`, `open_request` and `close` become arms). `protocol/mod.rs`, `lib.rs` cbor and corpus, `tests/protocol.rs` select the candidate on `gwz_session_candidate`. The transport-only build then equals the post-S7.1 ordinary build plus the transport switch. Reword or remove the lint-reason strings (finding 2). `scripts/candidate_switch_inventory.txt` updated in the same change; `build.rs` keeps both declarations.
- Green: the gwz-core gate; both candidate shapes in `transport-candidate.yml` (`rustflags` at `:37`, `:39`); Windows parity inventory (`scripts/checks/windows_parity/`) for any test that gains a Unix-only gate; **a Windows receipt** (`src/transport_host`, `src/git/gitbackend.rs` and `transport_*` are trigger paths).
- Split if it passes 500: C3a-1 the production arms, C3a-2 the tests.
- **Review:** dual (Code and State). It is the step that decides what the ordinary build can do.

**C3b: gwz-core, remove `gwz_transport_candidate`** *(about 300 lines, almost all `all(gwz_transport_candidate, any(unix, windows))` becoming `any(unix, windows)`)*

- Red tests: `rg gwz_transport_candidate gwz-core` over source, tests, scripts and manifests (the amendment's completeness rule; `dev-docs` excluded) finds the remaining rows, and fails; `test_prepare.py`'s protocol-boundary assertion and `test_windows_qualification_boundary.py` (`:92-93`) are inverted or retired first.
- Change: the 15 (a) rows; `Cargo.toml`'s six tables become `[dependencies]`/`[dev-dependencies]` entries and the `windows-sys` list merges; `build.rs` drops its declaration; `check_process_globals.py:63`, `check_windows_parity.py:30` and their tests; `windows_lane_check.py`; `tests/transport_backend/prepare.py` is retired or inverted (it keeps only the placement guide, now a session test, D6) and its README; CI: `transport-candidate.yml`'s transport-only leg becomes the ordinary job, the session leg's flags become `--cfg gwz_session_candidate` alone.
- Green: gate and boundary job, the Windows matrix on dabeest, `Windows-receipt`. This step needs S7.1's gates (S5.5, S5.6, S3.3, TR2.6), as the remaining plan says.
- Review: phase-level, not per step.

**C4a: gwz-cli** *(about 120 lines)*: the 19 (a) rows; the four inverted arms as §1.3 says; `build.rs`; `scripts/candidate_switch_inventory.txt` becomes empty of rows (or is retired with `scripts/test_candidate_switches.py`, a decision for the lane owner); the 10 lint strings; `tests/transport_workflows.rs` loses its cfg. `docs/CLI.md` is regenerated here only if C4a's own doc test needs it, otherwise C5.

**C4b: gwz-py** *(about 150 lines)*: the 3 (a) rows removed; the 2 (b) rows move to `gwz_session_candidate`; `Cargo.toml:15` keeps only the session declaration; `scripts/build_candidate_extension.py` and `run_tests.py --candidate` stop adding `gwz_transport_candidate`; `src/tests/test_candidate_switches.py`, `test_build_candidate_extension.py`, `test_ci_workflows.py`; its transport workflow. Python tests with the `candidate` fixture run on the ordinary extension. This is S7.3's evidence, so no weakening of an assertion is allowed in the move.

C5 (S7.2) is unchanged and runs after C4b.

C4a and C4b are independent of each other and can run in parallel after C3b.

## 6. Decisions and review points

| # | Question | Recommendation | Who |
|---|---|---|---|
| D1 | Ordinary fields: `max_retries` only (A), or with the four receipts (B)? | **B.** The receipts are the observable proof of "pooling within one command" and of the route assertion; the transport writes them on every open (`transport_observations.rs:113-120`), so gating them would put a `cfg_if` inside `opened()` and turn four core pooling tests and the gwz-py route test into session tests. A costs those tests, and a new route discriminator for S7.3. Neither adds a gwz-cli output field. | Operator; then dual review. |
| D2 | Field presence contract in the ordinary schema: `MISSING_OK`, or house-style `optional=True`? | **`MISSING_OK`.** `optional=True` makes the new reader reject every older writer's map (`try_get(9)?`, shown in scratch). It matches the candidate's generated lines exactly. | Review (wire format). |
| D3 | The remaining plan says "one generated module". The placement projection (86 lines) stays candidate, so a second module stays until S7.1 (1.2.0). Also: how to keep a genuine old-schema reader in `test_candidate.py` now that `CORE_SCHEMA` includes the fields: a checked-in pre-C2 copy of `gwz.taut.py` (digest `76455b40...`, pinned), or a test-side strip of the five members? | Read C2's "one module" as "one *production* module that already has everything 1.1.0 needs", and keep the pair for 1.2.0. For the old reader, the test-side strip (smaller, and it is the repository's "measured additive" idiom); the Rust old reader is a real checked-in file already. | Operator for the reading; reviewers for the idiom. |
| D4 | Accept that gwz-py's `--json` rows gain four keys under B? | Accept and document (S7.5 Surface, migration notes). Hiding them would need a Python-side filter. | Operator. |
| D5 | Accept that Python code constructing `OperationPolicy` or `TransportObservation` by keyword must pass the new arguments? | Accept, as with `url_scheme` before. Surface and migration note. | Operator. |
| D6 | `docs/TransportPlacement.md` and the placement guide test: session-only, not shipped in 1.1.0, contrary to `prepare.py`'s "C3 turns the guide into an ordinary test". | Session candidate; remove the doc from the 1.1.0 package or label it. | Operator. |
| D7 | Re-cut C3/C4 into C3a, C3b, C4a, C4b as §5 proposes? | Yes. | Operator. |
| D8 | `cli_hint` at the four sites that read `is_cli` (finding 1): keep today's `false`, or fix now? | Keep, file the finding, do not widen S7.1. | Operator. |
| D9 | Review shape: wire-format change means dual review at C2.1/C2.2 as one object (the granularity rule: per-step dual for wire format), dual at C3a, phase review for C3b/C4. Max two reviewers per object. | As stated. | Lane owner. |

Dependencies: C2 can run now, in parallel with Phase B. C3a can start after C2.2 and S7.1's inventory precondition (the new rows). C3b and C4 wait on S7.1's gates.

## 7. Evidence index (files written outside the lane, scratchpad only)

All in `/private/tmp/claude-501/-Volumes-projects-limbo-gwz-dev/351b18f9-4ec1-4306-ac0a-299e9bded6dd/scratchpad/c2protocol-analyst/`: `gwz.taut.py` (the edited schema), `gen/` (regenerated Rust), `pyout/` (regenerated Python and IR), `corpus/`, `core/` (candidate regeneration, byte-identical to the lane's), `classify.py` (the counts in §1.1). The lane itself is unchanged apart from this file.

## Lane owner's dispositions (2026-10-16)

D1 B (`max_retries` and the four observation receipts in the ordinary protocol); D2 `MISSING_OK`; D3 one production module carrying everything 1.1.0 needs, the 1.2.0 projection staying candidate, the old-schema reader built by stripping the five members in the test; D4 and D5 accepted, recorded for S7.5's Surface review and the S7.2 notes; D6 the placement guide and `docs/TransportPlacement.md` stay session candidate; D7 C3 and C4 re-cut as C3a, C3b, C4a, C4b; D8 `cli_hint` kept as today, the finding filed; D9 dual review of C2.1 with C2.2 as one object (wire), dual at C3a, phase-level for C3b and C4.
