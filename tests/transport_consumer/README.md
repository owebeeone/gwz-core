# Test-only transport consumer

This crate proves that core can consume the exported `gwz-transport` schema
without generating a second copy of its transport types. Normal builds use
only the checked-in `src/generated.rs` and the exact registry requirement
`gwz-transport = "=0.1.0"`; Cargo does not run Python or discover a schema.
The package is not published by this proof. Until that release exists,
registry resolution is intentionally deferred to the explicit archive proof.

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
isolated, offline proof. It copies only this consumer and the supplied archive
into temporary paths, and uses a temporary Cargo patch; no sibling checkout is
needed:

```sh
cargo package --manifest-path gwz-transport/Cargo.toml
gwz-core/protocol/.regen-venv/bin/python \
  gwz-core/tests/transport_consumer/package_proof.py \
  --archive gwz-transport/target/package/gwz-transport-0.1.0.crate \
  --archive-sha256 8c7d91d54f0e176109f286beec65c24544f35f23506380939fdb081e6cef87a6 \
  --source-revision 163feebe439edd5d1fbaf11e163882e80d4f2257
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
