# GwzRemoteTransportPlan — G46 REVIEW

**Review object:** `gwz-core/dev-docs/GwzRemoteTransportPlan.md` (untracked
working-tree draft). Status line: DRAFT, 2026-09-19; planning only; execution
has not started. SHA-256
`32d5d12a16287bed035cfb4fda0c2c3cbeb97823cb3337f8c7cc2aead1ec54df`.
**Baseline:**
- gwz-dev `7577e1fcb7f2f30837892fded3a1a861ef5a6dbc` (dirty:
  `dev-docs/CurrentProgramCheckpoint.md`, out of scope)
- gwz-core `1cea3a93bf980a80f14fa032e920befa7ccaa72b` (untracked object above;
  controlling design/requirements are the committed files at this HEAD)
- gwz-cli `7db07bbdefd2897c07fd0f9e550bf032bd8b1314`
- gwz-py `d07d55dacb1725d9306be9c04d157ac29a78e000`
- taut `7a5f616c3a9f72e143b6e20dab41ffa6e20e240a`
- taut-shape `74f375c9d3521f3e98110862dcf89ec64a3b6d6c`
Tuple identical at start and end. Sources read from those HEADs plus the
untracked plan blob named above. No builds, tests, network, or writes except
this report file.
**Date:** 2026-09-19
**Reviewer:** G46. Combined draft-stage plan review (Consistency of the
sequence against the accepted design/requirements, and Safety of what the
text permits an implementer to build). Not a dual peer-blind gate and not
acceptance of schema, code, or activation.

**Out of scope / not findings:** the product decision that this code will run
with core in one process and gwz-cli hosting the endpoint in another; iroh;
daemons; credential forwarding; concurrent leases on one connection; attacking
the design's GO/GO at `05842b38`. Today's in-process CLI/gwz-py embeds are the
baseline the channel must also dominate, not a reason to reject two-process
proofs.

**Verdict: NO-GO** — 0 P0 / 0 P1 / 4 P2 / 3 P3. I pre-commit to GO on a
revision that resolves P2-1, P2-2, P2-3 and P2-4 as specified.

---

## 0. Evidence base

**Object.** All 240 lines of the plan at the SHA-256 above.

**Controlling documents (gwz-core `1cea3a93`):**
- `dev-docs/GwzRemoteTransportDesign.md` in full (SHA-256
  `00b4d19a8ba5d85d59725b119f337b92107d21e949bda52d3b3c0bb96e54ccbb`).
  Status accepts the design draft for implementation planning only at
  `05842b38`; HEAD differs from that commit by acceptance annotations only
  (`git diff --stat 05842b38 HEAD --` that file: 7 lines).
- `dev-docs/GwzRemoteTransportRequirements.md` in full (SHA-256
  `2f606bc4407bb780817f8b84dced686401387f95349bd48df117d66a97efbdf2`).
- `dev-docs/GWZDesign.md` § Remote transport direction (lines 5–52) and
  transport capabilities/options/observations (179–223).
- `protocol/gwz.taut.py`: `TransportCapabilitiesResponse` tags 1–2
  (1064–1068); `TransportObservation` (1072–1080); `TransportOptions`
  (1095–1100); `RequestMeta.transport` tag 8 (1120); `OperationAttribution.credential_ref`
  (1001–1002).
- `protocol/regen.py`: `TAUT_GENERATOR_VERSION = "0.9.1"` (68); checked-in
  generation, not build.rs.
- `src/git/gitbackend/backend.rs`: `Git2Backend` is `Git2Repository`,
  `#[derive(Clone)]` (10–19); `operation_services` does
  `Arc::new(self.clone())` per operation (66–70).
- `src/operation/resolve_per_host.rs`: `DEFAULT_MAX_PER_HOST = 8` (1–10).
- `src/operation/eventemitter.rs`: `EventSink::deliver` is one-way (8–10).
- Cargo.lock: git2 0.21.0.

**Drivers:**
- gwz-cli `src/globalargs/dispatch.rs:4–8`: in-process
  `gwz_core::git::Git2Backend::new()` then `handle_*`.
- gwz-py `src/gwz/bridge.py:97–117`: native `call`/`submit` plus
  `subscribe_events` / `wait_events` (one-way operation events).

**Process:** `dev-docs/AgentProcessRules.md` L1-04/L1-05/L1-07;
`EVIDENCE.md`; taut-shape `dev-docs/TautShapeStreamDecision.md` (lossy
`stream`).

**Not used as authority:** the live in-process embeds, except as the
compatibility baseline Phase 4 already names (`embedded core and gwz-py
ordinary local requests`).

## 1. Findings

### [P2-1] Phase 1 freezes limits and an unnamed “public integration API”; Phase 6 still tunes finite limits

**Location.** Plan §3 Phase 1 steps 4–5 and Gate (82–101); Phase 6 exit
(188–195). Design §4.2 (273–280) and §12 Tuning row (664–670).

**Violated invariant.** Design §4.2: “Tuning can lower or revise these
declared finite limits **before schema freeze**; no unbounded mode is
supported.” L1-07: “frozen” means implementers may no longer choose its
meaning. A later phase cannot retune a frozen wire cap without an amendment.

**Evidence.** Phase 1 step 5 requires “enforceable initial limits before
freezing the wire contract” and then the Gate freezes “the schema and public
integration API.” Phase 6 exit requires “tuned finite limits and documented
defaults” and “Choose defaults from results.” Design §12’s completion
condition for Tuning is “Choose bounded payload/window/queue sizes and
wait/cleanup deadlines from fixtures and measurements.” Payload/window sizes
are also Open/Opened/Data wire fields (design §4 inventory, 217–221, and
§4.2 64 KiB / 128 KiB draft caps). The plan never states which of those are
hard caps frozen in Phase 1, which may only narrow later, and which are
construction defaults chosen in Phase 6. “Public integration API” is not
listed: generated message types, carrier framing map, runtime traits, and
GWZ `TransportOptions`/capabilities are different freeze objects (the last
of those is added in Phase 4).

**Reproduction.**
1. Phase 1 interface review freezes Data payload 64 KiB and the generated
   conversation traits as the “public integration API.”
2. Phase 2 implements against that freeze.
3. Phase 6 measurements want 32 KiB payloads or a different window type.
4. Changing a frozen tag/limit is an unamended contract change; leaving
   Phase 1 limits unfrozen makes the Phase 1 gate false.

**Impact.** The first interface freeze either ships unmeasured wire caps or
is not actually a freeze. Parallel HTTPS work (§4) cannot know whether it
may add scheme/auth fields after Phase 1.

**Required correction.** Name the Phase 1 freeze set exactly: taut schema
and tags, generated consumer types, outer framing/routing map, Bind/Bound/
Open inventory, and **hard ingress caps** (use the design §4.2 draft numbers
unless Phase 1 proofs force a documented change). State that Open/Opened may
only narrow negotiated limits, never raise local hard caps. Exclude from
Phase 1: pool/runtime traits, SSH/HTTPS adapter APIs, and GWZ placement
fields (Phase 4). Phase 6 may choose construction defaults (coalesce delay,
buffer/queue sizes, wait/cleanup deadlines, pool cap) **at or below** frozen
hard caps; revising a hard cap requires a requirements/design amendment and
re-review, which the plan already requires for behaviour changes (226–227).

**Closure test.** A one-row table in the plan: freeze object / phase / what
may still narrow. Phase 6 text no longer says it tunes the frozen wire caps.

### [P2-2] Stream-handle cloning is assigned; the endpoint-owned pool across backend clones is not

**Location.** Plan Phase 2 (109–117) and ownership table (31–35). Design §3
(84–86), §7.1 (426–427), §8 backend.rs bullet (542–543). Requirements C6/D1.

**Violated invariant.** D1/C6: the pool belongs to the endpoint, reusable
across operations; process shutdown closes it. Design §8: “operation-scoped
adapter state plus **shared endpoint handle, not a fresh pool per backend
clone**.” Design §3: backend clones “share the endpoint handle but retain
separate identity selections, errors and observations.”

**Evidence.** Phase 2 resolves “cloned-handle ownership” as “cloning a handle
does not acquire a second lease” — that is the **stream** handle rule in
design §6 (394–399), a different object. Core’s live `Git2Backend` is
`Clone` and `operation_services()` wraps `Arc::new(self.clone())` for each
operation (`backend.rs:10–19, 66–70`). Today there is no pool field, so clone
is cheap and isolated. The plan puts generic pool mechanics in
`gwz-transport` and “operation context” in core, and never says who installs
the local endpoint at runtime construction (design §3:71–75) or that
successive `operation_services()` clones must see the same pool.

**Reproduction.**
1. Store the pool on `Git2Backend`.
2. Each GWZ operation clones the backend into a new `Arc` (current call
   shape).
3. If the pool is an owned field, clone duplicates or empties it; each
   operation opens new SSH sessions.
4. C1/D1 reuse across operations fails while Phase 3 fixtures still pass if
   they reuse connections only **inside** one operation. Phase 3 exit does
   say “multiple repositories and repeated operations reuse” (139–140), but
   the ownership that makes that possible is not assigned, so the fixture
   can be satisfied with a process-global or per-operation pool that violates
   G7 or D1.

**Impact.** The first measurable SSH milestone can report reuse that is not
endpoint-owned, or can create a process-global pool to make reuse work —
the exact registration/coexistence class the design review already closed.

**Required correction.** Assign: the host installs one local endpoint (and,
when present, one driver binding) at runtime construction; `Git2Backend`
clones share that handle; identity, errors and observations stay
operation-scoped. Cloning a **stream** handle still does not take a second
lease. Forbid a process-global pool and a pool-per-`Git2Backend` clone.
Phase 2/3 exit: two successive operations through `operation_services()`
reuse one idle connection; dropping one backend clone does not shut the
endpoint.

**Closure test.** Named owner in §2; a Phase 2 case “backend clone is not a
new pool”; Phase 3 reuse fixtures go through two operation_services
lifetimes, not two streams on one backend.

### [P2-3] Phase 3 can advertise local SSH while design §8 network entries stay on the built-in stack

**Location.** Plan Phase 3 exit/milestone (138–145). Design §8 (535–551) and
§11 Git semantics / URL routing rows (644–647). Requirements C1, G7.

**Violated invariant.** Design §8: “Cover every network entry, including
workspace bootstrap clone, advertisement reads, materialize, fetch, tags,
pull and push verification.” C1: reuse across repositories, **phases and
successive operations**. “Never advertise an unimplemented placement or
protocol” (design §12:677). Partial coverage with the per-remote callback
enabled on some paths only is a silent dual SSH stack, not a listed
remaining capability.

**Evidence.** Phase 3 fixtures: “discovery, fetch/clone, push, rejection,
uncertain push and post-push reads.” That omits bootstrap clone,
materialize, tags and pull, which are live `max_connections_per_host`
callers today (`handle_init_from_sources.rs`,
`handle_materialize/apply.rs`, `pull_head_member_preflight`). The milestone
is “first measurable local SSH reuse” with “only qualified capabilities
advertised.” Nothing says leftover GWZ network entries remain on native
libgit2 SSH **and must not be counted as pooled**, or that the callback is
installed for every GWZ remote those entries use.

**Reproduction.**
1. Set `git_remote_callbacks.transport` only in fetch/push.
2. Advertise SSH endpoint support after Phase 3.
3. `gwz pull` / materialize still use built-in libgit2 SSH (new connection
   per stream, design historical snapshot).
4. Observations, pooling and fail-closed identity apply to some operations
   and not others on the same advertised capability.

**Impact.** False composition of “SSH reuse is on.” Mixed stacks also make
G7 coexistence tests pass for the wrong reason (unrelated traffic is
untouched because GWZ itself still uses the built-in transport on several
entries).

**Required correction.** Either (a) Phase 3 routes every design §8 GWZ
network entry through the per-remote callback before SSH is advertised, or
(b) the Phase 3 milestone lists remaining native entries explicitly as
not-yet-pooled, not advertised as the new SSH path, and Phase 6 cannot
close until the list is empty or accepted as remaining unsupported. File/
local-family stay on the existing credential-free path (design §8:550–551).

**Closure test.** An entry table in Phase 3/6: each §8 operation → new
adapter / still native / local-only. Advertising SSH requires column one
for every SSH-using entry, or a named remainder.

### [P2-4] One “cap of eight” can collapse the operation fan-out bound and the endpoint ceiling

**Location.** Plan Phase 2 (116–117). Design §7.2 (449–462). Requirements
C4. Live `OperationPolicy.max_connections_per_host` / `DEFAULT_MAX_PER_HOST = 8`.

**Violated invariant.** Design §7.2: existing
`OperationPolicy.max_connections_per_host` continues to bound **that
operation’s** fan-out through `par_map_per_host`. It is not a
process-global mutable setting. The endpoint ceiling is an **additional**
bound across operations; a request cannot raise it; setting a lower
operation limit does not evict another operation’s connections.

**Evidence.** Phase 2: “Start with the proposed cap of eight, qualify it
with the other limits.” There is one number and no owner. The live default
eight is the **concurrent-work** limit, not a persistent physical pool.
Implementing a single cap of eight as both (or reinterpreting the policy
field as the endpoint ceiling) changes request semantics: a later operation
with `max_connections_per_host: 2` could evict another operation’s idle
connections, or two overlapping operations could open 16 physical sessions
because each thinks it owns “the” cap of eight.

**Reproduction.**
1. Long-lived embedding, local endpoint pool cap implemented as
   `resolve_per_host(policy.max_connections_per_host)`.
2. Operation A (default 8) holds 4 idle connections after a fetch.
3. Operation B requests `max_connections_per_host: 1`.
4. Under the merged-cap reading, B evicts A’s idle entries or waits behind
   them; under the design reading, B’s fan-out is 1 and A’s idle connections
   remain until 60 s expiry, counting against the separate endpoint ceiling.

**Impact.** Either stolen idle connections (availability) or unbounded
physical growth across overlapping operations (the N+1 cost this programme
exists to remove, reintroduced between operations).

**Required correction.** Phase 2 states two bounds: endpoint construction
ceiling (start at 8 per user/host plus 8 aggregate per host, design §7.2)
and unchanged per-operation `max_connections_per_host` fan-out. Opening,
idle, allocated and closing count against the endpoint ceiling. Qualify
them together; do not implement one number.

**Closure test.** Phase 2 cases: overlapping operations with different
per-host policy values; lower operation limit does not evict the other
operation’s connections; combined physical count respects the endpoint
ceiling.

### [P3-1] Observation contract is under-specified relative to G3/D6

**Location.** Plan Phase 4 (157–158). Design §10 (602–608); requirements G3.

Phase 4 forbids copying another operation’s **attribution**. The design
forbids copying an earlier operation’s entire **observation row**; on reuse,
`credential_offered` stays false for this attempt; proven facts come from
the connection record; the legacy nullable `authenticated` field must not
be forced true; private-member suppression is an acceptance case (design
§11:646). Without those in Phase 4/6 exit, reuse can be reported as a fresh
credential offer or a secret-adjacent fingerprint leak.

**Correction.** Add offered/authenticated/reused, no-copy-row, and
private-member suppression to Phase 4/6 evidence. Sentinel credentials in
Phase 5 already cover messages/diagnostics.

### [P3-2] Timeout domains and P9 are unassigned

**Location.** Design §10 table (610–620); requirements P9, C5. No phase
owns allocation-wait vs connect vs active I/O vs coalescing vs
user-interaction vs close-cleanup vs pool-idle.

**Consequence.** Helper/login waits can consume the network timeout; idle
reaping can look like I/O failure. P9: missing login must fail actionably
and must not start a login workflow; any supported wait is cancellable and
separate from network timeouts.

**Correction.** One sentence in Phase 2/3/5: implement the seven domains;
gh missing-login is an actionable failure; no implicit login in this
programme.

### [P3-3] Surface review is named but not bound to the placement-option freeze

**Location.** Plan §5 (215–217); Phase 4 adds the typed placement option
(154–156); Phase 6 “Finish user configuration/help” (184–185).

The Surface axis exists because dual review missed a command family with no
uninstall. Placement default is local (design §3:74–75; P3). Phase 4 must
not advertise a placement flag/config whose disable path is “omit the
unknown field and hope.” Bind Surface review to the freeze of that option
(name, default = local, explicit unsupported/unavailable errors, help).
Lifecycle is install-binding at driver construction / drop-binding on
process exit unless a user-facing host command is added; if one is added,
it needs a matching teardown.

## 2. Invariant analysis

Attacks that **held** (not findings):

- **Two-process CLI/core is the design.** Plan Phase 1 step 4 (fake
  endpoint, both directions, two real processes) and Phase 4 (distinct
  credential environments, kill either process, no fallback) match P1/P4/P5
  and design §3:86–89. Using the one-way operation-event subscription is
  forbidden (plan 56–57); live `EventSink` / `subscribe_events` must not
  become that carrier. In-process delivery with the same bounds remains
  required (plan 58–59; S1).
- **Carrier is not claimed to already exist.** Plan 51–54 and design §4.1
  (lossy taut `stream` must not be silently reused) agree with
  `TautShapeStreamDecision.md`. Phase 1 gate to revise the boundary if the
  carrier cannot supply the behaviour is the right stop rule.
- **No process-global `git2::transport::register`.** Phase 3 states it;
  matches G7 and the closed design-review finding.
- **Schema ownership first, no hand-copy into core.** Matches design §4
  (208–209). Requirements S8 (extraction optional for *designing/testing*)
  is satisfied if Phase 1 tests the contract in the new package immediately;
  empty-repo creation alone is already called insufficient (plan 19–22).
- **Dependency direction.** Transport builds without core/cli; core never
  depends on cli (41–42).
- **gh-only HTTPS, 60 s idle, exclusive first-implementation leases, no
  automatic fallback, no daemon/iroh/credential forwarding.** Plan §1
  preserves D1–D2, D5, D8, D10, D14.
- **Checked-in generation.** Phase 1 accounts for `protocol/regen.py`
  rather than build.rs.
- **Design GO/GO is not code/schema acceptance.** Plan header and §5.
- **Evidence split.** Phase 6 points at workspace `EVIDENCE.md`; public CI
  must not depend on the private member.
- **Fetch Phase 3 is a consumer, not the programme.** Plan §1 and
  completion paragraph (229–232).
- **Tag preservation on GWZ protocol.** Phase 4; live
  `TransportOptions` tags 1–3 and `TransportCapabilitiesResponse` tags 1–2
  stay additive.

Attacks that **landed** are P2-1–P2-4: freeze vs tune, pool owner vs
backend clone, advertised SSH vs leftover native entries, and the two
capacity bounds.

## 3. Risks and next action

Residual (below the bar): Phase 1 is still a large interface package
(member + schema composition + reliable carrier + two-process proof); that
is appropriate for the freeze, not a reason to skip the carrier proof.
git2 0.21.0 still lacks a safe per-remote transport setter (design §8);
Phase 3 already treats that as a delivery dependency. libgit2’s write
callback calls `write_all`, not `flush` (design §5:349–353) — Phase 2
timers must run without a flush from the Git adapter; fold into Phase 2/3
tests when editing for P2-1. HTTP `EndWrite` vs `Flush` as end-of-body
(design §9) belongs in Phase 5 fixtures.

**Next action:** amend the plan text for P2-1–P2-4 (and the P3s if cheap).
Do not start Phase 1 execution, repository creation, or schema generation
on this draft. After the corrected plan is the object, re-verdict this
review.
