# Per-remote git2 binding qualification

This unpublished test package qualifies the per-remote binding patch to git2
0.21.0 and, in source mode, the native local-fetch correction on libgit2 1.9.7.
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

Alternatively, qualify the registered patched member checkout:

```sh
python3 gwz-core/tests/transport_native/prove.py --git2-source git2-rs
```

Exactly one of `--git2-archive` and `--git2-source` is required; neither input
has a default. Archive mode retains the released registry sys 0.18.8+1.9.7 and
characterizes its known noncommit-hint fetch failure. Source mode requires:

- the git2 0.21.0 release commit `dffaf272eb0e62ac15b74283c4e488252db9afc3`;
- sys source commit `6c93812dbc1c34aef6e6464a645545b4a4299807` (0.18.8+1.9.7);
- the initialized C submodule at the exact patched commit in `binding-pin.json`.

The Rust checkout must match the release plus the two pinned binding files,
exact sys baseline, path dependency and operator-fork submodule URL. The C
checkout and parent gitlink must both match the pinned commit. All source paths,
bytes and modes are checked against Git objects before an isolated copy is built.
Unrelated edits, extra files (even ignored build inputs), missing files and
file/symlink substitutions are refused. Only root `.git`, root `target/`, and
nested C `.git` metadata are omitted. Both modes force vendored C; source mode
also proves that the shared-object fetch regression now succeeds.

The forks remain unpublished. In this workspace the C submodule is initialized
from the sibling `libgit2` checkout containing the unpublished backport commit.
Its tracked URL is the operator's GitHub fork, but a fresh remote-only clone
cannot yet obtain that unpublished commit. Use the prepared workspace for source
qualification; publication and clean remote-only reproduction remain later gates.
No member files or registry caches are changed by the proof runner.

The runner selects Rust 1.95.0 by default. `--toolchain TOOLCHAIN` selects a
different rustup toolchain for additional qualification; it does not replace
the recorded Rust 1.95.0 baseline.

If several registries match that glob, supply one exact path instead. The runner
verifies the archive and patch digests in `binding-pin.json`, extracts into a
temporary directory, applies the binding patch there, and copies this fixture.
It verifies that the lock graph changes only from registry git2 to the patched
local git2, plus registry sys to exact local sys in source mode, then runs locked
offline tests. Versions and all unrelated dependency entries remain unchanged. It does not edit Cargo's cache, the
checked-in lockfile, or GWZ production dependencies. Normal Cargo tests directly
against this package's stock manifest are expected to reject the missing new
method; use the proof runner to qualify the patch.

Temporary sources are removed on success or failure. Build outputs remain in
this package's ignored `target/qualified` for reuse; deleting that directory
cleans up all retained build products. No credentials, services or remote
resources are created. A future upstream release or distributable pinned patch
must be selected before production activation. Results on one native host do
not establish Windows/Linux/macOS parity or real SSH trust/authentication.
