# gwz-workspace-install

Destination construction and installation ordering for the GWZ local clone
family. This crate composes a local clone destination in four ordered steps
through injected ports, and the order is the product. It admits the name, path,
source-layout and nested-repository checks before anything is written; reserves
the row and allocates the destination directory; builds by copying with
exclusions or constructing clean and bare repositories, installing the
destination's own configuration, pointer and marker; then publishes, writing
the final manifest last and marking the row ready.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
