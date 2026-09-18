# GwzRemoteTransportPlan — G46 REVIEW-1

**Review object:** `gwz-core/dev-docs/GwzRemoteTransportPlan.md` (untracked
working-tree draft, revised after G46 NO-GO). Status: DRAFT, 2026-09-19;
planning only; execution has not started. SHA-256
`55120dd1af7b77818eb71fda609818b6c1bb2539a08ec9f025ab01f7d4899b99`.
**Prior object:** SHA-256
`32d5d12a16287bed035cfb4fda0c2c3cbeb97823cb3337f8c7cc2aead1ec54df`, reviewed in
`GwzRemoteTransportPlanReview-G46.md` (unmodified;
`f28947411ac4b2937b6df6383584b6b6a5175fa5769a7f3273b19762008633bb`).
**Remediation record:** `GwzRemoteTransportPlan-RemPlan.md` (legitimate
round-2 input; dispositions are drafter claims, not closure).
**Baseline:**
- gwz-dev `7577e1fcb7f2f30837892fded3a1a861ef5a6dbc` (dirty:
  `dev-docs/CurrentProgramCheckpoint.md`, out of scope)
- gwz-core `1cea3a93bf980a80f14fa032e920befa7ccaa72b` (untracked plan, remplan,
  G46 report, and this re-verdict; controlling design/requirements remain the
  committed files at this HEAD)
- gwz-cli `7db07bbdefd2897c07fd0f9e550bf032bd8b1314`
- gwz-py `d07d55dacb1725d9306be9c04d157ac29a78e000`
- taut `7a5f616c3a9f72e143b6e20dab41ffa6e20e240a`
- taut-shape `74f375c9d3521f3e98110862dcf89ec64a3b6d6c`
Tuple identical at start and end of this re-verdict. Design/requirements
hashes unchanged from round 1
(`00b4d19a8ba5d85d59725b119f337b92107d21e949bda52d3b3c0bb96e54ccbb`,
`2f606bc4407bb780817f8b84dced686401387f95349bd48df117d66a97efbdf2`).
No builds, tests, network, or writes except this report file.
**Date:** 2026-09-19
**Reviewer:** G46, same reviewer as round 1. Combined draft-stage re-verdict.
Not a dual peer-blind gate and not acceptance of schema, code, or activation.

**Out of scope / not findings:** the two-process CLI/core product decision;
iroh; daemons; credential forwarding; concurrent leases on one connection;
attacking the design GO/GO at `05842b38`. Drafter self-closure is not used.

**Verdict: GO** — 0 P0 / 0 P1 / 0 P2 / 0 P3 open. All four P2 and three P3
findings from G46 are closed on this revision. No new blocking finding in the
changed range. This accepts the **plan draft** only: sequence, Phase 1
boundary, freeze/ownership/coverage/capacity assignments. It does not freeze
schema tags, accept implementation, or authorize repository creation.

---

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| **P2-1** — freeze versus later tuning | Accept: freeze table; design §4.2 starting caps; Phase 6 construction defaults only | Original counterexample retraced below | **CLOSED** |
| **P2-2** — pool owner across backend clones | Accept: §2 shared endpoint handle; Phase 2/3 clone-drop tests | Original counterexample retraced below | **CLOSED** |
| **P2-3** — incomplete SSH entry coverage | Accept correction (a): coverage ledger; advertise only after full SSH coverage | Original counterexample retraced below | **CLOSED** |
| **P2-4** — operation and physical capacity conflated | Accept: two named bounds; overlapping-operation exit cases | Original counterexample retraced below | **CLOSED** |
| **P3-1** — observation semantics | Accept: no-copy-row; offered/authenticated/reused; nullable authenticated; private-member | Retraced below | **CLOSED** |
| **P3-2** — timeout domains and P9 | Accept: seven domains in Phase 2; Phase 5 no implicit login | Retraced below | **CLOSED** |
| **P3-3** — placement Surface gate | Accept: Phase 4 Surface freeze; default local; install/drop lifecycle | Retraced below | **CLOSED** |

## Changed-range analysis

The revision is documentation only (240 → 359 lines). Load-bearing additions:
§2 endpoint-install/share/drop rules (48–55); Phase 1 freeze table and
starting caps (101–126, 135–136); Phase 2 two capacity domains, seven
timeouts, backend-clone case, flush-independent timer (146–181); Phase 3
coverage ledger and advertisement gate (196–232); Phase 4 observation
contract and Surface freeze (244–266); Phase 5 login/EndWrite (274–285);
Phase 6 construction-only tuning (291–315); §5 Surface bound to Phase 4
(333–335).

No new architectural root: the accepted design’s freeze-before-tune,
endpoint-owned pool, full SSH entry coverage, and two-layer capacity are
now named in the plan rather than substituted. Parallel HTTPS-with-CLI
work remains, but Phase 1 now requires SSH **and** HTTPS descriptors in
the frozen inventory (101–102) and Phase 3 keeps HTTPS unadvertised until
its own gate (231–232), so the previous freeze-object hole is not reopened.
Residual notes from round 1 (write_all vs flush; HTTP EndWrite vs Flush)
are assigned in Phase 2 (178–179) and Phase 5 (283–284).

## 0. Evidence base

**Object.** All 359 lines of the revised plan at the SHA-256 above. RemPlan
read in full. Round-1 report read in full; not edited.

**Retrace method.** Each original reproduction was applied as a reading of
the new text: would that sequence still be licensed? Controlling design
§§3, 4.2, 6–10 and live `Git2Backend::operation_services()` clone boundary
were not re-opened as new design review; they remain the same committed
baseline as round 1.

## 1. Closure evidence (original counterexamples)

### P2-1

Round-1 sequence: Phase 1 freezes 64 KiB Data as an unnamed “public API”;
Phase 6 retunes a hard cap or changes a type.

New text: Phase 1 records the design §4.2 starting caps, including 64 KiB
Data / 128 KiB stream frames (107–113). The freeze table (117–122) names
schema/tags, full message inventory, generated types, framing map, and hard
ingress caps as the Phase 1 object; Open/Opened may only narrow (104–105,
119). Phase 6 “Wire fields, types, tags and hard ingress caps are not
retuned” (300–302); construction defaults stay at or below frozen caps
(297–300). A changed hard cap needs a requirements/design amendment
(112–113, 301–302). Pool/runtime, adapter APIs, and GWZ placement are
excluded from Phase 1 (124–126). Later adapters cannot silently add
SSH/HTTPS descriptors because those sit in the Phase 1 inventory (101–102).

The original sequence is no longer licensed. **CLOSED.**

### P2-2

Round-1 sequence: store the pool on `Git2Backend`; `operation_services()`
clones it; reuse is per-operation or process-global.

New text: the host installs one local endpoint and optional driver binding
at runtime construction; `Git2Backend` clones, including
`operation_services()`, share the handle; identity/errors/observations stay
operation-scoped; process-global and per-clone pools are forbidden; dropping
one clone does not shut the endpoint (48–55). Stream-handle cloning is
called out as a different rule (54–55, 146–147). Phase 2 exit includes
“backend clone is not a new pool” (175–176). Phase 3 exit requires two
successive `operation_services()` lifetimes: second reuses the first’s idle
connection; dropping the first clone leaves the endpoint live (220–222).

The original sequence is forbidden and the specified closure tests are now
exit evidence. **CLOSED.**

### P2-3

Round-1 sequence: hook fetch/push only; advertise SSH; pull/materialize
stay on built-in libgit2 SSH.

New text: every GWZ SSH network entry must use the per-remote callback
before advertising local SSH (196–197, 213–214). Ledger rows include
bootstrap/init-from-sources, clone/advertisement, materialize, fetch, tags,
pull, push/post-push, plus file/local-family and native HTTP/git
compatibility (201–211). Pending rows are not implementation evidence
(197–199). Partial measurements may not claim general SSH support (213–215).
CLI placement and HTTPS stay unadvertised until their gates (231–232).
Phases 4–6 repeat/check the ledger (215–216, 265–266, 284–285, 309–310).

Advertising SSH with pull or materialize still native is explicitly a closed
gate. File/local stay credential-free local; HTTP/git keep the local-only
disposition. **CLOSED.**

### P2-4

Round-1 sequence: implement the pool cap as
`resolve_per_host(policy.max_connections_per_host)`; operation B with
`max_connections_per_host: 1` evicts operation A’s idle connections, or two
operations open 16 physical sessions.

New text: endpoint construction ceilings start at eight per user/host and
eight aggregate per host; opening/idle/allocated/closing all count;
requests cannot raise them (155–158). `OperationPolicy.max_connections_per_host`
remains per-operation fan-out through `par_map_per_host`; a lower operation
limit neither resizes the pool nor evicts another operation’s connections
(159–162). Phase 2 exit requires overlapping operations with different
per-host limits and the non-eviction case (176–178). Phase 6 may tune
endpoint pool **ceilings** as construction defaults within the frozen
contract, and “cannot … change per-operation fan-out semantics” (298–303).

The merged-cap reading is no longer licensed. **CLOSED.**

### P3-1

Phase 4 forbids copying an earlier operation’s observation row; on reuse
`credential_offered` is false; proven facts come from the connection record;
nullable `authenticated` is preserved; private-member suppression is
required (244–248, 263–265). Phase 6 rechecks (311–312). **CLOSED.**

### P3-2

Phase 2 assigns all seven design §10 domains and forbids helper/backpressure
masquerading as network stall or idle expiry (165–169). Phase 3 binds
adapter waits (227). Phase 5: missing gh login fails actionably without
starting a login workflow; supported waits are bounded, cancellable, and
separate from network timeouts (274–276). **CLOSED.**

### P3-3

Phase 4 freezes the placement option with Surface review of name, default =
local, explicit unsupported/unavailable errors, configuration and help
(250–257). Omission selects local; an explicit failed request must not
become omission (252–253). Binding is runtime install / process-exit drop;
a future host command needs matching teardown before exposure (253–256).
Phase 6 validates that surface and does not defer the gate (256–257,
291–292, 333–335). **CLOSED.**

## 2. New findings

None. No new P0–P3.

## 3. Invariant analysis

Round-1 held attacks remain held (two-process CLI/core as design; carrier
not claimed to exist; no process-global register; schema ownership first;
dependency direction; gh-only / 60 s idle / exclusive leases / no fallback;
regen.py; design GO/GO is not code acceptance; evidence split; fetch Phase 3
is a consumer; additive GWZ tags).

The four round-1 landing attacks are closed as above. Changed-range
interactions did not produce a new architectural root.

## 4. Risks and next action

Residual (below the bar): Phase 1 is still a large interface package; that
is appropriate for the freeze. git2 0.21.0 still lacks a safe per-remote
transport setter; Phase 3 still treats that as a delivery dependency.
Parallel HTTPS-adapter work with CLI placement is allowed only behind the
Phase 1 frozen inventory and separate advertisement gates — keep those
gates closed in execution. Ledger “Pending” cells must not be mistaken for
executed evidence when Phase 3 starts.

This GO does **not** start Phase 1. Operator still has to request execution.
Immediate next action after this re-verdict: treat the corrected plan as
the planning object; do not create the transport repository, dependencies,
schema, or experiments until that request.
