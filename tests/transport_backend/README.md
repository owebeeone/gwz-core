# Full-core SSH candidate integration

This harness compiles the actual backend and workspace command drivers against
the local `git2-rs` fork and `gwz-transport` member. It covers ownership and
callback/report integration that the isolated `transport_ssh` fixture cannot
exercise. It does not change production manifests or enable a released route.

From the GWZ workspace root, using Rust 1.95.0 and the prerequisites in
`tests/transport_ssh/README.md`:

```sh
python3 -B gwz-core/tests/transport_backend/prepare.py /tmp/gwz-backend-new
RUSTFLAGS='--cfg gwz_transport_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-backend-new/Cargo.toml --offline \
  --target-dir /tmp/gwz-backend-new-target \
  --lib git::gitbackend::transport_candidate_tests
```

The first invocation derives a candidate lock from the production lock, using
already cached dependencies. Subsequent invocations must add `--locked`. The
generated manifest and lock identify that local resolution; retain their hashes
with evidence. This is not clean-install or published-source qualification.
The prepared directory symlinks source, so source edits require rebuilding.

The candidate configuration is Unix-only. Tests inject fixture-owned endpoint
paths, use temporary keys and loopback SSH servers, and never read the user's
agent, trust store or private key. Ordinary builds retain native libgit2 routing.
Only the candidate runtime's uninjected path reads HOME/SSH_AUTH_SOCK lazily.

Tests cover backend clone/fetch/tags/advertisement/manifest/push, push URL and
rejection callbacks, progress, independent operation observations, shared pool
ownership, failure classification, and bootstrap/member/workspace/pull driver
paths including nested materialization. Local-family operations retain their
existing native route. The existing synchronous key-availability preflight is
preserved; definitive snapshot admission is independently supervised before every
candidate pool checkout. This does not claim that inherited preflight filesystem
calls are bounded.
