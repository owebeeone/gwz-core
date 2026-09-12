# gwz-family-store

The sole writer of GWZ family metadata. `YamlFamilyStore` implements the family
store contract over the frozen format-1 files, the root index, clone pointers
and allocation markers, using same-directory temporary write and rename with
checked flushes, the 1 MiB encoded-index limit, and a small OS advisory
try-lock released with its handle. It is best-effort metadata publication
rather than a power-loss-safe multi-file transaction: every step is checked and
a failure is reported, with the partial effects that already landed, but
nothing is journalled, replayed or repaired.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
