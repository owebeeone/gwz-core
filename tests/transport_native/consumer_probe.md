# Q3 native consumer probe

`consumer_probe.rs` is an isolated qualification module. The owner harness
includes it by path in copied core and `gwz-git` consumers; it is deliberately
not registered in production or in this proof crate.

`run(root)` requires a new root and creates it with `create_dir`, so an existing
path fails. It asserts vendored libgit2 1.9.7, records HTTPS/SSH capability,
and exercises SHA-1 and SHA-256 bare repositories. Each row validates the
`gwz-git` full-ID commit record, tree and parent order, fixed identity/time,
raw message bytes, and the stored payload blob. It then performs two local
directory fetches: the base commit is fetched first, a receiver checkpoint
points at a shared blob, and the child fetch must succeed while preserving that
checkpoint. A stock-C failure is reported with `Q3 receiver noncommit hint fetch failed`.

A fresh fixture for each object format writes malformed `info/grafts` and requires the owned
native diagnostic class 36/code -1. Output is tab-separated: one native
capability row followed by one row per object format containing blob, base and
child IDs, `fetch-ok`, and `raw-class-36`. The probe performs no Git executable,
network, environment, cwd, or production-state operation. Native Windows and
clean distribution execution remain separate qualification gates.
