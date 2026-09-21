# SSH N2a — selected-key snapshots and native memory authentication

Status: implemented; aggregate retained Code/State review pending. This checkpoint
accepts no production activation. Controlling scope is
[SelectedIdentityDesign](GwzRemoteTransportSshSelectedIdentityDesign.md), N2a.

Three private files provide bounded container preflight, endpoint-local snapshot
ownership and native in-memory authentication. The isolated SSH fixture compiles
them; production module/dependency attachment remains N3. The fixture pins the
maintained base64 decoder at 0.22.1. No CLI/core/taut interface changed.

Every admission reserves one slot plus 1 MiB + one detection byte + 256 scratch
bytes before a supervised read starts. Opened descriptors must be regular; the
Unix nonblocking flag avoids waiting on FIFO open. Nonempty UTF-8, NUL-free,
size-bounded and unencrypted-container checks precede interning or native auth.
The registry atomically shares exact same-Key bytes including unproven candidates;
opaque monotonically increasing tokens are endpoint-local and never recycled.
Weak registry records cannot keep secrets alive. Strong owners release the actual
secret allocation before releasing its charge. Cancellation retains in-flight
read charges through real helper disposal.

The first implementation deliberately keeps each fixed read allocation fully
charged for its lifetime instead of risking an uncharged shrink/copy overlap.
Thus the 16 MiB byte budget limits distinct retained buffers to 15, before the
64-slot ceiling. Identical concurrent admissions deduplicate after their reads.
A quota refusal remains WouldBlock even if the request might match an existing
snapshot. Later memory efficiency is optional; no cap is relaxed here.

The classifier uses 135 bytes of explicit decoded scratch and a maintained
base64 engine. It accepts exactly matching armor and rejects unsupported encrypted
representations before native KDF work. It does not prove usable key material.
Native authentication rechecks host trust, uses only the immutable selected bytes,
retries EAGAIN under the original Control, and never opens an agent or alternate
path. The connection is disposed before its snapshot pin on rejected handoff.
Only a joined successful result plus a fresh original-request liveness guard
can mark the snapshot proven. Consumed helper Control is not that liveness guard.
The later physical-resource owner must retain the returned snapshot pin through
native disposal; candidate/proven status alone never grants a pool lease.

## Validation and gate

TDD missing-module reds preceded implementation. The owner corrected an exact
armor-label mismatch and native failure ownership order before review. A fixture
visibility compile failure used the existing session accessor instead of widening
its visibility. Eleven selected-key tests and five container tests pass, including
concurrent unproven interning, retention under a deliberately stuck read, slot/byte
caps, invalid/oversized input, symlink/FIFO behavior, native RSA formats/Ed25519,
path replacement, malformed native material, trust/rejection, cancellation/timeout
and stale handoff. Hostile encrypted work-factor framing never enters native auth.

The full locked/offline isolated SSH suite passes with Rust 1.95 on this Mac.
Raw reds, intermediate results and final full-suite output plus source fingerprints
are in [private evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-selected-key-n2a/README.md)
(access required). Native test fixtures are disposable localhost SSH servers.
598 production lines across three files; 868 test lines across two files, within
600/900 scope bounds. Counts describe scope, not a test-count release gate.

Next: retained aggregate Code/State review of a settled tuple, at most two merged
remediation rounds. N2b adds supervised admission before worker pool lookup,
resource pins and combined retained cleanup; its capacity-one initial fan-out
reuse gate remains outstanding. N3 attaches backend entry points. The operator's
platform/selected-source qualification batch remains deferred until those gates.

## Merged remediation 1

Code reported two P2s; State reported GO with a bounded P3. No dual-axis blind
convergence. [Remediation plan](../../dev-docs/GwzRemoteTransportSshN2a-RemPlan-1.md)
records every disposition. Ambiguous native failures now become sanitized Other,
which the actual setup boundary reports as Io; only unambiguous native auth
failure becomes Authentication. The native disconnect regression reproduced the
old incorrect classification and now passes with a single setup attempt.

Valid encrypted PKCS#8/PBKDF2 (8,388,607 iterations), OpenSSH/bcrypt (u32::MAX
rounds) and traditional encrypted PEM now prove InvalidInput, zero native dispatch
and full quota recovery. Fixtures are generated at cheap parameters, then their
framing parameters are changed; extreme KDFs are never executed. A guarded exact
wrapper around native dispatch also authenticates an unencrypted positive control.
This replaces the initial malformed-surrogate evidence for the mandatory gate.

NUL, leading-whitespace and newline scans now use the same128-byte cancellation
cadence as other classifier scans. A deterministic barrier test cancels after
exactly128 predicate calls and proves no later chunk is visited. The full isolated
suite passes after these corrections. [Remediation evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-selected-key-n2a-rem1/README.md)
requires private archive access; initial evidence is retained unchanged. Retained
Code/State closure of this settled correction is pending.
