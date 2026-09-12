# gwz-copy-contract

The tree-copy contract of the GWZ local clone family. This crate owns the
request, report, error and cancellation values of a whole-tree copy, and the
`TreeCopier` port that performs one. It carries no platform implementation:
`gwz-refcopy` implements the port with native copy-on-write plus an ordinary
fallback, and `gwz-workspace-install` consumes it through a trait object. Every
value crossing this boundary is owned plain data, so no OS handle, `git2` type
or protocol type leaks through it.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
