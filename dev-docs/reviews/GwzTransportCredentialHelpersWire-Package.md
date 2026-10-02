# TR2.22 wire checkpoint settlement manifest

Prepared for root-owned settlement. Only these paths are part of this checkpoint.
Protected drafts are deliberately excluded. The two configured-gh tests are the
accepted red characterization for the next secret-runner checkpoint.

Tier: mandatory dual Code/State on the settled tuple. No dirty review.
Coordinator gate before dispatch: archive-consumer proof from a clean settled
transport archive on Rust 1.95; then replace settlement tokens in canonical prompts.

## gwz-core

```text
dev-docs/GwzTransportCredentialHelpersImplementation.md
dev-docs/reviews/GwzTransportCredentialHelpersWire-PromptCode.md
dev-docs/reviews/GwzTransportCredentialHelpersWire-PromptState.md
scripts/checks/check_checked_artifact_boundaries.py
src/git/endpoint/budget_wait_tests.rs
src/git/endpoint/https_connection.rs
src/git/endpoint/https_remote_tests.rs
src/git/endpoint/https_worker.rs
src/git/endpoint/https_worker_tests.rs
src/git/endpoint/https_worker_tests/configured_helpers.rs
src/git/endpoint/placement_endpoint.rs
src/git/endpoint/placement_endpoint/admission.rs
src/git/endpoint/placement_endpoint/attachments.rs
src/git/endpoint/placement_endpoint/cancel.rs
src/git/endpoint/placement_endpoint/completion.rs
src/git/endpoint/placement_endpoint/retry_tests.rs
src/git/endpoint/placement_endpoint_tests.rs
src/git/endpoint/setup_retry.rs
src/git/endpoint/setup_retry/classify_tests.rs
src/git/endpoint/setup_retry/machine.rs
src/git/endpoint/setup_retry/machine_tests.rs
src/git/endpoint/setup_retry/operations.rs
src/git/endpoint/shared_reservation.rs
src/git/endpoint/ssh_pool.rs
src/git/endpoint/ssh_pump.rs
src/git/endpoint/ssh_setup.rs
src/git/endpoint/ssh_tests/placement_endpoint.rs
src/git/endpoint/ssh_tests/pool_host.rs
src/git/endpoint/ssh_tests/pooled.rs
src/git/endpoint/ssh_tests/worker.rs
src/git/endpoint/ssh_worker.rs
src/git/endpoint/ssh_worker_tests.rs
src/transport_host/fault_tests.rs
src/transport_host/https_endpoint.rs
src/transport_host/https_endpoint/retry.rs
src/transport_host/https_endpoint/retry_tests.rs
src/transport_host/request.rs
src/transport_host/session.rs
src/transport_host/session/driver.rs
src/transport_host/session/driver/opening.rs
src/transport_host/session/requests.rs
tests/transport_consumer/tests/admission.rs
tests/transport_consumer/tests/messages.rs
tests/transport_consumer/tests/pool_host.rs
```

## gwz-transport

```text
README.md
protocol/transport.ir.json
protocol/transport.taut.py
scripts/regen.py
src/admission.rs
src/binding.rs
src/codec.rs
src/codec/failure_detail.rs
src/codec/validate.rs
src/mux/mod.rs
src/mux/routing.rs
src/protocol.rs
src/stream/machine.rs
tests/async_stream.rs
tests/failure_detail.rs
tests/https_reuse.rs
tests/io_clock.rs
tests/mux.rs
tests/network_timeouts.rs
tests/placement_v2.rs
tests/pool.rs
tests/sequenced.rs
tests/setup_cause.rs
tests/stream.rs
tests/support/pool_case.rs
tests/wake_amplification.rs
```

This manifest itself is an owned core package input.

Executed gates and exact limitations: ../GwzTransportCredentialHelpersImplementation.md.
No root source/document path or private-evidence path is included.
