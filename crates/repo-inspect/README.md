# gwz-repo-inspect

Local Git and filesystem reads for the GWZ local clone family. This crate
implements the repository inspection and bounded object-reading contracts for
one admitted repository path, covering typed object ids in the repository's own
format, the layout hazards of gitfiles, alternates, external common directories
and escaping metadata, and physical observation of status-suppressed paths.
Every call opens the repository with libgit2, reads, and closes it. Nothing
writes: status is taken with the index refresh and the index update both
disabled, and no implicit fetch, flag clearing or maintenance ever runs.

Object-body read limits bound commits, trees and tags before parsing their
edges. Blobs are graph leaves: their headers establish local presence and size
without loading their payloads, so large blobs do not exhaust the body budget.
This is connectivity verification, not a blob-content integrity check.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
