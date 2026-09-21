# SSH agent A2 — native signing and connection handoff

Date: 2026-09-21. Status: implemented; aggregate Code/State review pending.
Authority: [accepted helper design](GwzRemoteTransportSshAgentDesign.md), A2,
and [accepted A1](GwzRemoteTransportSshAgentA1.md). This is an isolated fixture
checkpoint; production routing and dependencies remain inactive.

## Boundary and ownership

agent_auth.rs takes exclusive ownership of a handshaken SshConnection, username,
independently approved host-key bytes, the A1 cancellation/deadline control, and
a factory for its owned bounded Agent. It checks the supplied trust before opening
the agent and makes the SSH socket/session nonblocking. Enumeration occurs once;
each key is attempted once in order. Server refusal advances to the next key;
agent/protocol errors, cancellation and native failures terminate the connection.

The private binding calls pinned libssh2_userauth_publickey under Session::raw's
exclusive guard. A per-key stack callback state remains alive through every
native EAGAIN retry. No callback state or native session is cloned. Network waits
use the existing <=20 ms bounded polling fallback and the same absolute deadline.
The signing callback synchronously exchanges one bounded agent message on the
helper. It never returns callback EAGAIN; duplicate callback invocation refuses
instead of replaying. It never returns ALGO_UNSUPPORTED, which would permit the
native SHA-1 fallback. Rust unwinding is caught before crossing the C boundary.

The method is read from the native signed SSH userauth payload, validating message,
service, username, key and trailing bytes. Only Ed25519 and RSA SHA-256/512 are
admitted in A2. Agent response method must match; raw signatures must have the
Ed25519 size or RSA modulus size. Other algorithms are explicitly refused before
asking the agent to sign. No general key parser or SSH authentication emulation.

ssh2 0.9.6 Session::new passes null allocator callbacks; pinned libssh2 session.c
uses malloc/free. The Unix callback allocates with libc::malloc and transfers the
allocation exactly once, after all fallible validation. Native userauth.c frees
that allocation on both its success and allocation-failure paths. Null malloc
is handled as OutOfMemory. Windows is enclosed out of this binding until its
allocator/CRT and handle proof; no Windows behavior is claimed here.

Agent ownership ends before success leaves authenticate. A1 joins the helper
before transferring the authenticated connection. SshConnection shuts down its
TCP socket before native session destruction; failure and cancellation retain
this owner until destruction. Tests observe TCP shutdown and agent EOF before
joined disposal. The transferred connection is usable after dropping Job.
A3 still owns physical pool accounting, cleanup refusal and auth observations.

## Evidence and limits

61 focused executions pass, including seven A2 tests and 54 retained tests.
Private loopback fixtures prove Ed25519, RSA-SHA256 and RSA-SHA512 authentication,
a rejected first key, all keys rejected, wrong-host refusal before agent access,
malformed signature shape/algorithm, callback panic containment, sign deadline
and cancellation, continued independent channel work, native network wait retry
and cancellation, joined cleanup, and successful Git exchange after handoff.
Tests use generated fixture keys and an isolated ssh-agent; no user credentials.

Prepared-session handshake and known-host loading happen outside this helper in
the fixture. A2 rechecks an already approved key; production discovery, TCP/DNS,
handshake, trust-file I/O and explicit-key behavior are not qualified by A2.
No heap census, injected malloc failure, Windows/Linux primitive qualification,
or selected-source reconstruction is claimed. Platform/source work remains the
operator-deferred batch. No public API, CLI/core envelope or gwz-transport change.

246 production lines/one file. 552 test/support lines/two files: refine the A2
500-line test ceiling to 560 for the isolated native agent proxy, process cleanup
and native-wait fault tests; production remains below 350 lines/two files.
This supersedes only the A2 test line ceiling in design §8. No scope expansion.

Raw failures, passing runs and exact final source/native-source hashes are in
[agent-a2](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-agent-a2/README.md)
(private archive). Product shape failure was caught in TDD; fixture failures are
labelled separately. No known escaped defect. Retained dual Code/State gate,
P0–P2 blocking, at most two merged remediation rounds.
