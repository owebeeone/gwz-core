# Remote transport native binding qualification

Date: 2026-09-20. Status: implementation candidate, not production activation.
Controlling scope: Remote Transport Design §8 and Plan Phase 3. This is Phase
3a, the prerequisite safe per-remote binding, before the SSH adapter is enabled.
Phase 1/2 interfaces remain frozen. Root baseline is
`d86c7d079b524537f6cdfdc5352b930a0202389b`, core baseline
`9303eb86914aa5770b4f951613270b14b2108f73`.

## Boundary

Pinned git2 0.21.0 exposes only process-global transport registration. Its
libgit2 1.9.7 dependency has `git_remote_callbacks.transport`. A narrow patch
inside the binding will expose:

```rust
pub fn smart_transport<S, F>(&mut self, rpc: bool, factory: F) -> &mut Self
where
    F: FnMut(&Remote<'_>) -> Result<S, Error> + 'a,
    S: SmartSubtransport;
```

The callback builds an owned `Send + 'static` subtransport. The binding itself
constructs `Transport::smart` using the actual callback owner. Returning an
arbitrary Transport created for some other Remote is deliberately not part of
this safe interface. The borrowed Remote is a temporary non-owning view; it
cannot be retained in the returned static subtransport. Callback captures live
in RemoteCallbacks for construction; returned subtransports must own their
state independently, including across disconnect and callback destruction.
Existing progress/authentication callbacks keep the same payload and behavior.

This is a construction callback: libgit2 invokes it only when that Remote has
no existing transport. It does not replace a retained transport on reconnect.
GWZ must use a fresh Remote for a new operation context/route. The proof must
exercise retained transport behavior rather than claiming reconfiguration.
Normal/unconfigured remotes use native selection. Factory errors preserve
message/code/class. Panics are caught before the C boundary and resumed through
git2's existing panic machinery at the Rust call boundary. No current-operation
thread-local lookup, private-layout cast, URL rewrite, or global registration is
added. Existing upstream panic bookkeeping is not an operation routing facility.

## Qualification and packaging

Keep the patch and mandatory public tests in `tests/transport_native`. Verify
the exact upstream crate archive digest, copy into a temporary directory, apply
the two-file binding patch, and test through an isolated Cargo patch. Never edit
Cargo's source cache. The production dependency/lockfiles remain unchanged.
The proof's locked dependency graph may differ only in the git2 source/checksum
fields when patched locally; verify that before running locked offline tests.
No public build depends on the private evidence member. This retained gate
runner covers a new native callback/lifetime boundary which the existing
message/pool tests cannot exercise. Build artifacts stay in the proof's ignored
target directory. There is no remote fork/provisioning/publication in this step.

Tests must cover named, anonymous and clone-created remotes; owned context/drop;
stateful discovery/exchange continuity; factory error/panic; nested/concurrent
routes; other callbacks; and ordinary/foreign custom transport coexistence.
Local Git service subprocesses may act as controlled fixture servers. They are
not a production Git CLI fallback, SSH implementation, or authentication proof.
A foreign registration test must isolate its registry mutation in its own test
process and register before workers start; the patched binding never registers.

Budgets: at most 150 production patch additions, 550 test additions (20% allowance, at most 660), 200 runner
lines plus concise docs; imported upstream bytes are temporary, not vendored.
Lane owner owns patch/runner/docs; retained economical drafter owns tests only.
TDD begins with stock binding compile refusal of the new safe method, then
runtime regressions and green qualification. No test-first claim is made for
already-existing upstream behavior. The exact committed correction will receive
Code/State review by the retained reviewers and Surface review for the public
binding extension. No SSH support or cross-platform parity can be advertised
from this host-local qualification alone.

## Deferred activation

Choose upstream availability or a qualified distributable dependency patch
before connecting the GWZ production adapter. Test-only patched Cargo resolution
is not registry availability. Real SSH pumping, pool integration, host-key/key/
agent parity, cancellation, native Windows/Linux evidence, every network-verb
coverage entry, performance and CLI placement remain subsequent Phase 3+ work.

## Local qualification evidence

On 2026-09-20, the stock binding rejected `smart_transport` with E0599 before
the implementation was added. After fixing fixture-only compile errors, that
was the sole compile error. This is an API compile-red claim; not a claim that
all runtime tests first failed against an equivalent stock API.

The isolated proof runner passed all seven integration tests on this macOS host
with Rust 1.95.0. Fetch/clone assert exact seed commit identity and push verifies
the destination branch's exact new commit. Nested per-remote factories and two
concurrent routes remain isolated. Retained transport survives callback/options
destruction and is dropped once with its Remote; replacement callbacks do not
reconfigure it. Native file transport works before, during and after custom
transport lifetime. A foreign registry actor is tested in its own process.
Two Python provenance checks and Rust formatting also pass.

Reproduce the integration gate using the public README command, and the runner
guards from workspace root with:
`python3 -m unittest discover -s gwz-core/tests/transport_native -p 'test_*.py'`.
The patch adds 72 upstream lines across two files. Runtime fixture growth
remains within the recorded 20% test-budget allowance. No production source or
dependency was changed; no external network or private evidence is required.
