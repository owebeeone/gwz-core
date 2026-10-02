# SSH helper clock — remediation round 2

2026-10-03. Round-1 Consistency GO closes its original three P2 findings;
Safety closes both original P2 findings but remains NO-GO on Safety P2-3.
The reviewed object is core `d4c1f90f7b8c66c2dbbdf08451edc3db420273ea`,
root `f2db0e3b8e94f5aab7ecc0fd222b66a84637aeb7`. Complete reports are
filed verbatim. Shared-clock code remains gated; independent HTTPS work continues.

| Finding | Disposition | Closure obligation |
|---|---|---|
| Safety P2-3 | Accept: specify expired/invalid prepared-token refusal while the active authority is Alive, separately from an existing Terminal result | Disabled Network, captured 1-ms preparation, observe discards expired reservation, then publication refuses and reports boundedly without launch, latch, reset or waiting for a nonexistent terminal record; repeat with a later enabled Network deadline and active expiry/cancel controls |
| Consistency P3-1 | Fold into this correction: describe resume waits in terms of live deadlines, preserving Inactive stall | Disabled aggregate plus enabled-but-Inactive stall stays Inactive through delayed resume acknowledgement; only the next real network wait starts stall timing |

## Single bounded correction

Define the complete typed refusal outcomes and their ownership/settlement in
the existing authority/token/core-witness mechanism. Distinguish active-phase
Terminal, valid captured preparation expiry while Alive, and stale/foreign/
invalid token refusal. An Alive refusal cannot project a nonexistent Terminal.
Preserve the original captured allowance and identity; do not renew a budget,
start admission/child work, infer a helper expiry from an arbitrary unadmitted
witness, or wait indefinitely. State how core reports the valid expired
preparation and how authority/context owners settle it. Any active expiry or
cancellation that actually wins arbitration keeps its first cause; helper
metadata must not relabel that outcome. Cover a preparation for Interaction
expiring while Admission remains live as well as the initial Network case.

Fold the P3 live-deadline wording correction into the same patch. Keep common
arbitration, stall remainder, existing cause/provenance ownership, authentication
policy and numeric allowances unchanged. No new owner, synchronization
architecture, wire or public application shape is authorized.

## Closure gate

Settle one corrected draft, run its exact core commit gate, then continue the
original Safety reviewer to re-trace P2-3 and the full changed range. Continue
Consistency for the corrected typed outcome and its P3 closure. Required
implementation tests remain unexecuted obligations at this document gate.
Record prior-finding closures and any new architectural cause explicitly.

The reviewer classifies Safety P2-3 as non-architectural. Recorded architectural
cause total remains two; this is remediation round 2. A third new architectural
cause requires stopping the object for operator disposition. Prior document
GO does not certify this correction or implementation in advance.
