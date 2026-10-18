# TR2.8 — CODE-AXIS REVIEW

**Review object:** TR2.8 implementation diff in gwz-core `bb67a8264a71a5141d3345a5d1367f4228aeb5db..e3e5f12672efdcba66663a4b9a7fb8663cc94513`, comprising the inventory commit and implementation checkpoint. Status: implemented, pending acceptance.

**Baseline:** Workspace `/Volumes/projects/limbo/gwz-dev-tr2-8`; root `2f65e3c898fdd9c9eb78b7557339613aab1c8ba7`; gwz-core `e3e5f12672efdcba66663a4b9a7fb8663cc94513`; gwz-transport `6910ba669ccc654e11e7d7cc4a6a1f76b0db51b4`; git2-rs `d13951f7e0bfb6e0efcee1207ac5b140adefa455`. Sources read from this checkout and inspected with `git diff`, `git log`, numbered source reads, and targeted searches. The exact tuple matched at both start and end.

**Date:** 2026-10-02.

**Axis:** Code — architecture, interface contracts, call paths, native ownership, compatibility, and implementation against controlling authority. Independent, adversarial, read-only. Nothing here relies on the parallel reviewer. Filed verbatim by the lane owner.

**Verdict: NO-GO** — two P2 findings block; one P3 finding is nonblocking. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified.

---

## 0. Evidence base

Read and inspected:

- Root `AGENTS.md`, `AGENTS_GWZ.md`, `dev-docs/AgentProcessRules.md`, its controlling `GwzProcessOptimization.md` amendment, and core `AGENTS.md`.
- `GwzTransportReleasePlanAmendment-2.md` §3.19; `GwzTransportSshKeyTypes.md` §§1–6; `GwzTransportSshKeyTypes-Implementation.md` lines 1–55; actual agent authority `GwzRemoteTransportSshAgentDesign.md`, especially §§2, 5–6 and its amendments.
- Production changes: `agent_keys.rs` lines 1–249; `agent_auth.rs` lines 1–306; `agent_client.rs` lines 1–200; `ssh_key_container.rs` lines 1–289; module registration.
- Retained production callers: `ssh_key_snapshot.rs` lines 134–186 and `ssh_key_auth.rs` lines 18–78, establishing which captured bytes reach native parsing.
- Changed tests and fixtures: `ssh_tests/agent_keys.rs`, `agent_client.rs`, `agent_fixture.rs`, `key_container.rs`, `key_files.rs`, `key_fixture.rs`, `key_types.rs`, `rsa_sha1.rs`, `local_endpoint.rs`, and test module boundaries; `ssh_fixture.rs`, `ssh_password_fixture.rs`, `key_agent.py`, and `password_sshd.py`.
- Bundled libssh2-sys 0.3.3 source, particularly `pem.c` lines 73–97 and 800–860, `openssl.c` lines 4985–5035, `userauth.c` lines 1730–1834, and `packet.c` lines 848–910.
- External `ssh-focused.log`: line 354 records **148 passed, 0 failed, 3 ignored**. This is supplied execution evidence, not a test run performed by this reviewer. Full-suite acceptance was not inferred.

No builds, new tests, mutations, file writes, or peer-report inspection occurred. Finding reproductions below are concrete source-derived sequences, not claims of executed reproductions.

## 1. Findings

### [P2-1] RSA downgrade bypasses signature and key-shape validation

**Location:** [agent_keys.rs:102](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/agent_keys.rs:102), lines 102–113; fallback handling at [agent_auth.rs:242](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/agent_auth.rs:242).

**Violated invariant:** A malformed agent reply terminates authentication. The implementation document states this at lines 15–20, and the inventory states that each signature’s shape is checked before libssh2 sees it and that malformed replies end the login (`GwzTransportSshKeyTypes.md:159–174`).

The downgrade branch recognizes `ssh-rsa`, checks only that its signature string is nonempty and has no trailing bytes, then returns `Signed::Downgraded`. It exits before `self.key(blob)` and before the RSA modulus-length validation.

**Reproduction sequence:**

1. List an ordinary 2048-bit RSA key.
2. Let the server advertise and accept a `rsa-sha2-512` query.
3. Answer the agent’s SHA-2 sign request with a correctly framed envelope containing algorithm `ssh-rsa` and **257 bytes of `0xff`** as the signature.
4. `signature()` accepts this as `Downgraded`, although that value cannot be a signature for the 256-byte modulus.
5. The callback returns `LIBSSH2_ERROR_ALGO_UNSUPPORTED`; pinned `userauth.c:1754–1763` offers the key again under its default algorithm.
6. A valid response to the second request can complete authentication.

Thus malformed input permits a second sign request and SHA-1 authentication instead of terminating the operation.

**Impact:** The newly permitted fallback weakens the promised fail-closed boundary. Invalid RSA responses can select the downgrade path.

**Required correction:** Validate the key and RSA signature shape before recognizing the downgrade outcome. Apply the same structural policy to both ordinary RSA signatures and the SHA-1 response that authorizes fallback.

**Closure/regression test:** Extend `ssh_tests/agent_keys.rs:213–242` with oversized, empty, truncated, and trailing-data downgrade responses, plus malformed RSA key blobs. Add a production bridge row in which the first response has the oversized signature and the agent would return a valid second response: authentication must return `InvalidData`, with exactly one sign request and no SHA-1 query. Retain the valid fallback row in `rsa_sha1.rs:65–75`.

### [P2-2] Container admission and native parsing disagree on carriage-return boundaries

**Location:** [ssh_key_container.rs:55](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_key_container.rs:55), lines 55–61, and [ssh_key_container.rs:233](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_key_container.rs:233), lines 233–237.

**Violated invariant:** Admission requires exactly one unencrypted private key, and encrypted containers are refused before native authentication (`GwzTransportSshKeyTypes.md:176–180`; implementation document lines 22–25).

The checker splits only on LF and strips a CR only at the end of that LF-delimited line. Pinned libssh2’s memory parser splits on **either CR or LF** (`pem.c:81–95`). Since TR2.8 now skips arbitrary text outside recognized blocks, a complete native-readable block can hide inside a checker-skipped line.

**Reproduction sequence:**

1. Generate different unencrypted OpenSSH private keys A and B.
2. Construct the file as:
   ```text
   "ignored\r" + armor_A.replace("\n", "\r") + "ignored\n" + armor_B
   ```
3. The checker sees the entire A portion as one LF-delimited line beginning `ignored`; it skips that line. It recognizes and validates B, then accepts the file.
4. `ssh_key_snapshot.rs:179–185` retains the complete original text. `ssh_key_auth.rs:50` passes that text to `userauth_pubkey_memory`.
5. OpenSSL’s PEM reader cannot read these OpenSSH blocks, so the pinned backend falls through to its OpenSSH memory parser (`openssl.c:5016–5031`).
6. That parser reads CR-separated lines and finds A first (`pem.c:815–827`). Against a server authorizing A only, native authentication selects A.

The admitted file therefore contains two keys, and native signing selects a block the checker skipped. Making A encrypted similarly allows an encrypted block past admission, although native authentication subsequently fails.

**Impact:** The checker’s single-key and encryption guarantees do not describe the bytes interpreted by native code. Ambiguous files reach network authentication and can authenticate with the unchecked block.

**Required correction:** Align admission’s line-boundary grammar with the native readers, or reject ambiguous bare-CR input before skipping surrounding text. Ensure every native-recognizable private-key block participates in the single-key and encryption checks.

**Closure/regression test:** Add the A/B construction above to container tests and the production snapshot/authentication path. It must fail admission before authentication, including when A is encrypted. Cover bare CR, LF, and CRLF boundaries while retaining the intended bag-attribute and surrounding-block compatibility rows.

### [P3-1] Key fixture children lack cleanup ownership during construction

**Location:** [ssh_tests/key_fixture.rs:56](/Volumes/projects/limbo/gwz-dev-tr2-8/gwz-core/src/git/endpoint/ssh_tests/key_fixture.rs:56), lines 56–122; cleanup exists only in `KeyAgent::drop`, lines 141–146.

**Violated invariant:** Disposable fixtures must reap their children on failure as well as success.

`start()` spawns the private `ssh-agent` into a bare `Child`, then performs fallible readiness checks, `ssh-add` calls, file operations, Python startup, and ready-response parsing before constructing `KeyAgent`. Dropping `std::process::Child` during unwinding does not kill or reap the process.

**Reproduction sequence:** Call `KeyAgent::start` with a nonexistent private-key path. The private agent starts; `ssh-add` fails; `common::run` asserts at `ssh_fixture.rs:245–249`; construction unwinds before `KeyAgent` exists. Its cleanup does not run, leaving the private agent alive. Failures after Python spawn can leave both children.

**Impact:** Failed qualification runs leak processes and sockets, potentially contaminating later runs and requiring manual cleanup.

**Required correction:** Give each child a kill-and-wait guard immediately after spawning, then transfer ownership into the completed fixture.

**Closure/regression test:** Exercise a failed private-key load and a failed ready-response parse under `catch_unwind`; verify both child PIDs have terminated and been reaped.

## 2. Invariant analysis

The following attacks did not reveal additional defects:

- **Admitted algorithms and certificates:** The type table and certificate stems cover the stated families. RSA certificate methods map to base SHA-2 algorithms, so flags 2/4 reach the agent. Unsupported types are skipped explicitly. Certificate parsing accounts for nonce, key material, and the certificate tail.
- **Security-key framing:** The validator retains the signature string and exactly five following bytes for flags/counter (`agent_keys.rs:114–127`). This matches pinned libssh2’s unframed security-key append at `userauth.c:1808–1820`. Both security-key families and their certificates are exercised against OpenSSH’s independent server verification.
- **Negotiation:** Algorithm selection remains libssh2-owned. The ordinary three SHA-1 cases have production-path fixture rows; SHA-2 preference and `METHOD_NONE` behavior are explicitly tested. `server-sig-algs` remains the post-key-exchange extension processed by the pinned SSH implementation; the diff adds no unprotected negotiation source.
- **Refusal versus transport failure:** Exact one-byte agent refusal returns `None` without poisoning the connection. Other codec/I/O errors mark the agent failed; callback errors terminate authentication before another identity. Cancellation checks surround exchange and handoff. P2-1 is the exception in malformed-response handling.
- **Native lifetime and allocation:** Per-key callback state remains stable across EAGAIN. The callback catches Rust unwinds, bounds invocation count, and copies successful signatures into Unix `malloc` storage. Pinned native code frees that storage on its successful append and allocation-failure paths. Windows remains explicitly unadmitted.
- **Selected-key ownership:** The bounded snapshot reader and immutable captured text remain in place; native authentication does not reopen the path or fall back to an ambient agent. Scanning and decoding retain bounded cancellation checkpoints. P2-2 concerns interpretation of those captured bytes.
- **Scope and platform boundaries:** The diff changes private implementation and test surfaces, without a public API, dependency, or GWZ protocol change. New conditional sections use explicit `cfg_if!` boundaries, including disabled non-Unix branches.

The passing focused suite demonstrates the intended ordinary paths. Its malformed-signature rows do not cover malformed fallback responses, and its container rows do not cover the CR/LF parser differential.

## 3. Risks and next action

Hardware-key manual execution, Windows bridge/allocator qualification, and Linux/Windows release qualification remain explicitly deferred and are not findings. This review does not establish their release readiness.

Next action: remediate P2-1 and P2-2, add the specified regressions, and settle a new tuple for bounded Code re-review. Address P3-1 in the same revision if practical. The reviewed tuple remained unchanged through the final verification.
