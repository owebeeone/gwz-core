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


## Endpoint placement candidate

The same prepared manifest also selects the shared generated schema and the
`transport_host` facade. Run the complete host/CLI-endpoint integration suite:

```sh
RUSTFLAGS='--cfg gwz_transport_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-backend-new/Cargo.toml --locked --offline \
  --target-dir /tmp/gwz-backend-new-target --lib transport_host -- --test-threads=1
python3 -B -m pytest -q gwz-core/tests/transport_backend/test_prepare.py
```

This compiles the exact example from `docs/TransportPlacement.md` and tests
in-memory host message delivery, separate credential homes, real SSH streams,
shared pools, endpoint preflight, authentication/refusal facts, cancellation,
concurrent streams, and ordinary workspace command drivers. It does not add a
physical CLI/core carrier or qualify a split-process deployment. The local SSH
fixture server uses disposable keys; no user's credentials are used.

## In-process operation-message embedding

The prepared candidate has a test-only PyO3 dependency (`auto-initialize`) to run
Python inside the Rust test process. A Python installation with a linkable shared
library is required; select it with `PYO3_PYTHON` if autodetection is unsuitable.
Python and Taut sources are loaded from the same workspace's `gwz-py/src` and
`taut/src`; no Python worker process or network carrier is started. The existing
SSH fixture still starts its disposable loopback SSH server as usual.

```sh
python3 -B gwz-core/tests/transport_backend/prepare.py /tmp/gwz-placement-c-new
RUSTFLAGS='--cfg gwz_transport_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-placement-c-new/Cargo.toml --offline \
  --target-dir /tmp/gwz-placement-c-target --lib message_embedding_tests \
  -- --test-threads=1
```

The first run resolves the external candidate lock; subsequent runs add `--locked`.
Fixtures wrap live envelopes in `InitFromSourcesRequest`/`InitFromSourcesResponse`
metadata and route attachments ahead of ordinary dispatch. The Rust branch uses
the core-generated types used by the CLI's direct handler call. The Python branch
uses the actual gwz-py codec with candidate dataclasses and schema selected only
in that test interpreter. Full CLI executable/native-extension activation is not
claimed. The test checks message embedding at their existing boundaries.

See [Placement C](../../dev-docs/GwzRemoteTransportPlacementC.md) for scope and
future wire plausibility. Production manifests and generated artifacts remain
unchanged; physical wire testing and iroh are outside this cycle.
