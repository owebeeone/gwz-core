# Nonblocking SSH channel qualification

This unpublished fixture compiles core's preactivation
`src/git/endpoint/ssh_channel.rs` and `ssh_connection.rs` with ssh2 0.9.6. It does not enable the GWZ SSH
endpoint or change production dependencies. It qualifies channel ownership and
cleanup, not production authentication, known-host lookup, agent behavior, pool
integration or platform parity.

Use Rust 1.95.0 or newer, Git, `ssh-keygen`, `ps`, `kill`, and a local OpenSSH
`sshd` at `/usr/sbin/sshd`. A missing server fails this native gate; it is not counted as a passing skipped test. From the
workspace root:

```sh
cargo +1.95.0 test --manifest-path gwz-core/tests/transport_ssh/Cargo.toml --locked
```

The native tests use a loopback server, a high port, temporary host/client keys,
a temporary known_hosts and a temporary repository. They do not use or change
user SSH configuration, keys, agent or known_hosts. The fixture verifies its
host before authenticating with its temporary client key. Server processes are
terminated and reaped by the fixture; temporary files are removed on teardown.
The stalled-peer regression temporarily pauses only processes belonging to its
own SSH server, then resumes them during cleanup.
Build outputs remain under `tests/transport_ssh/target` and can be deleted.

## Host API

Create `SshConnection::new(TcpStream)` around the host's connected socket.
Use its `session()` setup access for handshake, host-trust verification and
authentication, then call `set_nonblocking()` (sets both socket and session).
Transfer this whole connection owner to
`SshChannel::new(connection, GitService, resolved_repository_path)`. The host must
have verified trust before authentication, and must retain no Session/Channel
clone, additional native channel, or listener that could outlive or drive the
same connection. `GitService` is UploadPack or
ReceivePack; commands are fixed and the repository operand is quoted. URL/SCP
and home-relative path resolution belong to the caller. The source currently
accepts nonempty UTF-8 paths up to 16 KiB, without NUL bytes.

Call `poll_open` until ready, treating WouldBlock as a readiness wait. Once
active, drive Read/Write and `read_stderr` from the same worker using bounded
buffers. `block_directions` supplies the session's latest socket interest; it
is a snapshot. Drain stderr even under stdout backpressure. Preserve a pending
write buffer until its bytes are accepted; do not infer progress from polling.
The host owns cancellation, socket readiness and all deadlines; this primitive
adds no timeout defaults, background thread, queue, or transport messages.

`send_eof` ends outgoing data after pending writes complete. Drain stdout and
stderr through EOF, then call `finish` until the close acknowledgement arrives.
Its exit status is an observation, not proof of Git success. `Write::flush` is
a no-op after checking active state: accepted writes already reached the native
channel; no incoming bytes are discarded. `into_session`
returns the whole SshConnection owner only after finish; otherwise it returns
the still-owned SshChannel as Err. A native failure prevents reuse. `abort` refuses further work.
Drive `poll_dispose` until it succeeds; WouldBlock retains ownership and uses
`block_directions` for readiness. At the host's cleanup deadline, call
`force_dispose`: it shuts down the socket before dropping native channel/session
owners, without waiting for the peer. A shutdown error retains ownership for
retry and is not a disposal acknowledgement. Both disposal calls are idempotent;
`is_disposed` becomes true only after release. Repeated success does not authorize
multiple pool releases. The pool reclaims capacity once, after successful
disposal. `block_directions` is None after disposal. Early Drop attempts forced
termination as a fallback; explicit disposal is the observable host contract.
Neither early drop nor abort proves reusable health.
EOF is distinct from WouldBlock or a native error. Native control errors retain
the ssh2 error as their io::Error source. Read/write error detail follows ssh2's
standard I/O implementation. No operation is automatically replayed.
