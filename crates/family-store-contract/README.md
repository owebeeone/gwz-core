# gwz-family-store-contract

The family store contract of the GWZ local clone family. The store is the sole
writer of the family index, clone pointers and allocation markers, and this
crate owns the location, the read observation, the locked session port, the
typed store errors and the partial-effect vocabulary. It contains no YAML, no
lock implementation and no filesystem code: `gwz-family-store` implements it,
while `gwz-workspace-install` and `gwz-local-disposal` consume it through a
live session. Every error names its operation, typed cause and known partial
effects.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
