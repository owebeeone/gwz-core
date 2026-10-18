# Nonblocking SSH channel primitive

Date: 2026-09-20. Status: **accepted preactivation primitive at core
`f03f5f79bae73d378e575273af0b9ed2a87c052d`, root
`6076c6153f2b4fb74da5179b0ec6ffd2765d81ad` after original Code/State/Surface
reported GO in root `dev-docs/GwzRemoteTransportSshChannel-Review{Code,State,Surface}-1.md`.
This accepts channel/connection ownership and native cleanup only.**
Scope: Remote Transport Design §8, after accepted AdapterFoundation. This
primitive owns an already trusted/authenticated SSH session for one Git command
channel. The native fixture authenticates only with temporary loopback keys and
checks its temporary known_hosts before authentication. Production connection,
trust/identity resolution, agent handling and pool integration remain subsequent
work; this primitive cannot advertise an endpoint capability by itself.

`SshChannel::new` takes sole ownership of a SshConnection containing an
authenticated, nonblocking ssh2 Session and the socket shutdown capability. The caller must not retain Session/channel clones or other native channels/listeners. `poll_open` retries
only the in-progress native open/exec transition, preserving the same command;
it never starts a second command after execution. The command is one of the two
Git services and its repository operand is shell-quoted with a separate `--`.
The caller supplies the resolved remote path; URL/SCP/tilde parsing is elsewhere.

Read/Write and stderr draining are nonblocking and driven by one host worker.
WouldBlock preserves progress, EOF is distinct, and caller buffers bound memory.
The worker owns readiness polling, both directions, cancellation and deadlines.
It must drain stderr even when stdout is backpressured. No native work enters
gwz-transport and no message or CLI/core interface is added.

Send EOF once all request bytes have been handed to the channel. Finish is
permitted only after local EOF and both response streams have drained; close and
wait-close must complete before the session can be extracted for a new lease.
A native error poisons the primitive; early drop/abort must not return a session.
`poll_dispose` retains ownership on native close WouldBlock. `force_dispose`
terminates the socket before dropping native objects; it is the host-deadline
fallback. Completion is observable through successful disposal/is_disposed and
only then permits one pool capacity release. Drop attempts forced disposal as a
fallback. The socket owner also protects idle and partially opened disposal.
`Write::flush` checks active state but never calls native channel_flush, which
discards incoming bytes; accepted writes have already reached the native channel.
Channel exit status is an observation and cannot replace Git's result parsing.

A public standalone fixture compiles the exact preactivation core module with
ssh2 0.9.6. It should prove real loopback SSH traffic, command quoting, complete
channel cleanup and two successive command channels on one connection. Tests
must keep all server keys/config/repositories temporary and bound waits. Record
unsupported native fixtures honestly; no Linux/Windows parity is inferred.
Budget: 250 primitive source lines and 350 test/fixture lines plus small package
metadata/docs. Review tier dual Code/State with Surface if API frozen. No
production activation, registry publication, agent or host-trust parity claim.

## Local evidence

On this macOS arm64 host, OpenSSH 10.3p1 and ssh2 0.9.6 passed two real
loopback fixture tests. Receive-pack performed two complete command lifetimes
using one authenticated session; upload-pack completed advertisement/flush/EOF,
refused premature session extraction, and an additional active channel was
aborted without yielding its session. A shell-special repository path did not
create the injection marker. The fixture checks independently generated host
trust before authentication, bounds handshake/auth and channel loops, and drains
stdout/stderr into capped buffers. Server availability is required; a missing
sshd fails rather than silently passing a skipped test.

Command from workspace root:
`cargo +1.95.0 test --manifest-path gwz-core/tests/transport_ssh/Cargo.toml --offline --locked -- --nocapture`.
Formatting passes. The missing-module compile failure preceded implementation;
real native behavior tests were completed against the candidate, so no claim is
made that each native behavior test preceded its implementation. The initial source was 221
lines and fixture 311 lines; remediation sizing is recorded below. No production dependencies, module wiring or active
network entry changed. This is not a throughput benchmark, pack transfer test,
SSH-agent/known-host policy parity result, or cross-platform qualification.


## Round 1 correction scope

Code and State independently found that ssh2's nonblocking destructors discard
EAGAIN and can leak on cancellation. The lane owner separately found native
flush discarding unread stdout. Root RemPlan maps both to concrete regressions.
The channel grows within the existing 20% budget allowance; the new 53-line
connection owner is a separate socket/native lifetime boundary, not a carrier.
Production connection policy, pumping and pooling remain deferred. Original reviewers closed their counterexamples in round 1; all three axes
reported GO with no remaining P0–P3.

Correction qualification on the same macOS host: five native tests pass (two
lifecycle tests and three regressions), with no failures, ignores or warnings.
The flush test failed against the old implementation before correction and now
compares the complete advertisement byte-for-byte. The stalled-peer regression
pauses the fixture-owned process tree, drains queued output, observes two close
WouldBlock results with ownership retained, then completes forced disposal.
Normal disposal, idempotence and refusal to extract a disposed connection pass.
The process-resume guard is armed before stopping any child and does not panic
while unwinding. Source: channel 270 lines, connection owner 53. Test code is
split by lifecycle/shared fixture/regressions and remains within the combined
350 + 180 line allowance plus its 20% margin. Disposal behavioral tests were
written during correction, not before the new disposal API; do not claim a
behavioral red-before-implementation sequence for that API. Formatting and diff
checks pass. Both original blocking findings and the independent flush defect are closed
by the original reviewers; all three axes returned GO on the exact tuple above.
