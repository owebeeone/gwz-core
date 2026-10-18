# Safety re-review, round 2: exact SSH agent-key selection (D38)

- **Object:** `gwz-core/dev-docs/GwzExactAgentIdentityDesign-DRAFT.md`, revision 1, at gwz-core `7f206c22` (`7f206c22a2b8104f5f8ebd1f61fb662f380ebe3f`), one commit over `de8e4788`. I also read `GwzExactAgentIdentityDesign-RemPlan.md`. Lane: `/Volumes/projects/limbo/gwz-dev-exactagent`.
- **Date:** 2026-10-18
- **Axis:** Safety
- **Verdict: GO.** All 13 round-1 Safety findings are closed. New findings: P0 0, P1 0, P2 0, P3 3. None is ARCHITECTURAL.

I checked the commit at the start and at the end: `7f206c22` both times. The only untracked file, `dev-docs/GwzRemoteTransportBugReport.md`, was there before I started and is not mine. I built nothing and ran no experiments this round.

## §0 Evidence base (new this round)

- **`agent_keys.rs:133-183`, `KeyType::key`:** it walks every field of every family, including the full certificate tail (serial, type, key ID, principals, validity, options, extensions, reserved, signature key, signature), and fails unless the input is fully consumed. So rule C step 6's layout check catches truncated blobs and truncated certificates.
- **`setup_retry.rs:136-158`, `merged_facts`:** a later attempt's value wins, an offer in any attempt stays offered, and `key_fingerprint` is kept with "or earlier".
  - For SSH it is applied in gwz-core by `placement_endpoint/admission.rs:360-370` (`keep_facts`), not in gwz-transport.
  - The Local placement already has a side channel to the host's words, the `Handoff` (D12 uses it for the host-key finding: `ssh_local.rs:46-47`, `driver.rs` `trust`).
- **`agent_socket.rs:67`:** a Unix connect error passes through by its kind alone, so `EACCES` becomes `PermissionDenied`. `ssh_setup.rs:604` maps that to `Authentication`. `agent_source/fault.rs:34-44` says an agent fault is "Never `PermissionDenied`", but the Unix socket path does not go through `AgentFault`.
- **`agent_client.rs:46-48`:** an `SSH_AGENT_FAILURE` reply to the key list is also `PermissionDenied`, which revised rule 4 deliberately words as K1.
- **Capability routing:**
  - In a transport build, gwz is the default transport (gwz-cli `globalargs/transport.rs:14`).
  - gwz-py's network operations take `Route` (`native/src/route.rs:17-24`); `Git2Backend::new()` is used only on the native arm or for non-network requests (`shims.rs:1-7`, `route/transport.rs:185`).
  - The session projection is under `all(gwz_transport_candidate, gwz_session_candidate, ...)` (`cli_placement.rs:7`).
- **sshd certificate lines (round-1 run, OpenSSH 10.3p1, VERBOSE):** a rejected certificate logs `Refusing certificate ID "certid" ...` and then `Failed publickey ... ED25519-CERT SHA256:<key fp> ID certid (serial 0) CA ED25519 SHA256:<ca fp>`. The design says this line was not observed; this is the line step 2.0 should pin.

## Closure of the round-1 findings

| Finding | Closure in revision 1 | Counterexample re-run | Status |
| --- | --- | --- | --- |
| **S P2-1**: capability not tied to the compiled route | §5 lines 439-455: `transport_route_installed()` uses the route's own cfg; the static handler answers with it; doc comment rewritten; the ordinary leg asserts false and the transport leg true, on three platforms. Row Capability (529); step 5.1 (618). | A non-candidate build (ordinary gwz-py, crates.io, today's release) now answers false. A candidate build defaults to gwz (`transport.rs:14`), and gwz-py's network operations take the route. | **Closed** |
| **S P2-2**: METHOD_NONE worded K2 | Rule 6 (230-233): the exact step calls `rejected()`. Table row 7 (270), §4.1 (393), row X8n (516), step 1.2 (606), unit row (546). | An RSA `.pub` against a server accepting no RSA algorithm: fingerprint set at the offer, then `authenticated: false`, so row 7 gives K3. The ambient step is unchanged. | **Closed** |
| **S P2-3**: oracle can't tell a certificate from its key; no row with both | The oracle asserts on (type token, fingerprint, ID), never the fingerprint alone (485-491). Rule 1's note (210), rule 8's intern key (239-240), rows C1-C4 (511-514), matching and admission unit rows (541-548). | With a fingerprint-match bug and the agent holding plain + cert, C1 in the cert-first order logs `ED25519-CERT` and fails. The unit row "a certificate does not match its plain key" fails too. Caught twice. | **Closed** |
| **S P2-4**: PuTTYgen RFC 4716 | Rule C step 4 (116-119) routes RFC 4716 and `.ppk` to K4. K4 names PuTTYgen's box and `ssh-keygen -i -f`, with shell-safe lines (313). Measured PowerShell forms (191-203), parser rows (539-540). | A file whose first line is `---- BEGIN SSH2 PUBLIC KEY ----` reaches step 4 and gets K4. | **Closed** |
| S P3-1: K3 after a refused URL password | The "fingerprint" bit in place of `credential_offered` (258-260); row UP (520). | Password refused (offered, `false`) then no match: no fingerprint, so row 5 gives K1. | **Closed** |
| S P3-2: K1's remedy wrong for Pageant | Three per-source K1 remedies (291-296); row X9e (517). | — | **Closed** |
| S P3-3: K5's classifier unspecified | One shared classifier, bounded read through `regular_file::open`, words that defer to the native note (104, 316-323, 148-153). | A no-host-context backend gets "This operation is using libgit2's native transport", with no guessed setting. | **Closed** |
| S P3-4: line rule and control characters | Step 1 uses the container's splitter (107); step 6 refuses control characters (127-128); unit rows include CR-only and hidden armor (534-537). | — | **Closed** |
| S P3-5: blob layout not validated | Step 6 uses `KeyType::key` (127); truncated-blob row per family (536). | Checked: `KeyType::key` consumes the full layout, certificates included. | **Closed** |
| S P3-6: "no fallback" row could pass vacuously | NF1 on a password-only server, with an ambient control that runs the helper; NF2 (523-524). | The helper branch is reachable for the control (`ssh_local.rs:161-163`), and the exact branch returns first. | **Closed** |
| S P3-7: step 2.1's intermediate state | Invariant stated and tested (386-389); row (527). | An Agent entry reaching `ssh_key_auth` fails in libssh2's from-memory read, or in `cng_reads` on Windows. Nothing is sent and no helper or ambient branch runs. | **Closed** |
| S P3-8: gwz-py's advisory check before the flip | Moved into 5.1 (156-160, 618). | — | **Closed** |
| S P3-9: URL password still offered first | §2.3 (163-167), long help (177-178), §8 (643-645), step 3.3. | — | **Closed** |

## New findings (all P3, none ARCHITECTURAL)

### N-P3-1. As worded, the negative control cannot pass in some list orders

- **Location:** §6, lines 493-495 ("the same scenario under the ambient step shows every agent key's pair"); rows X1, C1, C2, C3 and PX (504-519), which run both list orders.
- **Root cause:** In the order where the authorizing key is listed first, the ambient step authenticates with it and never offers the remaining keys. So "every agent key's pair" is false for that order (for example X1 with A first, or C3 with the certificate first).
- **Violated invariant:** A negative control must be satisfiable, and must show the line a fall-through would leave.
- **Sequence:**
  1. Step 2.0 writes the control literally.
  2. It is red forever in one order.
  3. It gets weakened or dropped in review.
- **Impact:** Bounded. Each row's other order still catches a fall-through.
- **Correction:** Define the control per row: run it in the order where the non-selected key comes before the authorizing key (or against a server that authorizes no key), and assert that the non-selected key's (type, fingerprint) pair appears.

### N-P3-2. A Unix agent socket that refuses access is worded K1, not K0

- **Location:** Rule 4 (219-221), rule 5 (222-226), table rows 2 and 5 (265, 268).
- **Root cause:** `agent_socket.rs:67` passes `EACCES` through as `PermissionDenied`, which becomes `Authentication` (`ssh_setup.rs:604`). That is the same code rule 4 assigns to "agent refuses to list" (`agent_client.rs:46-48`). Row 5 then matches before any K0 condition, because K0 needs `Unavailable`.
- **Violated invariant:** Rule 5: K1 is only for a reachable agent.
- **Sequence:**
  1. `sudo -u deploy gwz fetch --identity k.pub` runs with the invoking user's `SSH_AUTH_SOCK` preserved, or with a socket whose directory belongs to another user.
  2. The connect fails with `EACCES`.
  3. K1 says "does not hold the key ... Add the key with ssh-add".
- **Impact:** Misdiagnosis on an uncommon path. It fails closed.
- **Correction:** Map agent-socket connect errors to `AgentFault` (`NotRunning` or `Denied`, so `Unavailable`), so that row 2 (K0) applies, and add an X5u case for an unreadable socket.

### N-P3-3. Rule 9 accepts a misword on an unestablished premise

- **Location:** Rule 9 (241-245); §8 (649-651): "separating the last attempt's facts would need a wire change".
- **Root cause:** For SSH, the merge happens in gwz-core (`placement_endpoint/admission.rs:360-370`, `keep_facts`). In the Local placement (1.1.0), the endpoint already passes local findings to the host's words through the `Handoff`, as D12 does for host keys. Only the Cli placement (1.2.0, `cli_placement.rs:7`) is limited to the wire.
- **Violated invariant:** A limitation should trace to a demonstrated necessity.
- **Sequence:**
  1. Attempt 1 offers the key and stalls; the open is retried.
  2. Attempt 2 finds the agent without the key.
  3. The user gets K2, "holds the key but did not sign", with an "(attempt 2 of N)" suffix.
- **Impact:** A bounded, disclosed misdiagnosis. It is not a fall-through.
- **Correction:** Either record the final attempt's offered bit through the `Handoff` for the Local placement (no wire change), or state why that is not wanted, and keep the pinned row.

## Attacks on the revised parts that failed

- **`transport_route_installed()`:**
  - The predicate is the route's own cfg, in one place, with a test of both answers.
  - A transport build defaults to gwz; gwz-py's network operations take the route, and only its native arm uses `Git2Backend::new()`.
  - A process that selects native at run time, or an embedder with no host context, meets K5 and fails closed. This is disclosed (§5 457-461, OQ4).
  - Observation for 1.2.0, not a 1.1.0 finding: the session projection answers for the client's build, while a Cli-placement endpoint is another binary that does its own classification. An older endpoint would refuse, failing closed. When the session lands, its answer should come from what the endpoint advertises.
- **Rule 9 and retries:** a retry re-runs the same pinned entry and offers the same key. The merged facts can only misword; they cannot widen what is offered. K3's claim "offered that key and no other" stays true across attempts.
- **Rule C fail-open:** no classification can offer more than the one key the file names.
  - Armor wins, using the container's own line splitter, so the two cannot disagree.
  - A private key misread as public would select the agent copy of the same key the file names, not a wider set.
  - A one-line `known_hosts` or `@cert-authority` line that reaches step 5 can only select a host or CA key the agent does not hold, so it gets K1 or K4.
  - A UTF-16 private key gets K4. That fails closed; the words are slightly off but bounded.
  - A non-canonical base64 encoding decodes to the same blob, and matching is on the bytes.
  - Certificate layout is fully checked; a DSA certificate is refused (K4).
- **K4's table row:** without facts (the driver's own `UnsupportedOperation`) it cannot match. Ambient Pageant produces no `SshLimit`, so the arm cannot capture an ambient failure.
- **The step-2.1 invariant:** checked against `ssh_key_auth.rs:48-55`. An Agent entry fails at `key_form_limit` (Windows) or at libssh2's from-memory parse before anything is sent. Tokens are unique, so there is no pool collision.
- **Rows NF1, NF2, UP and X8n:** each control can now be satisfied, and the code path each needs exists.
- **Disclosure:** K0 to K5 quote no socket, pipe, comment or key bytes. The only fingerprint shown is the selected key's.
- **Round-1 attacks:** pool equality, source precedence, the native credential path, the absent git CLI fallback, D5 and D25 are unchanged by the revision and still fail.

## Open questions, where safety bears on them

- **OQ1 (option E):** Agree it is not recommended. One path whose credential source depends on whether the file is encrypted is a real hazard of confusion.
- **OQ2, OQ3, OQ8:** As in round 1: content only, defer the fingerprint selector, keep exact blob equality. The certificate rows now hold the line on OQ8.
- **OQ4:** Acceptable now that the answer is tied to the route. Reading the process's transport setting adds a race and buys only an earlier refusal, so I don't recommend it.
- **OQ5:** The in-crate synthetic Pageant (step 2.4) is sufficient on the Safety axis. The filter sits above `Channel`, and X9e and B06e drive the real `PageantChannel` and source selection.
- **OQ6:** Set the fingerprint at the offer. It is the bit that separates K1 from K3; setting it only on success would bring back S P3-1.
- **OQ7:** Approve. The revised K6 stops sending users to the all-keys route.
- **OQ9:** No safety bearing. git2-rs stays untouched.

## Next action

GO on the Safety axis. The three P3s can be folded into steps 2.0 (N-P3-1), 1.3 and 2.2 (N-P3-2), and 2.3 (N-P3-3) without another review round. Step 2.0 should pin the rejected-certificate line shown in §0 when it pins the oracle's line shapes.
