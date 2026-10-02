# SSH helper setup clock — remediation round 1

2026-10-03. Status: NO-GO at draft core
`eb06fac24a3e1eb8f4db32de261a26627e1fd01b`, root
`5d89a096dd0688e5b82b501a6ce997dcfae060f1`. Complete initial
Consistency and Safety reports are filed verbatim. No shared-clock code is
authorized before root-relayed review GO; independent HTTPS implementation
continues. This is one combined document correction, not a patch series.

## Verdict merge and cause accounting

Both axes independently found the same transition/expiry arbitration gap and
the same undefined stall resumption. Consistency additionally found missing
helper-timeout provenance on pool-first expiry. The reviewers classify the
first and third causes as architectural, and stall resumption as a bounded
contract omission: two architectural root causes on this object. This is the
first remediation round; the normal two-round cap applies. A reviewer finding
a third new architectural cause stops this object for operator disposition.

| Finding | Disposition | Required closure |
|---|---|---|
| Consistency P2-1; Safety P2-1 | Accept: define one linearization rule for publication, expiry, cancellation, terminal state and acknowledgement, covering every reader | Timely pause, delayed PoolHost, Control-first expiry observation must not latch displaced Network expiry; late publication, cancellation and expired local acknowledgement cannot launch or resurrect |
| Consistency P2-2 | Accept: define core-owned captured timing provenance and a neutral connection/phase identity bridge through every expiry/reporting winner | Pool-first, Control-first and helper-first outcomes have identical code/setup cause/exact captured allowance for Allocation and Interaction; zero allocation and generic non-helper timeout are distinguished |
| Consistency P2-3; Safety P2-2 | Accept: pause/resume the live stall remainder, preserving disabled and inactive states, without manufacturing network progress | Local work longer than the original stall interval succeeds; resumed inactivity expires at the retained remainder; pre-entry expiry cannot be revived |

## One correction and ownership requirements

1. Specify the actual common arbitration primitive, visibility boundary,
   timestamp source and acknowledgement owner. It must resolve every already
   published transition before any of PoolDriver, Control or post-result
   classification commits expiry. A shared deadline pointer alone is not
   arbitration. Bound pending storage, settlement on driver loss/disposal and
   acknowledgement waiting. State lock order; perform waits, callbacks, Git,
   filesystem and child work outside arbitration locks. Cancellation remains
   effective before acknowledgement and launch. Delayed acknowledgement
   cannot outlive the acknowledged phase's own deadline.
2. For the shared path, define network aggregate and stall transition state
   under that authority. Capture any live stall remainder at a valid local
   entry; retain inactive/disabled states; restore the remainder on Network
   resume before any fresh Control check. Local activity is not network
   progress and cannot reset that allowance. Define repeated local entries
   and exact-boundary behavior. Existing non-shared Control users retain
   their original contract.
3. Add the precise core-owned per-connection/per-attempt timing witness,
   created from the same values applied to the timers. Keep helper policy,
   classification, secrets and FailureDetail in core. Generic transport may
   carry neutral phase/connection identity, with explicit owner/lifetime,
   allowing core to project the witness even when generic pool expiry wins.
   Include ssh_worker/EndpointOpenFailure::capture and the pool error bridge
   in affected owners and supersession. No inference from absent detail or
   generic timeout strings; terminal reporting cannot lose or replace the
   first authoritative cause. Keep this witness through logical reporting
   and retained physical cleanup. Define zero-allocation refusal too.
4. Revise the DRAFT's interfaces, phase transitions, owner/call graph,
   supersession and regression obligations together. No new authentication
   policy, wire field, public GWZ request/response or protected dependency
   change is authorized. Do not turn the generic pool into a helper-policy
   owner.

## Closure and re-review

Settle the single corrected draft and exact tuple through GWZ. Run its exact
core commit gate. Implementation tests are required obligations at this
document gate, not claimed execution; final source acceptance must execute
the actual Control/pool interleavings and retained-cleanup paths.

Continue the original reviewers for full changed-mechanism re-review and their
own counterexample closures, following the operator's standing direction to
use old reviewers. Require prior-finding tables and changed-range analysis;
reviewers classify any new architectural cause. Their previous proofs do not
certify the correction in advance. Root files reports verbatim and relays GO
only after every blocking finding closes. Final secret Code/State and combined
Surface implementation review remain separate and owed.
