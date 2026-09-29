# gwz-session-host

The machinery of GWZ's core session host, generic over the session data that
gwz-core keeps. It holds the operation gate, the session limits and the host
context's supervisor, which the core session crate map moves out of gwz-core
(gwz-dev `dev-docs/GwzCoreSessionCrateMap.md` §2 and §6 step 4). Its authority
is the core session contract (`dev-docs/GwzCoreSessionDesign.md`, revision 5).
Phase 2 of the session plan adds the host itself here: `serve` and the
`HostPorts` that gwz-core implements.

It encodes and decodes no GWZ message, and none of its errors are gwz-core's.
Core's per-session data is a type parameter, `S`, that it never looks inside,
so the environment snapshot stays in gwz-core. It keeps no global state: no
static and no thread-local.

## The operation gate

- `CallControls::new(&Arc<S>) -> (CallControls<S>, OperationGate<S>)` creates a
  call's token, its record's controls and its gate together, when the host
  reads the call's frame.
- `CallControls<S>` is the record's authority: `cancel()`, `revoke()` and
  `view()`. `revoke` cancels first, then returns once no crossing is in
  flight; after it, nothing crosses the gate.
- `OperationGate<S>` is the worker's. `effect` and `append` are refused with
  `Refused::Cancelled` once the token is cancelled, and with
  `Refused::Revoked` after revocation. `report` still lands after
  cancellation and returns `None` after revocation. Each crossing takes
  `&mut self` and hands its closure a `GateScope`.
- `GateScope<'_, S>` exposes `session() -> &S`, and nothing that crosses or
  revokes.
- `GateView<S>` clones, and gives `state()` (`Live`, `Cancelled` or
  `Revoked`, read without a lock) and `token()`.
- `CancellationToken` clones, and gives `is_cancelled()` and
  `on_cancel(FnOnce() + Send + 'static) -> Result<CancelRegistration, Refused>`.
  A callback runs once, on the cancelling thread, and receives nothing.
  Dropping its registration detaches it.
- `HandlerContext<S>` owns the gate: `gate()`, `token()` and `view()`.
- A gate holds its session weakly. Once the session's data has dropped, the
  gate acts revoked.

Nesting is stopped by capabilities. `CallControls`, `OperationGate`,
`HandlerContext` and `GateScope` are not `Clone`, so each capability has one
holder. A closure cannot reach the gate it runs in, which it borrows
mutably, and a `'static` callback cannot borrow a gate. As a backstop, each
gate records the thread running its closure, and a revoke or crossing of that
gate from that thread panics instead of deadlocking. What stays undetected is
controls smuggled into a closure or a callback, such as another operation's
gate, and review checks that no such path exists.

### Why a cancel callback receives nothing

The core session crate map (§2) says that a crossing's closure and a cancel
callback "receive only a `GateScope`". Here the closure does, and a cancel
callback receives nothing, as in CS1.4. That is narrower, and it guards the
same thing: neither can cross a gate or revoke one. The reasons:

- Every planned callback needs no session data: a transport request
  signalling its runtime (session plan CS3.7), a credential helper's kill
  (contract §5.8) and a lock wait's wake (§5.1). CS3.7's callbacks may not
  report through a gate at all.
- A scope needs the session alive when the token is cancelled, and a gate holds
  its session weakly, so a callback's scope would have to be optional.
- A callback runs on the canceller's thread, the host's reading thread for
  `operation.cancel`, where session data would invite a session lock, which
  the rule forbids and which could stall control frames.

The literal alternative, if the review wants it, is a local change:
`on_cancel(impl FnOnce(Option<GateScope<'_, S>>) + Send + 'static)`, with
`None` once the session has ended. The token would then be generic over `S`,
and so would everything that holds one, a transport request included.

## The limits

- `Limits` carries the contract's §1 limits and defaults. It is
  `non_exhaustive`: start from `Limits::default()`.
- `MAX_READ_WAIT` is 30 seconds. The frame limit is `gwz-session-contract`'s
  `MAX_FRAME_BYTES`.
- `validate_limits(&Limits) -> Result<gwz_session_contract::Limits, InvalidLimits>`
  applies `open`'s rules and returns the two limits the channel takes. Each
  `InvalidLimits` has the text of gwz-core's `invalid_request` refusal.

## The supervisor

- `Supervisor::new()` starts no thread until `supervise(Box<dyn SupervisedJob>)`
  hands it a first job.
- `supervise` refuses with `SuperviseError::ShutDown` after `shutdown`, or with
  `SuperviseError::Spawn` if the thread cannot start, and drops the job
  unpolled either way.
- A job whose poll panics is quarantined: never polled again, and dropped
  when the thread stops.
- `shutdown(bound) -> usize` closes it to new jobs and waits at most `bound`
  for its jobs. It returns how many remain, still running or quarantined, and
  the thread polls the running ones to their end. A later call returns the
  same count, and a concurrent one waits for the first.
- Dropping the supervisor releases its thread without waiting.
- The `test-support` feature, which only gwz-core's dev-dependency enables,
  adds `test_support::watch`, a view of the thread.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
