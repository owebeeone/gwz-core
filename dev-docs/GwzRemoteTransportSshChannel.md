# Nonblocking SSH channel primitive

Date: 2026-09-20. Status: implementation candidate, preactivation.
Scope: Remote Transport Design §8, after accepted AdapterFoundation. This
primitive owns an already trusted/authenticated SSH session for one Git command
channel. The native fixture authenticates only with temporary loopback keys and
checks its temporary known_hosts before authentication. Production connection,
trust/identity resolution, agent handling and pool integration remain subsequent
work; this primitive cannot advertise an endpoint capability by itself.

`SshChannel::new` takes sole ownership of an authenticated, nonblocking ssh2
Session. The caller must not retain Session/channel clones. `poll_open` retries
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
`cargo +1.95.0 test --manifest-path gwz-core/tests/transport_ssh/Cargo.toml --offline --locked --test channel -- --nocapture`.
Formatting passes. The missing-module compile failure preceded implementation;
real native behavior tests were completed against the candidate, so no claim is
made that each native behavior test preceded its implementation. Source is 221
lines, fixture 311 lines. No production dependencies, module wiring or active
network entry changed. This is not a throughput benchmark, pack transfer test,
SSH-agent/known-host policy parity result, or cross-platform qualification.
