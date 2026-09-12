# gwz-local-import

Family import and push orchestration for the GWZ local clone family. This crate
pairs every selected receiver with its source repository by identity, captures
the source object ids, fetches them through its own narrow local transport port
into one fresh, collision-checked import ref in every receiver, and verifies
every received object id before the caller enters the merge or pull engine. A
companion entry point publishes explicit refspecs into a family member with
per-repository partial results. Nothing here deletes a ref: the port has no
removal method at all.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
