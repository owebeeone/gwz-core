# Safety review: Exact SSH agent-key selection design (DRAFT)

- **Object:** `gwz-core/dev-docs/GwzExactAgentIdentityDesign-DRAFT.md` at gwz-core `de8e4788` (`de8e478848298f5ef257aa799a7afa657729374a`), lane `/Volumes/projects/limbo/gwz-dev-exactagent`.
- **Date:** 2026-10-18
- **Axis:** Safety (what the text lets go wrong)
- **Requirement:** D38 with D30 (`GwzTransportTR18-OperatorDecisions.md:73`, `:81`). I did not question D38's decision to build the feature.
- **Verdict: NO-GO.** P0: 0, P1: 0, P2: 4, P3: 9.

I checked the commit at the start and at the end: `de8e4788` both times. The tree is unchanged: the only untracked file, `dev-docs/GwzRemoteTransportBugReport.md`, was there before I started and is not mine. I built nothing, so there is no `CARGO_TARGET_DIR` to delete. I ran one experiment in the scratchpad (a local sshd at different LogLevels) and then deleted its files; no sshd process is left running.

## §0 Evidence base

**Code I checked against the design's claims:**
- **Agent loop** (`agent_auth.rs`):
  - The key list is fetched once (`:90`), and every listed key is offered in a loop (`:93`). Unsupported types are skipped (`:96`).
  - `offered()` runs before every native call, including each EAGAIN pass (`:113-114`).
  - `rejected()` is called **only** for `LIBSSH2_ERROR_AUTHENTICATION_FAILED` (`:60-64`).
  - A refused signature and `METHOD_NONE` end the same way (`:151-160`), and other errors return `Other` (`:161-165`).
- **Setup routing** (`ssh_local.rs`):
  - The selected-key branch returns unconditionally (`:145-160`). The helper branch (`:161-180`) and the ambient branch (`:184-215`) come after it.
  - The helper branch is reached only when `password_only` holds (`:163`).
  - `offered` sets `credential_offered=true` and `authenticated=None` (`:92-97`).
  - A URL password is offered before any key (`:124-144`).
  - The key exchange is retried only while no credential has been offered (`:110-115`).
- **Key registry** (`ssh_key_snapshot.rs`): interning compares `(key, text)` (`:128-131`). Tokens are a monotonic `selected-N` and never reused (`:137-143`). The container check runs on every admitted file (`:186`).
- **Admission and limits:**
  - `ssh_admission.rs:182-193`: `sanitized` marks every `SshLimit` as `SshKey`.
  - `ssh_limits.rs:65-76`: `of_failure` has no arm for `SshAgent`.
  - `ssh_key_container.rs:61-66`: the container check splits lines on CR as well as LF.
  - `ssh_key_container.rs:77-80`: armor must start `-----BEGIN `.
- **Error codes:** `ssh_setup.rs:600-620` maps PermissionDenied to Authentication and Unsupported to Unavailable. `ssh_setup.rs:37-50`: `Authenticated::selected` reports facts with method `SshKey`.
- **Retry:** `setup_retry.rs:63-77` retries only Io, CarrierLost, a stall or aggregate Timeout, and a refused connection. `setup_retry/requeue.rs:22-42` agrees.
- **Pool:**
  - `gwz-transport/src/pool/allocation.rs:26,167,268`: reuse requires strict identity equality.
  - `ssh_worker/runner.rs:153-166`: each URL-password open gets a unique identity. `:170-183` partitions helper-disabled opens.
  - `ssh_worker/endpoint.rs:183-186`: the selected file is re-admitted before every checkout, including reuse.
- **Agent sources:**
  - `agent_source.rs:70-78`: one source per runtime, with no fallback.
  - `agent_source.rs:129-135`: a Pageant that is gone or replaced is `NotRunning`.
  - `agent_source/select.rs:12-30`: Pageant first, else `SSH_AUTH_SOCK`, else the default pipe.
  - `agent_client.rs:39-69`: an `SSH_AGENT_FAILURE` reply to the list is PermissionDenied. The list is capped at 256 keys and the blob bound is 64 KiB (`:8`).
- **Key types:** `agent_keys.rs:56-66` checks only the type-name field of a blob.
- **Host-side wording:**
  - `pageant_words.rs:80-110`: `NO_KEY_WORDS` fires on Authentication with nothing offered.
  - `transport_host/session/driver.rs:263-273` (`agent_reason`) and `:327-360` (`open_failure_io` checks Pageant first, then the agent, then limits).
- **Native route** (`transport_support.rs`): `explicit_credential` (`:236-260`) never calls `ssh_key_from_agent`. Only `remote_credential` reaches the helper spawn (`:262-300`).
- **Route binding** (`transport_binding.rs`): the transport exists only under `cfg(gwz_transport_candidate)` (`:1-9`). A backend without a host context takes native (`:243-247`). Every SSH URL with a host context takes the transport (`:275-310`).
- **No git CLI fallback:** `transport.rs:327-331` says the Git fallback is gone, and the remaining `git` spawns are local commit and tag (`repository.rs:301`, `refs.rs:174,245`).
- **Capability answers:**
  - `protocol/transport_capabilities.rs:4-17`: the static handler, documented as "Capabilities of the native backend", is compiled into every build and answers false.
  - `transport_host/cli_placement.rs:129`: the session projection answers `!cfg!(windows)`.
  - The gwz-cli `release.yml` sets no `gwz_transport_candidate` cfg. gwz-py's README (`:130-131`) treats the candidate extension as a separate build.
- **Wire validation** (gwz-transport): `codec/validate.rs:203-212` refuses a reused open that offered a credential. `:243-251`: `key_fingerprint` is refused only alongside native facts. `admission.rs:367` bounds it.
- **sshd fixture** (`ssh_fixture.rs`): `with_config` always puts `LogLevel VERBOSE` first, and sshd keeps the first value (`:74-78`). `new_debug` runs `-ddd`, serves one connection, and on Unix has no `-E` with stderr sent to null (`:170-185`).
- **Agent fixture:** `agent_fixture.rs:117` adds the keys in a fixed order.

**Experiment (OpenSSH_10.3p1, LogLevel VERBOSE):**
- A rejected query logs `Failed publickey ... ssh2: ED25519 SHA256:Tc5B...`.
- An accepted query (PK_OK) logs `Accepted key ED25519 SHA256:... found at`.
- A certificate and its own plain key log the **same** fingerprint. Only the type token tells them apart: `ED25519-CERT SHA256:Tc5B... ID certid ...` against `ED25519 SHA256:Tc5B...`.

## Findings

### P2-1. The capability flip is not tied to the route the build actually contains

- **Location:** §5, lines 323-331 ("the static handler ... answers true"); §7 step 5.1, line 433.
- **Root cause:** The static handler is compiled into every build (`transport_capabilities.rs:4-17`, documented as the native backend's answer). The transport route exists only under `cfg(gwz_transport_candidate)` (`transport_binding.rs:1-9`), and today's release workflow, crates.io gwz-core and gwz-py's ordinary extension are built without it.
- **Violated invariant:** A capability states only what the build enforces. The design's own definition (line 323) is "the route the build carries by default".
- **Sequence:**
  1. Step 5.1 merges and makes the literal `true`.
  2. A non-candidate build answers `exact_agent_identity: true` (an ordinary gwz-py extension, any crates.io consumer, or any build before activation).
  3. gwz-py's advisory check passes.
  4. Every `.pub` reaches the native route and is refused (K5).
- **Impact:** An invented fact. It fails closed, so no credential is exposed, but it is exactly the false capability the mandate excludes.
- **Correction:**
  - Answer with the same predicate that installs `smart_transport`: `cfg!(gwz_transport_candidate)` today, or whatever activation replaces it with.
  - Update the doc comment.
  - Make the qualification test assert `false` without the cfg and `true` with it, on all three platforms.

### P2-2. A `METHOD_NONE` failure is worded as K2 ("the agent did not sign"), not K3

- **Location:** §3.2 rule 6, lines 169-173 (says K3); §4.1 "Words", lines 285-289 (an offer with `authenticated` not false is K2); §4.1 "The agent step", lines 276-279.
- **Root cause:** The `authenticated=false` bit is set only by `rejected()`, which fires only on `AUTHENTICATION_FAILED` (`agent_auth.rs:60-64`). `METHOD_NONE` takes the same `break` as a refused signature (`:151-160`). Meanwhile `offered()` has already set `credential_offered=true` and `authenticated=None` (`ssh_local.rs:92-97`).
- **Violated invariant:** Rule 6 maps "the server accepts none of an RSA key's algorithms" to K3. Under the selection function the design specifies, that case comes out as K2.
- **Sequence:**
  1. The user selects an RSA `.pub`.
  2. The server lists none of `rsa-sha2-512`, `rsa-sha2-256` or `ssh-rsa`, so libssh2 returns `METHOD_NONE` and sends nothing.
  3. The facts read: offered, `authenticated: None`.
  4. The user gets K2: "holds the key but did not sign ... security key may be unplugged".
- **Impact:** Diagnosis points at the agent when the cause is the server. Row X8's "accepts none" case has no specified expectation, so nothing would catch this.
- **Correction:** In the exact step, record `authenticated=false` on `METHOD_NONE` (call `rejected()`), or carry a distinct marker. Add an X8 row for "server accepts none of the three algorithms" that expects K3, pinned.

### P2-3. The test oracle cannot tell a certificate from its own key, and no row loads both into the agent

- **Location:**
  - §6 oracle, lines 345-351.
  - §6.1 X7, line 365.
  - §3.1, lines 147-149 (the selection "holds the blob bytes and the SHA-256 fingerprint").
  - §3.2 rule 1, lines 153-155.
- **Root cause:** sshd logs a certificate and its plain key under the same fingerprint; only the type token differs (shown above on OpenSSH 10.3p1). X7 covers only an agent that holds the certificate alone. `ssh-add id_x` loads `id_x-cert.pub` with it by default, so "both held" is the common real-world state, and no row covers it.
- **Violated invariant:** Rule 1 and Rule 2: the selected identity, and only it, is offered. A cert and a plain key are distinct credentials, and a certificate's principals can map to a different account.
- **Sequence:**
  1. The implementation matches by fingerprint, or strips `-cert-v01@openssh.com` before comparing. Either is a natural shortcut, since the selection stores the fingerprint.
  2. The user selects `id.pub` while the agent holds `id` and `id-cert`.
  3. Both credentials are offered.
  4. The oracle compares by fingerprint and sees only the "selected key", so X1 to X8 all pass.
- **Impact:** A non-selected credential is offered and the tests do not detect it. This can reproduce the wrong-account hazard D38 exists to close.
- **Correction:**
  - Add rows with the agent holding both the plain key and its certificate: select each one, in both list orders.
  - The oracle asserts on the (type token, fingerprint) pair, plus the certificate ID for certificates.
  - Each row's negative control shows both lines under the ambient step.
  - State that the oracle never matches on fingerprint alone.

### P2-4. PuTTYgen's public-key file falls through to the private-key refusal

- **Location:** §2.2 rule C, lines 92-99; K4, lines 206-211.
- **Root cause:**
  - Step 1 recognises `-----BEGIN ` (5 dashes), and so does the container (`ssh_key_container.rs:77-80`).
  - Step 3 recognises only OpenSSH type words.
  - PuTTYgen's "Save public key" writes RFC 4716 (`---- BEGIN SSH2 PUBLIC KEY ----`, 4 dashes then a space). It therefore reaches step 4, today's refusal (`InvalidInput`, then `InvalidRequest`).
  - K4's remedy (`ssh-keygen -y -f <private>`) cannot read a `.ppk`.
- **Violated invariant:** A public-key file the agent cannot use gets K4, a named cause and a working remedy, on every D38 platform, Pageant included.
- **Sequence:**
  1. A Pageant user saves the public key from PuTTYgen.
  2. They run `--identity work.pub`.
  3. They get the generic key-file refusal.
- **Impact:** It fails closed, but the standard export on the platform D38 names explicitly is never diagnosed.
- **Correction:** Either have step 3 recognise RFC 4716 armor and give K4 with "use PuTTYgen's *Public key for pasting into OpenSSH authorized_keys* box, or `ssh-keygen -i -f <file>`", or accept RFC 4716 outright (it is unambiguous). Add a parser row for it.

### P3 findings (bounded)

1. **K3 after a refused URL password, when the agent lacks the key.**
   - §4.1 words (lines 286-289) treat `credential_offered` as one bit for the whole open.
   - The password step sets `offered=true, authenticated=false` (`ssh_local.rs:124-133`).
   - If the exact step then finds no match, the user gets K3, "GWZ offered that key", which is false; the case is K1.
   - **Fix:** use `key_fingerprint.is_some()` (which §3.6 sets only when the key is offered) as the "key offered" bit.
2. **K1's remedy is wrong for Pageant** (line 190).
   - `ssh-add` and `ssh-add -l` talk to the OpenSSH agent.
   - In X9e (the pipe agent holds the key, Pageant does not), `ssh-add -l` *shows* the key while K1 says it is missing.
   - **Fix:** the Pageant variant names Pageant's "Add Key" and its key list instead of `ssh-add`.
3. **K5's classifier is not specified** (§2.3 lines 118-121; K5 lines 213-217; step 3.1 line 429).
   - Native passes the path straight to libssh2 (`transport_support.rs:250`), and nothing says whether K5 decides by suffix or by content.
   - A content read inside the libgit2 callback would need the bounded reader that refuses FIFOs (`regular_file::open`).
   - Native is also chosen with no `--transport` setting (a backend with no host context, `transport_binding.rs:243-247`), yet K5's text blames `--transport native`.
   - **Fix:** specify the classifier, the read discipline, and wording that fits every reason native was chosen.
4. **Rule C's line rule** (§3.1 line 142).
   - The container check splits on CR as well as LF (`ssh_key_container.rs:61-66`).
   - The public parser describes LF lines with an optional CRLF ending.
   - A CR-separated armor block hidden in a comment could be seen as "one public line" by one rule and as armor by the other.
   - **Fix:** use the same CR/LF split as the container, and refuse control characters in the comment.
5. **Blob structure is not validated.** `KeyType::of` checks only the type name (`agent_keys.rs:56-66`). A truncated but valid-base64 blob is accepted and can never match, giving a K1 loop with the advice "ssh-add". **Fix:** validate each family's field layout in `agent_pubkey.rs`, refuse with K4, and add a parser row.
6. **The "No fallback after failure" row can pass vacuously** (§6.1 line 372).
   - The helper branch is reachable only on a password-only server (`ssh_local.rs:161-163`).
   - The row does not fix the server's authentication methods.
   - **Fix:** require one variant on a password-only server (the helper must not run after the key) and one on a password+publickey server (the URL password is offered first), each with an ambient control.
7. **The plan's intermediate state is not stated** (§7 lines 415, 426-427).
   - Step 2.1 can land before 2.2.
   - Today an Agent entry that reaches `ssh_key_auth` fails closed: libssh2 cannot read it as a private key (`ssh_key_auth.rs:52-55`).
   - **Fix:** state the invariant for step 2.1 ("an Agent entry in the selected branch is refused; it is never filtered out of `selected`") and add a test for it, so a partial merge cannot route the entry to the helper or ambient branches.
8. **Step 3.2 lands before step 5.1.** gwz-py's advisory check (step 3.2, line 430) requires `exact_agent_identity` before the capability changes in 5.1 (line 433). gwz-py refuses every `.pub` through Phase D, so the §6.1 "Surface" row for gwz-py cannot pass there. **Fix:** move the advisory check into 5.1, or re-sequence that row.
9. **A URL password is still offered first** (§2.3 lines 128-129; help text line 133). The help says "No other key is tried", but a URL password goes before the key and can authenticate a different account, which is the same hazard. It is user-supplied and the behaviour predates this design. **Fix:** say so in the long help and in the troubleshooting entry.

## Attacks that failed

- **The exact branch reaching helpers or the ambient agent:** the selected branch returns unconditionally (`ssh_local.rs:145-160`), and the design keeps that route.
- **A pooled connection for key A serving a request for key B:**
  - Reuse requires strict identity equality (`allocation.rs:26,167,268`).
  - Tokens are monotonic and never reused.
  - The file is re-admitted before every checkout, reuse included.
  - Each URL-password open has a unique identity, and the helper-disabled partition holds.
  - The unit row "a changed file gives a new token" covers interning distinct keys apart. The design should still name the intern key as (Key, kind, blob).
- **Ambient and explicit serving each other:** blocked by the equality rule.
- **The retry machine offering a different key:** a retry re-runs the same setup with the same pinned entry, and the key exchange is retried only while nothing has been offered.
- **Bypassing source precedence (X9e):** the source is chosen once and `open()` has no fallback. A gone or replaced Pageant is `NotRunning`; the pipe is never contacted.
- **An absent or unreachable agent:** `None` is NotFound, then Unavailable, then `agent_reason`. `Refused` is an `AgentFault`. Neither reaches K1.
- **The native route falling back to the agent:** `explicit_credential` never calls `ssh_key_from_agent`, and the second callback returns an error.
- **A git CLI fetch fallback:** it is gone (`transport.rs:327-331`).
- **Misclassification that fails open:**
  - Armor wins over a public line.
  - Anything malformed is refused.
  - A private key cannot be read as public, because it needs armor.
  - An encrypted private key stays on the D17 path.
  - A symlinked or FIFO path goes through `regular_file::open`, as it does today.
- **Cross-type confusion:** byte equality includes the type string.
- **Disclosure:** nothing records the agent's list. The K texts name no socket or pipe. The only fingerprint shown is the selected public key's. The wire accepts `key_fingerprint` on non-native facts, with a bound.
- **Oracle silence:** VERBOSE logs both rejected and accepted queries with their fingerprints (seen above), and the zero-query rows have their controls. Two side notes:
  - `new_debug` is unusable for these rows: on Unix it logs to null and serves one connection.
  - `with_config("LogLevel ...")` cannot change the level, because VERBOSE is already first. VERBOSE is enough, so cite `with_config` alone.
- **X6 order:** any fall-through would offer the distinct `bad` key, and the log would show it.
- **D25:** the exact route spawns nothing, and K5 runs before any helper spawn.
- **D5:** a Pageant prompt stays in the Network phase, and a retry offers the same key.
- **Pageant's 8,188-byte limit:** unchanged, as for the ambient step.
- **A reachable agent that refuses to list:** PermissionDenied, then K1, which is reasonable.
- **A pooled connection after `ssh-add -d`:** bounded by the pool's lifetime, and §8 already records the 1.2.0 possession proof. It is not a cross-key path.

## Open questions, where safety bears on them

- **OQ1 (option A):** Safe. Every misclassification fails closed. It is subject to P2-4 and P3-4.
- **OQ2 (classification):** Content only is acceptable. Both outcomes use exactly one key named by the file, and requiring the `.pub` suffix as well adds no protection. Fix the line rule (P3-4).
- **OQ3 (fingerprint selector):** Defer it. Matching by fingerprint would bring back the certificate/plain-key confusion in P2-3.
- **OQ4 (capability):** A default-route answer plus K5 is acceptable only once the answer is tied to the compiled route (P2-1). gwz-py knows its own transport setting and can add that to its advisory check.
- **OQ5 (Pageant evidence):** The filter sits above `Channel`. What is specific to Pageant (K1 taking precedence over `NO_KEY_WORDS`, X9e, the list limit) is deterministic and can be unit-tested. In-crate rows are sufficient if they drive the real `PageantChannel` against the sshd-log oracle. Otherwise, run one operator session.
- **OQ6 (observation):** Setting the fingerprint at offer time is safe. It is the selected public blob, offered and proven, and `authenticated` reports the outcome. It also makes the P3-1 fix possible.
- **OQ7 (D17 words):** The revision helps safety: today's text steers users to the all-keys route.
- **OQ8 (certificates):** Keep exact blob equality. Offering the certificate when the plain key is selected offers more than was selected, possibly under different principals. The rows in P2-3 are needed either way.
- **OQ9 (estimate):** Agreed that git2-rs and libgit2 stay untouched. Nothing here needs them.

## Next action

The drafter revises four places:
1. §5 and step 5.1: tie the capability to the compiled route (P2-1).
2. §3.2 rule 6, §4.1 and X8: word `METHOD_NONE` as K3 (P2-2).
3. §6 and §6.1: add the certificate-plus-plain rows and the typed oracle (P2-3).
4. §2.2 and K4: handle RFC 4716 (P2-4).

The P3 corrections can be folded into the same revision. Then a re-review on this axis covers those four changes only.
