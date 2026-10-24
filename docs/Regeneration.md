# Regeneration

Protocol generation has two tracks:

- release protocol regeneration from `protocol/gwz.taut.py`;
- docs catalog regeneration from the same schema.

Run commands from `gwz-core/` unless noted.

## Protocol Bindings And Corpus

```text
python protocol/regen.py --check
python protocol/regen.py
```

`protocol/gwz.taut.py` composes the transport owner's pinned export (`gwz-transport/protocol/transport.ir.json`, beside gwz-core), and the owner's declarations are generated as re-exports from `gwz_transport::protocol`; there is no vendored `src/cbor.rs`, the protocol shares `gwz_transport::cbor`.

`protocol/regen.py` provisions a `uv` virtual environment, installs the released
`taut-proto==0.8.1` interface-checkpoint release, and regenerates:

- `src/protocol/generated.rs`;
- `protocol/corpus/golden.json`;
- `protocol/corpus/rust/vectors.rs`.

Use `--check` in CI or local drift checks. A nonzero result means committed
generated artifacts do not match the schema/generator.

Useful options:

```text
python protocol/regen.py --check
python protocol/regen.py --recreate
python protocol/regen.py --taut-version 0.8.1
python protocol/regen.py --venv protocol/.regen-venv
```

## Message Catalog

The docs catalog is generated from the taut IR, not from generated Rust:

```text
python docs/generate_message_catalog.py
```

The generator writes `docs/MessageCatalog.md`. `--check` writes nothing and exits 1
when the file differs from a fresh render; `scripts/checks/test_message_catalog.py`
covers the Optional column and runs `--check`. The generator uses the vendored `taut/src`
tree when present and sets `SETUPTOOLS_SCM_PRETEND_VERSION=0.6.0` as an import
workaround for non-version GWZ Git tags. This affects catalog inspection only;
release protocol generation still uses `protocol/regen.py`.

## Verification

Recommended checks after schema or docs changes:

```text
python docs/generate_message_catalog.py
git diff --exit-code -- docs/MessageCatalog.md
python docs/generate_message_catalog.py --check
python protocol/regen.py --check
cargo test
```

If a schema edit adds a request, response, enum, or message, update docs that
describe behavior and review the request/response matrix in the catalog
generator.
