# Test-only transport consumer

This crate proves that core can consume the exported `gwz-transport` schema
without generating a second copy of its transport types. Normal builds use
only the checked-in `src/generated.rs` and the exact registry requirement
`gwz-transport = "=0.1.0"`; Cargo does not run Python or discover a schema.
The package is not published by this proof. Until that release exists,
registry resolution is intentionally deferred to the explicit archive proof.

## Prerequisites

Run these commands from the workspace root. Install Python 3.10 or newer with
venv/pip, Git, and Rust through rustup. Use Rust 1.96.0 with its rustfmt component
for regeneration (the exact formatter version is pinned in the manifests);
the Rust package supports 1.95.0 or newer. Keep the canonical `taut` checkout at
the revision recorded in `protocol/generator.json` within this consumer.

Create the interpreter used by the commands below once:

```sh
python3 -m venv gwz-core/protocol/.regen-venv
gwz-core/protocol/.regen-venv/bin/python -m pip install taut-proto==0.9.1
rustup toolchain install 1.96.0 --component rustfmt
export RUSTUP_TOOLCHAIN=1.96.0
```

An existing environment is usable only with the same pinned package version.
Alternatively create a fresh environment elsewhere and replace the interpreter
path in both command blocks with its Python executable. The installed package
supplies version metadata; consumer generation loads and verifies the exact
canonical checkout passed through `--taut-source`, rather than trusting installed
generator code. Cargo's dependency cache must contain this consumer's locked
dependencies before the offline archive proof; packaging performs the owner
build first. No private environment or credentials are required.

## Regeneration and archive proof

Regeneration is explicit and checks the owner package/version, exported-schema
digest, taut source revision, and external-type generator file hashes:

Supply the verified checkout's canonical `src` directory. Run the command in a
fresh interpreter; cached taut modules are refused, and imported module paths
must resolve inside that exact source directory.

```sh
gwz-core/protocol/.regen-venv/bin/python \
  gwz-core/tests/transport_consumer/protocol/regen.py \
  --owner-schema gwz-transport/protocol/transport.ir.json \
  --taut-source taut/src --check
```

After `gwz-transport` is actually released, refresh this proof crate's lockfile
against the registry package, then run the focused proof with:

```sh
cargo test --manifest-path gwz-core/tests/transport_consumer/Cargo.toml --locked
```

The checked-in lockfile currently records the temporary local package proof;
it is not evidence of registry resolution and must be intentionally refreshed
for the first published release.

For the current unpublished source, first create an archive and then run the
isolated, offline proof. It copies this consumer, the core blocking-adapter source identified below,
and the supplied archive into temporary paths, and uses a temporary Cargo patch; no sibling checkout is
needed:

```sh
cargo package --manifest-path gwz-transport/Cargo.toml
gwz-core/protocol/.regen-venv/bin/python \
  gwz-core/tests/transport_consumer/package_proof.py \
  --archive gwz-transport/target/package/gwz-transport-0.1.0.crate \
  --archive-sha256 986033108eab2967028dc52c69f94e859ed6cbb78384648f03e88d9703383191 \
  --source-revision 28f5afb3938a2aa8af0e1e8d5b07779add6ab776
```

The runner prints the source revision, package identity, archive digest, and
exact offline Cargo command before running the consumer suite. It covers all
transport envelope variants, Bind/Open admission before fake effects, paired
typed/encoded stream lifecycle cases, deadline failures, and fake-host clock and
pool-lifetime duties. Open network deadlines cover native disabled and maximum
positive values. A clock timeout preserves the byte prefix and structured
`Timeout` / `Possible` failure across the generated wrapper.

The serialized stream case treats its generated outer wrapper as a trusted
fixture and applies the owner transport codec's bounded encode/decode to the
inner envelope. Pre-allocation limits for any real carried wrapper remain a
duty of the supplied communication layer.

## Blocking Git adapter qualification

The consumer also compiles core's preactivation
`src/git/endpoint/stream_io.rs`. It is not yet linked into production core. The
archive runner copies that exact module from this core checkout into its isolated
tree and prints its SHA-256 alongside the transport archive provenance.

`BlockingStream::new(Stream)` provides std::io `Read` and `Write`, including the
standard `write_all` helper. Writes may be partial. Clones share one exchange
and its byte positions; they do not allocate a second connection. The adapter
adds no buffer, physical carrier, thread, socket or implicit deadline. The host
must deliver messages and drive the accepted stream's timers independently;
never run a blocking call on that sole host worker. Small writes rely on that
host timer even when libgit2 never calls `flush`.

`end_write` half-closes outgoing bytes; reads remain available. `close` waits
for cleanup and returns the endpoint's CloseResult, including disposition/facts;
it does not prove Git success or release a physical connection itself. `cancel`
wakes blocked calls and requests cancellation. Dropping the final Stream owner
requests cancellation; dropping one clone leaves other owners live. There is
no blocking cleanup inside Drop and no inferred connection reuse.

EOF returns zero. Timeout is `TimedOut`; delivery loss is `BrokenPipe`; cancellation
is `ConnectionAborted`, so std::io `write_all` cannot retry it as Interrupted.
The underlying structured stream Error is retained in `io::Error::get_ref()`
for downcasting, including peer failure code/effect. Protocol errors are
`InvalidData`; caller misuse is `InvalidInput`. Other failures remain `Other`.
All buffering and deadline defaults are those of the supplied Stream config.
