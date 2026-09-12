# gwz-local-disposal

Explicit keep and one-shot disposal for the GWZ local clone family. This is the
only local-clone service that removes directory contents. Under the family lock
it validates an intact ready target, gathers fresh work evidence through its
ports and classifies it, asks the history port whether every protected root is
preserved elsewhere, refuses dirty, unpreserved or unknown evidence unless a
named hazard waiver covers it, records the intent through the store session,
then removes the validated directory once. The refusal is the product: every
uncertain answer refuses, and on any error it stops and reports what remains.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
