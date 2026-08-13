# Checked-artifact semantic vectors v1

`vectors.txt` is an independently authored compatibility fixture for the
private checked-artifact recovery protocol. It is deliberately outside
`checked_artifact-corpus/`, is not an input or output of `protocol/regen.py`,
and has no retained generator.

Each non-comment line is:

```text
name|record-kind|reviewed coverage|literal canonical CBOR in lowercase hex
```

The initial literals were authored from the reviewed semantic tables and Taut
field-tag specification using a one-shot, standalone deterministic-CBOR
encoder. That authoring helper did not import the Rust adapters, generated Taut
code, or generated shape corpus and is not retained. The committed bytes—not a
recipe—are the compatibility contract.

The compiled interface test reads every literal through its bounded semantic
adapter and requires byte-for-byte canonical re-encoding. It independently
checks the complete durable-record family set, closed enum arms, durable
identity variants, path modes, catalog root kinds, managed phases and purposes,
cleanup aliases, and both maximum managed schedule layouts.

Changing a tag, enum arm, binding digest, canonical encoding, record family, or
maximum layout requires a deliberate reviewed edit to `vectors.txt`. Protocol
regeneration must never rewrite this directory.
