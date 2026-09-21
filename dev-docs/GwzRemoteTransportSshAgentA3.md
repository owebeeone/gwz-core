# SSH agent A3 — supervised pool ownership and observations

Date: 2026-09-21. Status: implemented locally; retained aggregate review pending.
Authority: accepted helper design A3 and accepted A1/A2. Local integration only;
production discovery/routing activation and deferred platform/source batch remain
separate gates. No CLI/core message or gwz-transport schema changes.

## Refined ownership boundary

The pool's connecting resource owns an A1 Job returning an A2 authenticated
connection, identity proof and existing transport Facts. A bounded factory selects
owned setup inputs before spawning; actual setup runs on the helper. The worker
and connector share one Instant origin, so converting the pool's absolute logical
deadline does not grant a fresh budget. A joined, authenticated result may become
idle; failed/cancelled results must be disposed before capacity acknowledgement.
Force means request cancellation, never pretend a running helper has stopped.

Normal ready/active ownership uses the accepted SshConnection/SshPump. Resource
Drop cancels a connecting job and leaves its physical ownership with A1's
supervisor; idle/active native owners terminate sockets before destruction.
This supersedes the old Resource comment requiring every Drop to synchronously
terminate a socket, only for a connecting job with retained supervised ownership.

A worker cleanup error closes endpoint admission and is sticky in its shutdown
status. Worker shutdown remains bounded. If cleanup is unfinished at its budget,
transfer the entire stopped pool host (including driver and charged entries) to
the existing process supervisor. It polls disposal until entries are physically
destroyed and the ledger acknowledges closure; only then report cleanup complete.
A reported cleanup error never turns into success, even after eventual completion.
Worker panics are caught while the host remains owned; recover by shutdown and
retained cleanup. No join of a live setup helper runs on the endpoint worker.

Supervisor admission reserves at most 64 active-or-retained endpoint cleanup
owners, separately from A1's 64 setup helpers. Reserve before worker creation;
retention consumes its existing reservation, so shutdown cannot fail to enqueue
owned cleanup or expand capacity. Endpoint recreation cannot evade this bound.
One existing supervisor thread polls jobs and retained pools; no extra reaper
thread per endpoint. Polls and destructors must be bounded and non-panicking.
The supervisor must keep polling retained pools even when setup-helper count is
zero. No endpoint/helper thread may be detached and forgotten.

## Observation boundary

Use existing typed Opened/Facts for the per-remote operation receipt. The first
exchange carries proven authentication and actual credential-offered facts;
subsequent exchanges on that native connection explicitly mark reused and clear
credential_offered while retaining authentication proof. A per-operation Route
observer receives its own receipt, even when Routes share an endpoint. Healthy
close carries the same facts; a pool hit must never fabricate a fresh key offer.
Unknown facts stay unknown for legacy injected resources. Backend activation
will attach these receipts to its operation-scoped observation sink; this slice
qualifies that internal binding without changing generated public observation
fields or activating unqualified production setup.

## Limits and gates

At most 750 added/changed production lines across setup resource, worker, pool,
pump, route and supervisor; at most 800 new focused test/support lines. Existing
bounded native fixture helpers may be reused. Refine before expanding discovery,
explicit keyfile support or production call-site activation.

TDD: native agent authentication through pool/worker/per-remote Git; reuse through
independent Routes with distinct observations; helper failure; exact/shared
deadline; cancellation with disabled timeout; forced cleanup overrun keeps pool
counts and refuses opens; bounded endpoint Drop; supervisor owns late cleanup;
eventual acknowledgement; healthy shutdown; failed shutdown remains failed;
endpoint admission bound and recovery. Retained Code/State aggregate review on a
settled tuple; P0–P2 block, at most two merged remediation rounds.

## Local results

Full isolated Rust 1.95 locked/offline gate passes. New coverage includes native
signing and pool reuse, two independent per-remote Git clone receipts, stalled
signature cancellation with timeouts disabled, failed setup/capacity recovery,
original-origin deadline, cleanup overrun with admission stopped automatically,
bounded endpoint destruction with physical charge retained, sticky failure after
eventual cleanup, factory panic recovery and the separate cleanup-owner cap.

TDD found a dequeued Connect could remain unowned after a factory panic. The
start boundary now settles that ledger entry before resuming unwind into worker
shutdown; existing owned resources remain in the retained host. Connector::start
must unwind without residual physical ownership, just as for its error return.
The native setup factory runs before helper creation, so its injected panic owns
no helper; a successfully created Job immediately transfers to NativeResource.
The native resource and all later resource polls remain bounded as contracted.

627 added production lines across seven files, 37 removed; 481 new focused test
lines across two files plus existing fixture wiring. No ceiling expansion.
[Raw red/green evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-agent-a3/README.md)
requires private archive access. Review findings and exact acceptance tuple will
be filed after retained Code/State review. No known escaped defect; one owner
regression discovered and corrected the panic ledger defect before review.
