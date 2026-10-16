# Full-core SSH candidate integration

This harness compiles the actual backend and workspace command drivers against
the local `git2-rs` fork and `gwz-transport` member, with the SSH endpoint's own
suites (`src/git/endpoint/ssh_tests`, formerly the separate `tests/transport_ssh`
crate). It does not enable a released route.

## Where the transport's dependencies are

They are in gwz-core's own `Cargo.toml`, in `[target.'cfg(..)']` tables whose condition
names `gwz_transport_candidate`: `gwz-transport`, `gwz-sspi`, `ssh2`, `tokio`, `hyper` and the rest
(normal dependencies); `pyo3`, `rustls`, `tokio-rustls` and `russh` (dev-dependencies); the 19
`windows-sys` features the transport adds to the production list; and the per-platform TLS tables
(`native-tls` and `tokio-native-tls` on Apple and Windows, `openssl` and `openssl-probe` elsewhere, with
`native-tls` as the OpenSSL build's test oracle). Cargo reads a `cfg(..)` target key against
`rustc --print cfg`, which lists the `--cfg` values of `RUSTFLAGS`, so a build with
`RUSTFLAGS='--cfg gwz_transport_candidate'` resolves them and an ordinary build does not: its
`cargo tree` is the one the manifest had before the tables were added.
`test_prepare.py::test_the_ordinary_shape_resolves_the_dependencies_it_resolved_without_the_switch_tables`
holds that for macOS, Linux and Windows. `Cargo.lock` names every package of both shapes, since a lock resolves
every target, and all builds use that one lock. Cargo reads a path dependency's manifest whatever the
condition, so `../gwz-transport` and `../gwz-sspi` must be on disk beside gwz-core for every build, ordinary
ones included, as `../git2-rs` is. A candidate build needs no generated manifest for these dependencies:

```sh
RUSTFLAGS='--cfg gwz_transport_candidate' cargo +1.95.0 test --lib --locked --target-dir <outside the workspace> <filter>
```

`prepare.py` is kept for what the manifest cannot say: the `transport_placement_guide` test target
(no cfg gates a `[[test]]` in a manifest; the guide documents the Cli placement, so its test is empty unless
`gwz_session_candidate` is set), the protocol pair's `placement-candidate.json`, and an isolated
tree for callers that build away from the workspace (`windows_lane_check.py`, gwz-py's
`build_candidate_extension.py`, the CI jobs). Steps C3b and C4 of `dev-docs/GwzV110RemainingPlan.md` retire the rest.

From the GWZ workspace root, using Rust 1.95.0, Git, OpenSSH's `/usr/sbin/sshd`,
`ssh-keygen`, `ssh-agent` and `ssh-add`, `openssl`, `ps`, `kill` and `python3`:

```sh
python3 -B gwz-core/tests/transport_backend/prepare.py /tmp/gwz-backend-new
RUSTFLAGS='--cfg gwz_transport_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-backend-new/Cargo.toml --offline \
  --target-dir /tmp/gwz-backend-new-target \
  --lib git::gitbackend::transport_candidate_tests
```

The first invocation prunes the copied lock to what the prepared tree uses (it is not a
workspace, so the members' own dev-dependencies leave it), using already cached dependencies.
Subsequent invocations must add `--locked`. The generated manifest and lock identify that local
resolution; retain their hashes with evidence. This is not clean-install or published-source qualification.
The prepared directory symlinks source, so source edits require rebuilding. It also links `.github` and
`scripts`, which `tests/publish_workflow.rs` reads through `CARGO_MANIFEST_DIR`: a read through `tests/../.github`
resolves to the core on Linux and to the prepared directory's own `.github` on Windows, where `..` is collapsed
before the lookup.

The HTTPS tests' TLS servers (`src/git/endpoint/https_fixture.rs`) are rustls over the
`ring` provider, test-only dev-dependencies that `Cargo.toml` names behind the switch; the clients under test stay the product's TLS stack (`src/git/endpoint/https_backend.rs`: native-tls
on macOS and Windows, the `openssl` crate directly elsewhere) and verify against the fixture's CA, which
`openssl` generates per run. The servers are not native-tls because schannel loads a server
identity through the user's key store, which Windows denies under a key-based OpenSSH logon.

The candidate configuration is Unix-only. Tests inject fixture-owned endpoint
paths, use temporary keys and loopback SSH servers, and never read the user's
agent, trust store or private key. Ordinary builds retain native libgit2 routing,
and so does a candidate backend without a host context (TR2.11). HOME and
SSH_AUTH_SOCK are read only by `SshEndpointConfig::from_environment`, when a
runtime is built from it, as `with_local_transport` does.

On Windows the SSH test server is the system's own OpenSSH server, `sshd.exe`, started as a plain
foreground child of the test, with a configuration, keys and `authorized_keys` of its own in a
temporary directory (no service, no system-wide setting). It is found at `GWZ_TEST_SSHD`, else
`%SystemRoot%\System32\OpenSSH\sshd.exe`, else on `PATH`; a missing server fails the test, as on
Unix. The keys are RSA, since libssh2 on Windows CNG has no ed25519 (the files keep their
`_ed25519` names). The server's exec requests run under the host's OpenSSH default shell, which must be a POSIX shell
(`HKLM\SOFTWARE\OpenSSH\DefaultShell` Git for Windows' `bash.exe`, `DefaultShellCommandOption` `-c`; the
candidate-windows job sets it on its runner): `sshd.exe` replaces its sessions' `PATH` with the registry's, so a
`cmd.exe` session can neither find a Git service nor quote its repository, and a fixture refuses such a host with
that said. Git and a POSIX `sh` (`GWZ_TEST_SH`, or `sh.exe` on `PATH`) must be on the test's `PATH`, as Git for
Windows provides. Every process the server starts is in a Job Object
that the test owns and closes.

The stuck-close rows (`ssh_tests::background_close`, `ssh_tests::channel`) need a channel whose output ends while the
server's session process lives, which a Windows `sshd.exe` 9.5p2 never does (it sends EOF when the process exits). They
run against a small SSH server a test controls instead, on every platform: `src/git/endpoint/ssh_controlled_server`,
test code over [russh](https://crates.io/crates/russh), a dev-dependency that `Cargo.toml` names behind the switch
(`default-features = false`, features `ring` and `rsa`, so it shares `ring` with the TLS servers and needs no C
toolchain). It serves the fixture's host key, public-key authentication for the fixture's one client key, and exec
requests for `git-upload-pack` and `git-receive-pack`, which it runs with Git; its `Ending` says whether the channel then
closes as `sshd`'s does or sends the EOF and nothing more. russh is Apache-2.0 licensed, and nothing of it is compiled
into a product build or linked into a binary the package ships.

Stock `sshd` checks only system passwords, which no test may send, so the
URL-password tests (TR2.18) run `password_sshd.py` instead: a loopback SSH
server on Python's standard library alone that accepts one fixed test password
and logs each authentication request without its password.

Tests cover backend clone/fetch/tags/advertisement/manifest/push, push URL and
rejection callbacks, progress, independent operation observations, shared pool
ownership, failure classification, and bootstrap/member/workspace/pull driver
paths including nested materialization. Local-family operations retain their
existing native route. The existing synchronous key-availability preflight is
preserved; definitive snapshot admission is independently supervised before every
candidate pool checkout. This does not claim that inherited preflight filesystem
calls are bounded.


## Endpoint placement candidate

The same prepared manifest also adds the `transport_placement_guide` test target.
With `--cfg gwz_session_candidate` too, the build selects the session candidate's
generated schema (`src/protocol/candidate_generated.rs`) and the Cli placement of the
`transport_host` facade (S7.1 (1.1.0) step C3a moved both off the transport switch).
Run the complete host/CLI-endpoint integration suite and compile the guide's example:

```sh
RUSTFLAGS='--cfg gwz_transport_candidate --cfg gwz_session_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-backend-new/Cargo.toml --locked --offline \
  --target-dir /tmp/gwz-backend-new-target --lib transport_host -- --test-threads=1
RUSTFLAGS='--cfg gwz_transport_candidate --cfg gwz_session_candidate' cargo +1.95.0 test \
  --manifest-path /tmp/gwz-backend-new/Cargo.toml --locked --offline \
  --target-dir /tmp/gwz-backend-new-target --test transport_placement_guide
python3 -B -m pytest -q gwz-core/tests/transport_backend/test_prepare.py
```

`test_prepare.py` needs Python 3.11 or newer. The guide target
(`guide_test.rs`) compiles the exact example from `docs/TransportPlacement.md`
with `gwz_core` as an extern crate, so the example reaches only public API. The
suite tests in-memory host message delivery, separate credential homes, real SSH streams,
shared pools, endpoint preflight, authentication/refusal facts, cancellation,
concurrent streams, and ordinary workspace command drivers. It does not add a
physical CLI/core carrier or qualify a split-process deployment. The local SSH
fixture server uses disposable keys; no user's credentials are used.

## In-process operation-message embedding

The candidate shape has a test-only PyO3 dependency (`auto-initialize`, behind the switch in `Cargo.toml`) to run
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
