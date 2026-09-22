# Alpha SSH setup timeout diagnosis

2026-09-22. Reproduced using the operator-supplied SSH agent. Installed alpha
remains SHA256 `5419fd0218ab06084bee9152b09a79864a2c165bce62d9d7924a70d4bb6dce8c`;
no timeout fix or rebuild has been made.

A default-timeout live fetch failed five first-batch repositories after offering
credentials and before authentication completed. Later repositories succeeded.
Other default runs passed, confirming intermittence. All three runs with
`gwz-alpha --ssh-timeout 15 fetch` passed (JSON mode used for retained evidence).
This is a temporary workaround, not proof of a fix.

The alpha incorrectly couples the stalled-native-I/O setting to one cumulative
setup deadline. `SshEndpointConfig::from_environment` and session Open both set
connect_ms from the 3-second I/O setting. The setup job spends this deadline
across DNS, TCP, SSH handshake and agent authentication. Stable libgit2 applies
the configured timeout to individual native calls. Thus progressing setup may
exceed the alpha deadline while succeeding under stable gwz. OpenFailed is
rendered as PeerFailed, obscuring the setup stage in ordinary output.

Next: reconcile the native-timeout preservation requirement in transport design
section 10 with the pool's aggregate connect budget. Preserve bounded cancellation
and physical disposal. Add a deterministic progressing multi-stage setup
regression, true-stall expiry, disabled timing, and late-result/non-reuse tests;
review the bounded clock change, rebuild/reinstall alpha, then repeat cold live
fetches. Do not treat increasing the existing global timeout as the permanent fix.

Raw evidence and provenance (private access): gwz-core-evidence,
`campaigns/transport-qualification/runs/2026-09-22-alpha-setup-timeout`.
No per-stage timing trace was collected; the exact slow native sub-operation
inside authentication is not established. Broader release/platform work remains
paused as before.
