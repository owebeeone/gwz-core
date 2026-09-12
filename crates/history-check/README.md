# gwz-history-check

Bounded, read-only history preservation checking for the GWZ local clone
family. This crate decides whether every protected root of a deletion target is
reachable, with a complete locally available object graph, from the surviving
witnesses' own retained roots. It reads through an injected object reader,
memoizes visits within one invocation, accounts for its own bookkeeping against
explicit limits, polls a cancellation port between bounded units, and persists
nothing. A read failure, a cancellation or an exceeded limit is an unknown
answer, never a verified one, and nothing here can delete.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
