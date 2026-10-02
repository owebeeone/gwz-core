# TR2.8 — STATE-AXIS REVIEW

**Review object:** TR2.8 implementation diff in gwz-core, `bb67a8264a71a5141d3345a5d1367f4228aeb5db..e3e5f12672efdcba66663a4b9a7fb8663cc94513`, including its inventory and implementation record. Controlling DRAFT authority: `dev-docs/GwzTransportReleasePlanAmendment-2.md` §3.19, dated 2026-10-02. No merge reviewed.

**Baseline:**

| Repository | Exact revision |
|---|---|
| Workspace root | `2f65e3c898fdd9c9eb78b7557339613aab1c8ba7` |
| gwz-core | `e3e5f12672efdcba66663a4b9a7fb8663cc94513` |
| gwz-transport | `6910ba669ccc654e11e7d7cc4a6a1f76b0db51b4` |
| git2-rs | `d13951f7e0bfb6e0efcee1207ac5b140adefa455` |

Sources were inspected with `git diff`, `nl`, `sed` and `rg`. Final `git diff HEAD` checks showed no differences in the inspected authority and core source paths. The complete tuple matched at both start and end.

**Date:** 2026-10-02

**Axis:** State machines, interruption, resource ownership, bounded retries and fail-closed authentication. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — one P2 finding blocks; one additional P3 finding concerns fixture cleanup. I pre-commit to GO on a revision that resolves P2-1 as specified, provided the revision introduces no additional blocking defect.

---

## 0. Evidence base

Read:

- Root `AGENTS.md`, `AGENTS_GWZ.md`, core `AGENTS.md`, `AgentProcessRules.md`—particularly L1-13 through L1-21—and controlling `GwzProcessOptimization.md`.
- `GwzTransportReleasePlanAmendment-2.md:454–487`, establishing TR2.8’s key/signature parity, the three SHA-1 cases and required fixtures.
- `GwzTransportSshKeyTypes.md:1–206` and `GwzTransportSshKeyTypes-Implementation.md:1–55`.
- `GwzRemoteTransportSshAgentDesign.md`, particularly ownership and handoff at lines 86–98, bounded agent I/O at 100–130, signing at 132–170, authentication facts at 221–229, and TR2.8’s amendment at 310–314.
- Complete changed production files: `agent_auth.rs:1–292`, `agent_client.rs:1–200`, `agent_keys.rs:1–249`, and `ssh_key_container.rs:1–289`.
- Changed SSH fixtures/tests and their diff, including `ssh_tests/agent_keys.rs:1–290`, `rsa_sha1.rs:1–118`, `key_types.rs:1–171`, `key_fixture.rs:1–260`, `key_container.rs:1–235`, `key_files.rs:1–149`, and the added endpoint exchange row.
- Existing ownership context in `ssh_key_auth.rs:1–81`, `ssh_key_snapshot.rs:135–187`, and callback adversity tests in `ssh_tests/agent_auth.rs:210–329`.
- Bundled libssh2-sys 0.3.3 sources: `userauth.c` algorithm selection and authentication/retry paths, including 1748–1772; `openssl.c` in-memory key readers; `pem.c` memory parsing; `packet.c:848–898` extension handling; and `kex.c:4199`.
- `tests/transport_backend/key_agent.py:1–230` and changed `password_sshd.py` signature verification and extension handling.

No build, test, mutation, file write or new experiment was performed. The supplied `ssh-focused.log:354` records **148 passed, 0 failed, 3 ignored**. That existing result does not cover the counterexample below. No acceptance claim was inferred from incomplete full-suite logs.

## 1. Findings

### [P2-1] RSA downgrade bypasses signature-shape validation

**Location:** `src/git/endpoint/agent_keys.rs:102–108`, before key validation at line 113 and RSA signature-length validation at line 121. The result becomes native fallback at `agent_auth.rs:241–243`.

**Violated invariant:** Malformed signatures terminate authentication. The implementation record states this at `GwzTransportSshKeyTypes-Implementation.md:15–20`; the agent design requires validating returned algorithm and nested shape before native handoff at `GwzRemoteTransportSshAgentDesign.md:163–170`.

**Reproduction/state sequence:**

1. List a valid 2048-bit RSA key and use a server advertising `rsa-sha2-512` and `ssh-rsa`.
2. The server accepts the SHA-2 query; the callback asks the agent to sign with flag 4.
3. Return a correctly framed agent signature envelope containing `string("ssh-rsa")` and `string([0x01])`.
4. `KeyType::signature` sees the SHA-1 name, checks only that the signature string is nonempty and has no trailing bytes, and returns `Signed::Downgraded`.
5. The callback returns `LIBSSH2_ERROR_ALGO_UNSUPPORTED`. Bundled `userauth.c:1754–1763` resets authentication state and offers the same key under its default algorithm.
6. If the next agent answer contains a valid SHA-1 signature, authentication can succeed.

The one-byte RSA signature would fail the ordinary RSA shape check at line 121. Changing its algorithm name routes it around that check.

**Impact:** A malformed reply authorizes a second signing attempt and SHA-1 fallback instead of ending authentication. The attempt remains bounded, but its transition violates the promised fail-closed direction.

**Required correction:** Validate the key and RSA signature shape before returning `Signed::Downgraded`. Permit this transition only for a structurally valid `ssh-rsa` signature answering an allowed RSA SHA-2 request.

**Closure/regression test:** Add short and oversized SHA-1 replies to the downgrade unit cases for both plain RSA and RSA certificates. Add a production callback row with a short SHA-1 first answer and a valid queued second answer: require `InvalidData`, exactly one sign request, no fallback query, and connection disposal. Preserve the existing valid fallback row in `ssh_tests/rsa_sha1.rs:64–74`.

### [P3-1] Fixture children have no cleanup owner during construction

**Location:** `src/git/endpoint/ssh_tests/key_fixture.rs:54–122`. Cleanup exists only in the completed `KeyAgent` owner at lines 141–146.

**Violated invariant:** The fixture promises both agents stop on drop (`key_fixture.rs:7–8`); the review requires fixtures to reap their children, including failure paths.

**Reproduction/state sequence:** Call `KeyAgent::start` with a nonexistent private-key path. The private `ssh-agent` has already spawned at lines 56–62. `ssh-add` fails at lines 69–76, and `common::run` panics before `Self` is constructed. The local `Child` handle drops without killing or waiting for the running agent. Likewise, a Python startup/readiness failure after line 102 occurs before either child reaches the cleanup owner.

**Impact:** A failed fixture startup leaks test processes and potentially leaves private keys loaded in the surviving test agent until external cleanup or process termination.

**Required correction:** Install a kill-and-wait guard immediately after each spawn, then transfer the children into `KeyAgent` only after construction succeeds.

**Closure/regression test:** Inject failure after upstream spawn and after proxy spawn; catch construction failure and verify both children terminate and are reaped. Keep successful drop coverage.

## 2. Invariant analysis

- **No new durable recovery grammar:** This diff adds authentication and container-validation behavior, not production journal writes or durable transitions. No new production crash window between filesystem writes was identified.
- **Authentication proof:** Host-key comparison precedes agent enumeration (`agent_auth.rs:62–75`). Success requires a callback invocation and native authenticated state, followed by cancellation/deadline checks before return (`120–127`). An accepted query alone cannot promote a connection.
- **Callback lifetime and native retries:** The per-key signer, key and context remain alive through the complete EAGAIN loop (`84–114`, `129–132`). Callback panics are contained (`198–217`). Successful signatures use Unix `malloc`, with ownership transferred once (`219–234`); inspected native paths free the transferred signature.
- **Bounded signing:** `invoked` and `downgraded` allow one normal call and one fallback call (`agent_auth.rs:199–203`). P2-1 concerns admission into that fallback state, not an unbounded retry loop.
- **Refusal versus terminal failure:** Exact agent failure returns `None` without poisoning the agent (`agent_client.rs:79–80`, `94–96`). Callback refusal permits another identity, while recorded malformed/I/O errors terminate before retry classification (`agent_auth.rs:117–118`, `135–148`). Production refusal coverage exists at `key_types.rs:154–168`.
- **SHA-1 cases:** Reviewed the three authorized cases: absent `server-sig-algs`; advertisement of `ssh-rsa` without RSA SHA-2; and one structurally valid agent SHA-1 answer to a SHA-2 request. Existing tests exercise them at `rsa_sha1.rs:48–75`, with SHA-2 preference at 77–92. `server-sig-algs` travels after key exchange under SSH transport protection; an unauthenticated attacker on the path cannot strip it. P2-1 weakens validation of the third case.
- **Security-key framing:** The signature string plus five flag/counter bytes are retained (`agent_keys.rs:114–127`). The software authenticator constructs application hash, presence flag, counter and data hash (`key_agent.py:158–171`); independent OpenSSH server acceptance covers both key types and certificates (`key_types.rs:63–114`).
- **Selected-key ownership and bounds:** Snapshot reading remains bounded and checks cancellation around reads and container validation (`ssh_key_snapshot.rs:135–181`). Native authentication consumes captured text rather than reopening a path (`ssh_key_auth.rs:43–51`). Container scanning checks cancellation in bounded chunks, rejects encrypted/unknown private labels and headers, and rejects multiple recognized keys (`ssh_key_container.rs:28–44`, `75–84`, `218–273`).
- **Platform boundaries:** Changed conditional sections use enclosing `cfg_if!` boundaries. Unix FFI and software-authenticator fixtures remain explicitly scoped. Windows bridge/allocator qualification is not claimed.

## 3. Risks and next action

Hardware-key execution remains explicitly unrun. Linux/Windows release qualification and the Windows agent bridge remain deferred release work, not findings against this Unix checkpoint.

The next action is bounded remediation of P2-1, with its production regression test and focused re-review against a newly settled tuple. Address P3-1 alongside that correction to make fixture cleanup reliable on startup failures.
