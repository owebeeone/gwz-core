# GWZ transport release plan amendment 2 — verdict

Date: 2026-10-01. Status: **accepted at SHA-256 `c5850e52227e9f27e7c989c417ea262d6f749e312593af3cbbc7baeeda468509` after [Consistency-2](GwzTransportReleasePlanAmendment-2-ReviewConsistency-2.md) and [Safety-2](GwzTransportReleasePlanAmendment-2-ReviewSafety-2.md) reported GO; this accepts the amendment text only**.
- It authorizes no implementation, commit, tag, push or publish.
- OD13, OD14 and OD15 were decided by the operator on 2026-10-01: OD14 is applied in revision 3, and OD15 in revision 4, with OD16's shape following it. OD17 is taken only if TR8.1 (1.1.0) misses.

## Rounds

The same two reviewers ran every round.
- From round 2 on, each continued with its context intact.
- Each verified the tuple at the start and the end of each round: root `d6f6db59`, gwz-core `0ddc513c`, gwz-cli `5ebb001`, gwz-py `950064d` and gwz-transport `a24e70a`. Each also verified the object's hash, and from round 2 on the remediation plan's.
- Both ran only inspection commands.
- Neither saw the other's report from the current round.
- The lane owner had overwritten revision 0 when revising it. Revision 0 was then recovered from the reviewers' own reads of it, matching its reviewed hash, and each round received a byte-exact diff from the revision it last reviewed.

| Round | Revision (SHA-256) | Consistency | Safety |
| --- | --- | --- | --- |
| 1 | 0 (`57ce8e46…`) | [NO-GO](GwzTransportReleasePlanAmendment-2-ReviewConsistency.md): 6 P2, 12 P3 | [NO-GO](GwzTransportReleasePlanAmendment-2-ReviewSafety.md): 1 P1, 5 P2, 8 P3 |
| 2 | 1 (`42b91afd…`), after [remediation plan 1](GwzTransportReleasePlanAmendment-2-RemPlan.md) | [GO](GwzTransportReleasePlanAmendment-2-ReviewConsistency-1.md): all closed; 6 new P3 | [NO-GO](GwzTransportReleasePlanAmendment-2-ReviewSafety-1.md): all closed; 1 new P2, 1 new P3 |
| 3 | 2 (`c5850e52…`), after [remediation plan 2](GwzTransportReleasePlanAmendment-2-RemPlan-2.md) | [GO](GwzTransportReleasePlanAmendment-2-ReviewConsistency-2.md): all closed; 3 new P3 | [GO](GwzTransportReleasePlanAmendment-2-ReviewSafety-2.md): all closed; 2 new P3 |

- **Remediation rounds:** two, the cap.
- **Architectural root causes:** neither reviewer classified any finding as one.

**Blind convergence in round 1.** The two axes found these root causes independently:
- without a host context, SSH takes the transport's lazy endpoint, so activation would have put gwz-py on the transport;
- the adopted Python transport-route sentences stayed in 1.1.0's activation and release;
- S5.4, S5.5 and TR8.4 waited on each other;
- rule (e) named the second switch before it existed;
- gwz-py's `TransportSession` had already been removed;
- one measurement figure was the wrong median.

**Defects by phase found.** The review found three upgrade breaks before any implementation existed:
- gwz-py on the transport, which would also fail on Windows without `HOME`;
- the transport's SSH home taken from `HOME` alone, where 1.0.17 falls back to `HOMEDRIVE`/`HOMEPATH` and `USERPROFILE`;
- an unbounded default-credential route, which would have kept 1.0.17's forced-authentication exposure.

## Corrections applied after the GO

Each was applied as its reviewer specified, and the changelog records each. The amendment then carries the accepted status, and hashes `4da27115695dda389bd0a3279424f97a450b6a540312d5b12de1902ffb3982d9`.

1. **Consistency-2 P3-C.** When the challenge also offers a scheme that the transport's gh route answers for the host, OD16's trigger gives way to the gh route. A failed zone lookup does not satisfy the trigger. TR1.8 gains a `Negotiate, Basic` row.
2. **Safety-2 P3-11.** OD16's refusal names the Trusted zone first and the off switch second. The notes state that the off switch restores 1.0.17's default-credential offer for every host. S7.5 (1.1.0)'s Surface reads both. TR1.8 gains a Trusted-zone row.
3. **Consistency-2 P3-B and Safety-2 P3-10,** which are the same edit. Under alternative (a), OD13's exception names the refusal. Each alternative's list also re-reads S7.2 (1.1.0)'s notes item.
4. **Consistency-2 P3-A.** §1, §3.15 and §5 name CS6.5's two lazy-endpoint sentences and §2.4's candidate-build sentence.
5. **A follow-on from revision 2's move of the inventories into files.** §5's checkpoint bullet records the inventory files' digests once TR2.12 creates them.

## Recorded

- **Residuals the reviewers noted below the finding bar:**
  - TR8.1 (1.1.0)'s and TR8.4's off-switch rows need TR2.5, which the sketch does not draw. Phase 9 waits on all of Phase 2, so the rows can only wait, never run early.
  - The candidate job's both-cfgs leg has to drop the removed cfg at S7.1 (1.1.0). The completeness check forces that.
  - The inventory files mirror the tree. The absence assertions of S7.3 (1.1.0), the post-release check and S7.5 (1.1.0) carry the gate's weight.
  - Phase 10 (1.1.0)'s Windows precondition is a recorded, dispatched run, not a `needs:` edge in `release.yml`. Steps 1–4's publishes precede it, which is acceptable, because any later gwz-core commit needs those crates.
  - Whether the placement projection's fields enter 1.2.0's ordinary protocol is for S7.1 (1.2.0) and S7.5 (1.2.0)'s Surface review.
- **For TR2.11's brief** (Safety-1's residual): a source test that the CLI's `transport_meta` arms equal gwz-core's production `with_transport` call sites. That would keep "no silent native route" durable. They are equal today by inspection.
- **Evidence.** §2 item 2's measurement logs were kept only in a scratch directory, and TR2.9, TR2.10 and TR8.1 measure again under the plan's evidence rules. The 2026-09-23 alpha's rows in those logs, 5.6 s at its defaults over 8 connections, are consistent with the two causes named.

## Application

The amendment's §5 edits were made on this GO, each with a changelog entry:
- **`GwzTransportReleasePlan.md`**, **`GwzTransportReleasePlanAmendment.md`** and **`GwzV110PlanAmendment.md`** in gwz-core;
- **`GwzCoreSessionPlan.md`**, **`GwzCoreSessionCrateMap.md`**, **`GwzConnectionReuseDesign.md`** and **`GwzCoreServerDesign.md`** in the root.

Each status gains the amended-status sentence in AgentProcessRules §7.2's pattern. The crate map had no changelog section, and now has one.

Still to come:
- **The program checkpoint** records the acceptance now. It records OD14's, OD15's and OD16's answers when the operator gives them, and the inventory files' digests when TR2.12 lands.
- **Nothing is committed.** The amendment, its reviews, its plans, this verdict and the seven status edits wait for the operator's go.

## Revision 3: operator decision OD14

- **The decision.** On 2026-10-01 the operator decided OD14's alternative: gwz-py's network operations take the per-operation transport entry in 1.1.0.
- **The process.** The operator directed that the decision be applied without the review loop, and that one skim review check it once the design was done.
- **The changes.** Revision 3 restores the 1.1.0 amendment's S6.1–S6.3 for 1.1.0 (§3.17), with the changes the new [Python design](../../gwz-py/dev-docs/GwzPyPerOperationTransportDesign.md) requires:
  - the environment snapshot is taken under the GIL;
  - panic safety;
  - one transport predicate, shared by both drivers;
  - cancellation, `close()` and interpreter exit.

  The sections that cite OD14 follow. Revision 3 hashes `432d7118f8eaeccd32773613efb588c1ec22b92804b70c67be14e424344a01e5`.
- **Its review:** the [skim review](GwzTransportReleasePlanAmendment-2-ReviewSkim.md) reported NO-GO, with six P2 and three P3 text defects. As the operator directed, the amendment and the Python design apply all nine without a further round. The amendment then hashes `2af05214f2556eb7a371df03d09d1219b120bf80bae96f610cea63366cb40d43`, and the design `6145b471e12bc3c8226b3e7b913838566a8b144571030cece2738ed3c0daf027`. The [re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-1.md) then reported **GO**, with all nine closed. Its four new P3s are applied as it specified. The amendment then hashes `c5561fc59453e6ff8647eaf36d362323dfc458656702d689f7771bea7567a97d`, and the design `3d656d1a14070b73e2e8ea795b87a572c8ab5ffee8efe692b470a8942bf28ea0`. Revision 3 carries a skim-review GO, not a dual-review GO.

## Revision 4: operator decision OD15

- **The decision.** On 2026-10-01 the operator rejected native routes as the way to reach Windows parity: "we can't do native only", and "I said we need to support windows parity". OD13 already required parity, so OD15 is settled: the transport itself does on Windows what 1.0.17's native path does.
- **Why native routes fail it.** A native route runs libgit2 inside the core's process. It gets no pooling, and no reuse once 1.2.0 ships, and it cannot serve the SSH remote form, whose core runs on another machine, or client placement.
- **The changes.**
  - TR1.8 now designs Pageant's window protocol, the WinHTTP machine proxy and SSPI default credentials in the transport. TR4.8–TR4.10 implement them, and TR4.10 gets its own dual review.
  - OD16 keeps revision 2's zone bound, now applied by the transport, so no route changes after an open. The operator can still choose its unbounded alternative.
  - §3.18 changes the server design's Pageant bullet and moves the Windows logon session into every session's must-match rows. The session plan's CS8.18 and CS8.19 follow.
  - The server design's and session plan's status lines and changelogs record the change.
- **Its review:** one skim review, as for revision 3. TR1.8's and TR4.10's dual reviews carry the Windows designs themselves.
  - [Skim review 2](GwzTransportReleasePlanAmendment-2-ReviewSkim-2.md) reported NO-GO on revision 4 (`3b3b4092…`), with four P2 and four P3 text defects, and no P0 or P1. The most serious, P2-4: the text left the zone-checked URL undefined under a redirect, and its libgit2 cross-reference pointed at the configured URL. Read that way, an Intranet-zone URL that redirects discovery to an Internet host would have sent that host the logon session's NetNTLMv2 response. The check now uses the URL the credentials would go to, after the redirect.
  - All eight are applied without a further round. The same reviewer's [re-check](GwzTransportReleasePlanAmendment-2-ReviewSkim-3.md) reported **GO**, with all eight closed. Its one new P3 is applied: TR1.8's Safety finding list names the default-credential exchange with the machine proxy, should TR1.8 design one.
  - The amendment then hashes `ee130a1d0250648d39548a1ef95fd5435c7376286eee1c8000bed4faef74fb16`, the server design `9fe1738b5aefac1316de4dc98f2fe0cc45abf4ec042c4f281d82122de00d9327`, and the session plan `43952950938d88a91e9ee7291ea0709e681531f405469fb13ce56e11e941274f`. Revision 4 carries a skim-review GO, not a dual-review GO.

## Next action

OD13–OD15 are decided, and OD16's shape is applied. On the operator's go, the documents are committed, and the lanes in the amendment's §3.13 under "What can start now" can begin. TR2.9 is first: it covers the serialized closes, the measured blocker for 1.1.0.
