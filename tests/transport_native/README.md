# Per-remote git2 binding qualification

This unpublished test package qualifies a proposed patch to git2 0.21.0.
It is not linked into GWZ production, and does not enable SSH or pooling.
It needs Rust 1.95.0 (rustup), Python 3.10+, Git, and the native build tools
already required by git2/libgit2. Git also supplies local upload-pack and
receive-pack fixture servers. Tests must not contact real SSH endpoints.

## Proposed safe API

```rust
callbacks.smart_transport(false, move |remote| {
    // Inspect the selected Remote; return an owned SmartSubtransport.
    // Clone owned operation/endpoint context into that subtransport.
    Ok(MySubtransport::new(context.clone()))
});
```

The first argument is `rpc`: false preserves a stateful stream across discovery
and negotiation (SSH-style); true requests stateless exchanges (HTTP-style).
The caller must supply it; there is no implicit mode. The factory returns a
`SmartSubtransport`, whose trait requires `Send + 'static`. Only this Remote is
configured; there is no transport registry mutation. Other remote callbacks,
such as credentials or progress, remain independently configured.

The borrowed Remote is only available during construction. Callback captures
may borrow for the callback/options lifetime, but the returned subtransport and
streams must own their state independently. The binding constructs the native
transport using the actual owning Remote and transfers ownership to libgit2.
Errors preserve their message/code/class. A panic is caught before crossing C
and resumed when the enclosing git2 operation returns to Rust; it is not silently
converted into success or retried.

The factory is called only if libgit2 needs to construct a transport. Disconnect
does not necessarily destroy that transport. Reconnecting the same Remote may
reuse its original transport even with different callbacks. Use a fresh Remote
to change operation context or route. Disconnect/close ends the conversation;
dropping the Remote frees its retained transport and owned context. There is no
registration to undo and no background service installed by this package.

## Reproduce

From this directory, fetch the exact locked stock dependencies once:

```sh
cargo +1.95.0 fetch --locked
```

Then, from the workspace root, explicitly select the cached upstream archive:

```sh
python3 gwz-core/tests/transport_native/prove.py \
  --git2-archive "$HOME/.cargo/registry/cache"/index.crates.io-*/git2-0.21.0.crate
```

If several registries match that glob, supply one exact path instead. The runner
verifies the archive and patch digests in `binding-pin.json`, extracts into a
temporary directory, applies the binding patch there, and copies this fixture.
It verifies that the lock graph changes only from registry git2 to the patched
local git2, then runs locked offline tests. It does not edit Cargo's cache, the
checked-in lockfile, or GWZ production dependencies. Normal Cargo tests directly
against this package's stock manifest are expected to reject the missing new
method; use the proof runner to qualify the patch.

Temporary sources are removed on success or failure. Build outputs remain in
this package's ignored `target/qualified` for reuse; deleting that directory
cleans up all retained build products. No credentials, services or remote
resources are created. A future upstream release or distributable pinned patch
must be selected before production activation. Results on one native host do
not establish Windows/Linux/macOS parity or real SSH trust/authentication.
