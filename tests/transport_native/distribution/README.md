# GWZ git2 fork candidate

`gwz-git2` 0.21.0-gwz.1 is a proposed distribution of upstream git2 0.21.0
with GWZ's qualified per-remote smart-transport callback. The Rust library name
remains `git2`; consumers select it explicitly with a Cargo package alias.
Upstream MIT/Apache-2.0 licensing and copyright notices remain intact.
`GWZ-PROVENANCE.json` identifies the source archive and the two-file patch.

This is a local qualification candidate, not a published dependency. Preparing
it does not change production GWZ manifests or contact a Git service. It requires
Rust 1.95.0, Python 3.10+, Git and cached upstream dependencies.

From the GWZ workspace root:

```sh
python3 gwz-core/tests/transport_native/distribution/fork.py \
  --git2-archive "$HOME/.cargo/registry/cache"/index.crates.io-*/git2-0.21.0.crate \
  --output /tmp/gwz-git2-candidate
```

The default is offline. Add `--fetch` on the first run to let Cargo download
missing packaging dependencies from its configured registry. This does not
publish anything; the runtime proof still uses its fixed offline dependency graph.

Choose one archive if multiple registries match. Output must be a new directory.
The script verifies the accepted patch, prepares the renamed source, runs the
native binding suite, packages it with Cargo using the pinned packaging lock, and reruns against the exact
package contents. It refuses unrelated dependency graph changes. Output contains
the source, Cargo archive and a qualification record. Temporary source is removed;
build outputs remain in `tests/transport_native/target/distribution`. Delete the
chosen output and that build directory to clean up. There is no publication,
remote fork, credential setup or background service to undo.

Future production adoption must select one git2 package identity across the
core, its repo-inspect/testrepo crates and CLI. Proposed dependency form:

```toml
git2 = { package = "gwz-git2", version = "=0.21.0-gwz.1", features = ["https", "ssh", "unstable-sha256"] }
```

Keep each consumer's existing feature set. A root-only Cargo patch does not
supply this dependency to downstream registry users. Publication and native
platform gates must precede production activation. Returning to upstream later
requires a coordinated dependency change and the same ownership/transport tests.
