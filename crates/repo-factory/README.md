# gwz-repo-factory

Independent bare and clean repository construction for the GWZ local clone
family. This crate builds a destination's repositories from one captured freeze
vector, the destination, frozen commit, branch and origin of every repository
including the root, through the repository-builder port it owns. Everything is
checked before anything is created, so a refused construction has allocated
nothing. There is no implicit network, no hidden commit and no borrowed object
store: a missing object is an error, never a fetch. It writes no family
metadata, and reports what the installer still owes.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
