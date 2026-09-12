# gwz-repo-contract

The repository observation contract of the GWZ local clone family. This crate
owns the plain values that describe a Git repository's layout, its unsaved work
and its protected history, plus two read-only ports: one for the layout, work
and history inventory of a repository path, and one for bounded object-graph
reads. It carries no Git implementation, and adapters translate at the edge.
Object ids carry their object format, path names are bytes rather than UTF-8,
observations distinguish known from unknown, and a read that cannot stay within
its bounds fails with a typed error instead of guessing.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
