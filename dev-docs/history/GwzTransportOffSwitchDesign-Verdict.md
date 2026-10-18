# GWZ transport setting design (TR1.5) — verdict

Date: 2026-10-02.

**Accepted** at SHA-256 `145af486dc0d757ca6f653c6a8ad1f24235be34381ba35b1412143ac4caf3012` (revision 2), after [Consistency-3](GwzTransportOffSwitchDesign-ReviewConsistency-3.md), [Safety-3](GwzTransportOffSwitchDesign-ReviewSafety-3.md) and [Surface-3](GwzTransportOffSwitchDesign-ReviewSurface-3.md) reported GO on that revision. This accepts the design text only.

The [design](GwzTransportOffSwitchDesign.md) as filed is revision 3 (`490dac8673d9f314c6fa72dcabac14e72a5824a65de7237cc5d854cf97318032`) with two additions: the acceptance status block, and Surface-4's optional line that gwz applies no conditional include. It hashes `208be7f1eaf0c74177601e927de77fa2d2197c1471d06fc9430f2e72239907b7`. Revision 3 carries round 3's corrections, which the reviewers cleared without a further round. [Safety-4](GwzTransportOffSwitchDesign-ReviewSafety-4.md) and [Surface-4](GwzTransportOffSwitchDesign-ReviewSurface-4.md) confirmed the two forms that differ from their text (below).

## Round 1: revision 0 (`73d63a6f…`)

| Review | Verdict | Findings |
|---|---|---|
| [Consistency](GwzTransportOffSwitchDesign-ReviewConsistency.md) | NO-GO | 1 P2, 5 P3 |
| [Safety](GwzTransportOffSwitchDesign-ReviewSafety.md) | NO-GO | 4 P2, 2 P3 |
| [Surface](GwzTransportOffSwitchDesign-ReviewSurface.md) | NO-GO | 3 P2, 6 P3 |

- **The P2s:**
  - Consistency: the configuration files for an empty `XDG_CONFIG_HOME` and the passwd home.
  - Safety:
    - a repository value refusing gwz-py operations under the `error` filter;
    - an unreadable `~/.gitconfig` refusing every network command;
    - the native path running under defaults 1.0.17 never had;
    - a workspace scan that could block.
  - Surface: the double negative "the transport off switch is on"; `user_configuration`; scope words outside gwz's vocabulary.
- **Blind convergence:**
  - the scan-against-filter contradiction (Safety P2-1, Consistency P3-2);
  - git's against libgit2's global files (Consistency P2-1, Safety P3-1);
  - `--max-retries`' dependency on TR2.1 (Consistency P3-1 and a Surface risk note).
- [Remediation plan 1](GwzTransportOffSwitchDesign-RemPlan.md), D1–D7, produced revision 1.

## Round 2: revision 1 (`a72f9216…`)

Every round-1 finding is closed on every axis.

| Review | Verdict | New findings |
|---|---|---|
| [Consistency-2](GwzTransportOffSwitchDesign-ReviewConsistency-2.md) | GO | 4 P3, and one labelled architectural note: the native defaults resolved in the drivers rather than in core |
| [Safety-2](GwzTransportOffSwitchDesign-ReviewSafety-2.md) | NO-GO | P2-5, a workspace path unquoted in the removal commands (a copy-paste injection), and 3 P3 |
| [Surface-2](GwzTransportOffSwitchDesign-ReviewSurface-2.md) | GO | 3 P3 |

- **Blind convergence:**
  - the removal command for a value in an included file, on all three axes;
  - gwz-py's explicit `max_connections_per_host=32`, on two.
- [Remediation plan 2](GwzTransportOffSwitchDesign-RemPlan-2.md), E1–E9, produced revision 2. This was the second and last remediation round.

## Round 3: revision 2 (`145af486…`)

| Review | Verdict | New findings |
|---|---|---|
| Consistency-3 | GO | none; 3 notes |
| Safety-3 | GO | P3-6 to P3-8 |
| Surface-3 | GO | P3-10, P3-11 |

Revision 3 applies them all. Two take a form other than the reviewer's text, and each reviewer confirmed the form in a short follow-up, Safety-4 and Surface-4. Safety-4 adds that the session plan's re-check should record the edge CS6.6 ── CS6.4 in the Phase 6 sketch too.
- **Surface P3-10.** The locating command runs from `/`: `git -C / config --file '<file>' --includes --show-origin --get-all gwz.transport`. gwz evaluates includes against no repository, so `/` reproduces what gwz read.
- **Safety P3-8.** In 1.2.0 the native defaults are filled where CS3.10 picks the native path. The drafter found that CS6.4 needs an edge from CS6.6.

**Architectural root causes.** One was labelled, the site of the native defaults. E3 examined it and declined core resolution on cited grounds: no route method on the public `GitBackend` contract. Its 1.2.0 consequence is assigned (Safety-3 P3-8). The cap was not reached.

## Decisions applied as recommendations

The operator can reverse any of these before TR2.5:
- **D2.** The surface names the transport: `--transport <gwz|native>`, `GWZ_TRANSPORT=gwz|native`, `gwz.transport` in the user's global git configuration, and JSON `meta.transport_setting`. "Off switch" stays the plan's internal name only.
- **D3.** With native selected, 1.0.17's defaults apply: `--jobs` 50, `--max-per-host` 8, and `--ssh-timeout` 3 s in gwz-cli. In gwz-py the libgit2 timeout is one clock for the whole process, 9 s unless it is set.
- **E3.** In 1.1.0 the drivers fill those defaults in. gwz-py's `Client(max_connections_per_host=None)` keyword postdates 1.0.17. TR2.11's callers without a host context run at 100 and 32. In 1.2.0 the session host fills them.

## The operator's answers, 2026-10-02 ("All as recommended")

D2, D3 and E3 are kept, and each question is answered as recommended:

- **OQ1.** There is no way to silence the note.
- **OQ2.** It is a gwz-core module, not a crate.
- **OQ3.** All four items go to the server design's next revision.
- **OQ4.** S7.3 (1.1.0) gains a consumer-build row with `GWZ_TRANSPORT=native`. Amendment 2's next revision adds it.

TR2.5 may start.

## Follow-ons

- **The session plan's re-check:** CS3.10, CS8.3, C1, CS8.28 (gwz-py's CLI `--transport`) and CS4.5. Also the 1.2.0 fill under CS3.10, CS4.7 and CS6.4, and the edge CS6.6 ── CS6.4.
- **Status lines,** applied with this verdict: the retry plan and the Python design.
- **TR2.5** lands in three steps once the operator answers OQ1 to OQ4, with the edge TR2.1 ── TR2.5's retry row.
