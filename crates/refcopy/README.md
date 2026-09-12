# gwz-refcopy

The product tree copier of the GWZ local clone family. `SystemTreeCopier`
implements the tree-copy contract, copying a whole tree with exclusions applied
during traversal. Every regular file arrives either by the platform's native
copy-on-write mechanism (Apple `clonefile`, Linux `FICLONE`, Windows
`FSCTL_DUPLICATE_EXTENTS_TO_FILE`) or by ordinary buffered reads and writes.
Which one ran changes nothing an inspection of the destination can see, since a
clone shares physical blocks and is never a hardlink; it changes only the
counts in the report, which is how a copy says how the bytes actually arrived.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
