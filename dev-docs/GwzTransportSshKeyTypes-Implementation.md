# TR2.8 — SSH keys and signatures implementation

Date: 2026-10-02. Status: implemented checkpoint, pending Code/State review.

## Scope and authority

[Transport release amendment 2](GwzTransportReleasePlanAmendment-2.md) §3.19
controls this step. The baseline is gwz-core
`bb67a8264a71a5141d3345a5d1367f4228aeb5db`; the inventory was committed first
at `485546f`. [The key inventory](GwzTransportSshKeyTypes.md) records source
citations, the platform distinctions and the test matrix.

The private signing bridge now admits Ed25519, RSA, the three NIST ECDSA
curves, DSA, both OpenSSH security-key types, and the supported types'
certificates (excluding DSA certificates). It validates signature framing
before handing bytes to libssh2. Security-key flags and counters are retained.
RSA flags request SHA-2, including certificates. SHA-1 is used in exactly the
three cases in amendment 2 §3.19; an agent's SHA-1 response to a SHA-2 request
allows one fallback for that key. Malformed responses terminate authentication;
an explicit agent refusal permits trying the next listed key.

Selected private-key files now accept the five container forms the inventory
identified as parity gaps. Reads remain bounded and cancellable, encrypted
containers are refused before native authentication, and native signing still
uses the captured key bytes rather than reopening the path.

No application protocol, transport stream protocol, dependency or public API
changes. The existing Unix allocator/agent bridge boundary remains: Windows
agent primitives and platform qualification belong to Phase 4. The new
software-authenticator fixture is Unix-scoped explicitly.

## Automated validation

The focused candidate SSH suite passed: 148 passed, 0 failed, 3 ignored
child-only tests (run by their parent tests). Disposable OpenSSH 10.3p1 and
the standard-library Python server exercise certificates, security keys,
agent refusal, selected-key formats, and the three SHA-1 cases. Full suite
results are recorded in the acceptance section once complete.

Build manifest: `/Volumes/projects/limbo/gwz-tr2-8-candidate-20261002`.
Candidate target: `/Volumes/projects/limbo/gwz-tr2-8-target-20261002`.
Local gate logs: `/Volumes/projects/limbo/gwz-handoff-2026-10-02/logs/tr2-8-final/`
(local handoff access required; these paths are not CI dependencies).

Production additions at takeover were about 397 lines, within the aspirational
500-line step budget; fixtures and tests are reported separately. No new
production owner or protocol delta was introduced.

## Remaining release qualification

The hardware security-key row requires the operator's explicit go under
amendment 2 §3.19. It has not run; the software authenticator covers both
security-key algorithms and their certificates automatically. Linux and Windows
release qualification remain in the plan's platform batch. Passing this lane's
review accepts its implementation, not those outstanding release rows.
