# gwz-work-detector

Pure classification of unsaved work for the GWZ local clone family. This crate
turns one repository's observed on-disk work and the caller's decoded GWZ
evidence into a report: clean, dirty with named hazards, or unknown with
reasons. It reads nothing and holds no core or protocol type. Unknown dominates
dirty, and the hazards found alongside it are still listed. A tracked path
carrying a status-suppression flag is classified from its physical state rather
than from status, so it is never silently reported clean, and diagnostic
truncation never removes a hazard.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
