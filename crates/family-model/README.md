# gwz-family-model

The pure family model of the GWZ local clone family. A family is one original
workspace plus its named local clones, and this crate owns the deterministic
values and decisions of that model: names, root-relative member paths, ids,
rows, the frozen metadata format and its size limit, the observed-state
vocabulary, the one remote-token resolver shared by merge, pull and push, and
the index transitions. It performs no I/O, holds no lock and repairs nothing;
observations arrive as values and nothing here goes looking for them.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
