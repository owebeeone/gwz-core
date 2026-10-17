# Exact SSH agent-key selection (operator decision D38)

Date: 2026-10-18. Status: **DRAFT.** A design only: no product code is written, and nothing here is
accepted until the review the lane owner runs (Code and Safety, with the Surface concerns folded in,
because the feature adds a user-facing way to name a key). Authority: operator decisions D30 and D38
(`GwzTransportTR18-OperatorDecisions.md`). Read with
[`GwzDebtRecoveryAuthCapability.md`](../../dev-docs/GwzDebtRecoveryAuthCapability.md),
[`GwzRemoteTransportSshAgentDesign.md`](GwzRemoteTransportSshAgentDesign.md),
[`GwzTransportSshKeyTypes.md`](GwzTransportSshKeyTypes.md) and
[`GwzTransportWindowsParityDesign.md`](GwzTransportWindowsParityDesign.md) §4, §5 and §14.

## 0. Scope and summary

**Deliverable of this step:** this document. **At implementation** the repositories that may change
are gwz-core (the SSH endpoint, the driver's failure words, the native route's refusal, the capability
answers), gwz-cli (help text, troubleshooting entry, pins), gwz-py (help text and one advisory check)
and gwz-core-evidence (run archives). **Reference only, not changed:** gwz-transport (wire, pool),
gwz-sspi, gwz-git, git2-rs and libgit2. Section 5 shows why no gwz-transport change is needed.

**Recommendation in one paragraph.** `--identity PATH` (and `--remote-identity NAME=PATH`, and the
repository's `remote.<name>.gwzSshIdentity` setting) may name a **public-key file**, the `.pub` file
`ssh-keygen` writes. gwz then offers **that one key**, taken from the SSH agent, and no other: no
other agent key, no helper, no password, no file. This is OpenSSH's `-i key.pub` with
`IdentitiesOnly yes`. The endpoint tells the two kinds of file apart by content, so the public
protocol, the wire, the config key and the Python types do not change. `exact_agent_identity` becomes
true on all three platforms when the last Windows row passes (D30's condition).

**What makes this smaller than the 2026-09-07 estimate.** That estimate (5 to 10 engineering days)
priced a safe custom-credential API for git2 and agent signing under libgit2. The transport route
(the 1.1.0 default) already has its own agent client and its own `libssh2_userauth_publickey` signer
(`agent_auth.rs`), on every platform, with the three Windows and Unix agent sources behind one
`Channel` trait. Exact selection is a filter on the key list that route already fetches, plus a way to
name the key and words for the new failures. The step budget in section 7 is about 1,900 production
lines and 3,500 test and runner lines. As an estimate, not a measurement: 4 to 7 engineering days, with
dabeest bookings and one operator Pageant session setting the calendar.

## 1. What exists today

| Fact | Where |
| --- | --- |
| `--identity` takes a private-key file: "Use only this SSH private-key file; no agent fallback". Same text in gwz-py. | `gwz-cli/src/globalargs/parser.rs:52`, `gwz-py/src/gwz/cli_shared.py:308` |
| The wire identity is `{mode: ExplicitKey, key_path, path_base}`. The endpoint, not core, resolves `~/` and relative paths, and reads the file at **every** open. | `gwz-transport/src/protocol.rs:1445`, `placement_endpoint/admission.rs:578`, `ssh_key_snapshot.rs` |
| The endpoint reads the file under a supervised job, checks the container, and authenticates from memory. A key with a passphrase is refused (D17); an agent is never tried after an explicit key. | `ssh_key_snapshot.rs`, `ssh_key_container.rs`, `ssh_key_auth.rs`, `ssh_local.rs:145` |
| The ambient agent step lists the agent's keys once (`agent.identities()`), then offers each supported key in order and stops at the first the server accepts. A key of an unsupported type is skipped. | `agent_auth.rs:90`, `:93`, `:96` |
| All agent sources are `Agent<Box<dyn Channel>>` by the time `agent_auth` sees them. The key list and the signing are source-independent. | `agent_source.rs` `AgentSource::open` |
| The failure vocabulary on the wire is closed and carries no text. A limit travels as a code plus the open's `Facts` (`method`, `credential_offered`, `authenticated`), and the host words it. | `ssh_limits.rs` header, `session/driver.rs:265`, `:333` |
| `Facts.key_fingerprint` and `TransportObservation.public_key_fingerprint` exist and are never filled. | `gwz-transport/src/protocol.rs:1650`; `transport_observations.rs:128` |
| `exact_agent_identity`: the static handler answers false everywhere; the session-candidate projection answers `!cfg!(windows)`, true on macOS and Linux with nothing behind it (finding F1). gwz-py reads only `file_identity`. | `protocol/transport_capabilities.rs:15`, `transport_host/cli_placement.rs:129`, `gwz-py/src/gwz/client.py:364` |
| The native route's explicit identity is `Cred::ssh_key(user, None, path, None)`; its refusal text says exact-agent support "is unavailable". | `gitbackend/transport_support.rs:236-250` |
| gwz reads no `~/.ssh/config`. `IdentityFile` and `IdentitiesOnly` are not honoured anywhere. | no hit for either word in `gwz-core/src`; the SSH home serves `known_hosts` and `~/` paths only |

**1.0.17.** libgit2 lists the agent's keys and calls `libssh2_agent_userauth` on each in turn, moving
on after any failure (`GwzTransportSshKeyTypes.md` §1). git2's safe API has `Cred::ssh_key_from_agent(username)` only; the custom
credential libgit2 supports (an exact public key and a signing callback) is not exposed
(`GwzDebtRecoveryAuthCapability.md`). So **no 1.0.17 option selects an agent key**, and 1.1.0's native
route cannot either (git2-rs and libgit2 stay untouched). `--identity <file>.pub` on 1.0.17 hands the
public file to libssh2 as a private key and, by the source and the capability document's file-key cases (not
run for this design), fails closed with no agent fallback. The only 1.0.17
workaround is outside gwz: an agent that holds only the wanted key.

**The hazard this closes.** `GwzRemoteAuthProposal.md` §2.1: with two keys loaded, the first the
server accepts wins, and the result is a successful push under the wrong account. Today a user whose
key has a passphrase is told (D17) to load it into the agent and **drop** `--identity`, which is the
all-keys behaviour again.

## 2. The user-facing surface

### 2.1 Options

| | A. A public-key file in `--identity` (recommended) | B. New options (`--agent-identity`, `--remote-agent-identity`) | C. A fingerprint (`--identity SHA256:...`) | D. `IdentityFile` plus `IdentitiesOnly` from ssh config |
| --- | --- | --- | --- | --- |
| How it reads | `gwz fetch --identity ~/.ssh/id_work.pub` | `gwz fetch --agent-identity ~/.ssh/id_work.pub` | `gwz fetch --identity SHA256:Zk3...` | gwz parses `Host` blocks |
| Precedent | OpenSSH `-i key.pub`; `IdentityFile key.pub` | none | `ssh-add -l` output | OpenSSH |
| Public protocol (taut), generated code, gwz-py types | **unchanged** | `TransportOptions` and `RemoteIdentityRequest` gain fields: schema, two generated pairs, gwz-py, a dual review | `default_identity` unchanged | none, but a new configuration reader |
| Wire (gwz-transport) | **unchanged** | an `IdentityMode` value and an `AuthPolicy` pair, or a new field | unchanged, or a new mode | unchanged |
| Surface cost | one help sentence on three existing options; the stored setting keeps its name | three new options in two CLIs, config key, `auth identity` flag, help, gwz-py parity | a second meaning for the same string; a path can contain `:` on Windows, so `SHA256:x` is ambiguous with a drive-relative file | host aliases, `Match`, `Include`, tokens: a second configuration language and a behaviour change for every SSH remote |
| No file on disk needed | no | no | **yes** | no |
| Capability gate by the caller | advisory only (see 2.4) | exact, by field | advisory | n/a |

**Recommendation: A.** The string stays "a path to a key file"; what gwz does is decided by what the
file holds, as OpenSSH does. It is the smallest surface that names the key, it keeps `--identity` a
statement of "this key, nothing else", and it needs no schema or wire change, so there is no dual
review of a wire. C is a possible later addition and is not needed (every key `ssh-keygen` made has a
`.pub`, and `ssh-keygen -y -f KEY > KEY.pub` makes one). D is a new feature of its own and is out of
scope (section 8). If the operator prefers B, sections 3, 4, 6 and the endpoint steps are unchanged;
section 5 and Phase C change, and the wire becomes a dual review.

### 2.2 How a file is classified (rule C)

The endpoint reads the file as it does today (same job, same cap, same supervision), then:

1. If the text holds a line `-----BEGIN ` (armor), it is a private-key candidate and goes to today's
   container check, unchanged, whatever else the file holds. A file that holds a private block and a
   public line is a private key.
2. Otherwise, if it parses as **exactly one OpenSSH public key** (section 3.1), it selects an agent
   key.
3. Otherwise, if its first word is a key-type name (`ssh-`, `ecdsa-`, `sk-` or a `...-cert-v01@openssh.com`
   name) followed by a space, it is a public-key file gwz cannot use: refusal K4.
4. Otherwise it is refused as it is today.

The classification uses content only, never the file name: gwz does not derive a private-key path from
a `.pub` name, in either direction (no "use `id_x` because `id_x.pub` was named").

### 2.3 Interactions

- **`--identity` today.** A private-key path behaves exactly as before. A `.pub` is the only new input.
  Precedence is unchanged: `--remote-identity` for the remote, then `--identity`, then the repository's
  `remote.<name>.gwzSshIdentity`, then the ambient agent (all keys). A public key selected at any
  level is exact at that level.
- **`--remote-identity NAME=PATH` and two accounts on one host.** `origin=~/.ssh/work.pub` and
  `upstream=~/.ssh/home.pub` select different pool identities, so the two remotes use separate
  connections to the same host and each offers one key. This is the main use.
- **Workspace and repository configuration.** `gwz auth identity <remote> --set PATH` stores the
  resolved path in `remote.<name>.gwzSshIdentity`; availability is all it checks, so a `.pub` passes
  and nothing changes. Its help line ("a remote's local key path") says the path may be a public-key
  file. The setting name `private_key_path` in the schema and Python types stays; the docs say that a
  public key file is accepted.
- **The transport setting.** Exact selection is a feature of gwz's own transport. With `--transport
  native` (or `GWZ_TRANSPORT=native`, or `gwz.transport`), a `.pub` is refused before any key is offered
  (K5); gwz never moves an operation between transports. HTTPS remotes ignore the identity, as they do
  now.
- **gwz-py.** The same string options (`default_identity`, `remote_identities`). The client's capability
  probe (`_require_transport_capability`) may, as a courtesy, also require `exact_agent_identity` when a
  path ends in `.pub`, so that an older core says "update the core" instead of a key-format refusal.
  The suffix is advisory only: it never decides behaviour.
- **Placements.** Local and Cli placements are the same: the endpoint resolves `~/` and a relative path
  (under `endpoint_path_base`), and talks to its own process's agent.
- **A URL password.** Unchanged: libgit2 offers it before any key when the server offers password
  authentication, and a selected key follows. Nothing is added after a selected key fails.

### 2.4 Help text (proposed, for the Surface look)

`--identity PATH`: "Use only this SSH key: a private-key file, or a public-key file (.pub) to use only
that key from your SSH agent. No other key is tried." `--remote-identity NAME=PATH`: "Override the SSH
key for this remote name across selected repositories, as --identity; repeatable." The long help
carries the agent paragraph (K1's remedy) and the `--transport native` limit.

## 3. Semantics

### 3.1 The public-key file

Exactly one key: one non-blank line `TYPE BASE64 [COMMENT]`, with optional trailing whitespace and CRLF.
Refused: more than one key line, a line with authorized_keys options before the type, a type word that
differs from the type inside the blob, invalid base64, a blob over 65,536 bytes (the agent codec's
bound) or a file over 128 KiB, a file that is not UTF-8, and a type outside `agent_keys::TYPES` or an
OpenSSH certificate of one (a DSA certificate is skipped by the transport and is refused here).
Certificates (`id_x-cert.pub`) are accepted and select that certificate. The comment is ignored and is
never stored, logged or compared. The selection holds the **blob bytes** and the SHA-256 fingerprint
(`SHA256:` plus unpadded base64, as `ssh-keygen -l` prints).

### 3.2 Rules

1. **Exact match.** A listed agent identity matches when its blob is **byte-equal** to the selected
   blob. Never a prefix, a fingerprint comparison, a comment or a type comparison. A certificate does not
   match its plain key, and a plain key does not match its certificate; select the file you mean.
2. **Only the match is offered.** The list is filtered before the loop that calls `offered()`. A
   non-matching key is never passed to libssh2, so it is never queried at the server and never signed
   with. At most one offer is made (a duplicate listing is offered once).
3. **No fall-through, in any direction.** After a failure there is no other agent key, no other agent
   source (the source rules of section 4 stand), no password helper, no key file, no URL password after
   the key, no derived private-key path, and no retry that offers a different key. The setup retry
   rules are unchanged (they retry a failed key exchange while no credential was offered, not an
   authentication failure).
4. **The agent lacks the key** (the list has no match, is empty, or the agent is locked): the step ends
   **before** `offered()`, so `credential_offered` is false, and the open ends with refusal K1.
5. **Everything before the agent is unchanged:** the host key is checked first, then the agent is
   listed. The source being unusable (not running, refused `SSH_AUTH_SOCK`, no channel) keeps its
   existing words (`AgentFault`); K1 is only for a reachable agent.
6. **Algorithm and types are as for the ambient agent step.** Ed25519, ECDSA (P-256, P-384, P-521),
   RSA (SHA-2 first, `ssh-rsa` in the three cases of the key-types document §3), DSA where the server
   accepts it, security keys and certificates behave as TR2.8 lists. Selecting the key does not add or
   remove an algorithm. The server accepting none of an RSA key's algorithms (`METHOD_NONE`) is reported
   as the server not accepting the key (K3).
7. **Each open reads the file again** (as a private key file is). A changed `.pub` takes effect on the
   next open; the blob an open started with is immutable for that open.
8. **Pool eligibility.** An exact-agent open has an explicit pool identity, so a connection it made
   serves only an open that selected the same key bytes. It never serves an ambient open, a private-key
   open, or an open for another key, and the reverse (the existing rule: ambient never serves explicit).

### 3.3 Refusals (proposed texts K1 to K6, for the Surface look)

The style is the parity design's T-texts: the cause, what gwz did not do, the next action. `<file>`
is the path as the user can give it to a command (`key_file_name`). None quotes a socket path, an
agent comment or key bytes. They are proposals until the steps that build them land.

- **K1, the agent does not hold the key** (reachable agent, no match; before any offer). The source
  name is "the SSH agent", "the OpenSSH agent" or "Pageant":

  ```
  The SSH agent does not hold the key in <file>. GWZ was told to use that key and no other, so it offered the server nothing and did not try the agent's other keys. Add the key with ssh-add <the private key file> (ssh-add asks for its passphrase once), then run the command again. ssh-add -l lists what the agent holds, and ssh-keygen -lf <file> prints this key's fingerprint to compare. The agent may also be locked (ssh-add -X unlocks it). [Pageant only:] Pageant is the agent GWZ uses because a Pageant window is running; GWZ does not ask another agent.
  ```

- **K2, the agent holds the key but did not sign:**

  ```
  The SSH agent holds the key in <file> but did not sign with it. A security key may be unplugged or not touched in time, or a confirmation (ssh-add -c, or Pageant's own) may have been declined. GWZ uses that key and no other. Fix that and run the command again.
  ```

- **K3, the server did not accept the key** (`authenticated` is false; the fingerprint comes from the
  facts when the endpoint set it):

  ```
  <host> did not accept the key in <file> (<fingerprint>). GWZ offered that key and no other, so it did not try the agent's other keys. Check that the key is added to your account on <host> (ssh-keygen -lf <file> prints the fingerprint to compare), or select another key. For an RSA key this includes a server that accepts none of rsa-sha2-512, rsa-sha2-256 or ssh-rsa.
  ```

- **K4, the public-key file cannot select a key** (rule C step 3; `UnsupportedOperation`, before any
  connection work beyond admission):

  ```
  <file> cannot select an SSH agent key. It must hold exactly one OpenSSH public key on one line, as ssh-keygen writes to a .pub file, of type ssh-ed25519, ecdsa-sha2-nistp256, ecdsa-sha2-nistp384, ecdsa-sha2-nistp521, ssh-rsa, ssh-dss, sk-ssh-ed25519@openssh.com or sk-ecdsa-sha2-nistp256@openssh.com, or an OpenSSH certificate of one of those except ssh-dss. To use a private-key file, select the private key itself. If you only have the private key, ssh-keygen -y -f <the private key> > <file> writes its public key.
  ```

- **K5, the native route** (before any key is offered, at the credential callback):

  ```
  --transport native cannot use one key from the SSH agent, so <file>, a public-key file, cannot be used with it. GWZ's own transport can: omit --transport native (and unset GWZ_TRANSPORT or gwz.transport), or select a private-key file instead. gwz 1.0 had no way to select a single agent key.
  ```

- **K6, D17's words, revised** (a private key file with a passphrase; Surface to re-read an accepted
  text): the first remedy becomes "load the key with ssh-add <file> and select its public key,
  `--identity <file>.pub`, so GWZ offers that key only" (with the `ssh-keygen -y` line when no `.pub`
  exists), the second "drop `--identity` ... so GWZ offers every key the agent holds", the third the
  copy without a passphrase. The Windows RSA-PEM limit's words (`SshLimit::KeyFile`) gain the same
  sentence for ECDSA and Ed25519, which "can only come from an agent" there today.

Timeouts, Pageant's passphrase or confirmation prompt, a refused Pageant and a gone Pageant keep T6,
T6b, T10 and the gone-words unchanged: they describe the channel, not the key. Only Pageant's "no key
this transport can use" (`NO_KEY_WORDS`) yields to K1 when an exact key was selected, because Pageant
may hold other keys.

### 3.4 Encrypted keys

An encrypted private key is the case this feature exists for. The agent holds it (`ssh-add` asked for
the passphrase once), and `--identity key.pub` selects it. gwz never asks for a passphrase and never
reads the private file on this route. Pageant can hold a key whose passphrase it asks for on first use;
the wait is the existing Network-phase wait, and T6, T6b and the `--ssh-timeout` guidance apply
unchanged (parity design §5). An agent that confirms each use (`ssh-add -c`) is K2 when declined.

### 3.5 Certificates and security keys

A certificate is selected by its own file and offered as that identity (rule 1). The outcomes per
type are the ambient step's (`GwzTransportSshKeyTypes.md` §4): the transport signs Ed25519 and ECDSA
certificates, and an RSA certificate as the table says; a DSA certificate is refused at admission
(K4). A security key (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`) is selected
by its `.pub`; the agent signs and asks for the touch, and a missing device or declined touch is K2.
Real security-key and Pageant-held-certificate signatures remain unqualified (parity design §13),
as they are for the ambient step. Exact selection does not widen that claim.

### 3.6 What the user sees afterwards

On the exact route the endpoint sets `Facts.method = SshAgent` before the agent step (so the host can
word an early end), and `Facts.key_fingerprint` once the key is offered. `TransportObservation`
`credential_method` then reads `agent`, `selection_source` names where the key was chosen, and
`public_key_fingerprint` is filled (a field that exists and was never set). This answers the original
complaint that nothing in gwz's output says which identity authenticated.

## 4. Implementation shape, per platform

### 4.1 The shared part

Everything exact is **above** the `Channel` trait, so it is one implementation for all three agent
sources.

- **Parsing** (new `agent_pubkey.rs`): the public-key file to `{blob, fingerprint}`, bounded and with
  no allocation sized by the input; uses the existing `base64` and `sha2` dependencies and
  `agent_keys::KeyType::of` for the type check.
- **Admission** (`ssh_key_snapshot.rs`): `Reservation::read_with` classifies by rule C. A public key
  yields an entry of kind Agent holding the blob and fingerprint (not the file text), interned by blob
  (comment-insensitive), under the
  same registry, caps, supervision and per-open re-read. The pool token stays the registry's opaque
  token, so `Identity::Explicit` is used as it is, and no private bytes are involved.
  The classification refusal K4 travels as a new `SshLimit` variant: code `UnsupportedOperation`,
  marked `Facts.method = SshAgent`, `credential_offered = false` (the combination no existing limit
  uses; `of_failure` gains one arm).
- **The agent step** (`agent_auth.rs`): `authenticate_*` takes a choice, `Any` (today) or
  `Exact(blob)`. Exact filters the list (first byte-equal match, once), ends before `offered()` if there
  is none, and otherwise runs the existing per-key body for that one key. A declined signature or a
  `METHOD_NONE` ends the step with the failure the host words (K2 or K3) instead of moving to a next
  key, because there is none.
- **Routing** (`ssh_local.rs`): in the selected-key branch, an Agent entry goes to the exact agent
  step and returns; it never reaches the helper branch or the ambient agent call. `progress.method`
  is set to `SshAgent` before the step for every source (today only Pageant is marked early). The
  success facts are `method SshAgent`, `authenticated true`, `credential_offered true`,
  `key_fingerprint` set.
- **Words** (new module beside `agent_source/words.rs`, so `driver.rs` stays under 1,000 lines):
  K1 to K4 and K6, chosen by `open_failure_io` from the open's identity (it already holds
  `key_file`), the code and the facts. An exact open (`key_file` set and `method SshAgent`) with
  `Authentication` and no offer is K1; with an offer and `authenticated` not false it is K2; with
  `authenticated` false it is K3. Pageant's timeout, refusal and gone words are checked first, as now.

### 4.2 Per platform

| | Unix (macOS, Linux) | Windows OpenSSH agent (named pipe) | Pageant |
| --- | --- | --- | --- |
| Source | `SSH_AUTH_SOCK` socket path (`agent_socket.rs`) | the pipe `SSH_AUTH_SOCK` names, else `\\.\pipe\openssh-ssh-agent` (`agent_pipe.rs`) | the Pageant window found when the runtime was built (`agent_pageant.rs`), pinned to its process |
| Exact selection | the shared filter | the shared filter | the shared filter |
| What differs | nothing | nothing | the list may include a key whose passphrase Pageant asks for at first use, so the sign request can outlast the stall (T6, T6b); a request or reply over 8,188 payload bytes is refused (parity design §5), as for the ambient step |
| Evidence new here | two-key agent, both orders | two-key pipe agent (`agent_fixture_pipe`) | the operator-session rows (section 6) |

**Source precedence is unchanged and the exact key does not change it.** The runtime still chooses
one source once, when it is built: on Windows a Pageant window first, else a local pipe named by
`SSH_AUTH_SOCK`, else the OpenSSH default pipe (rows B06 and X9 of run `2026-10-17-pageant-operator`).
The exact key is looked up **only in that source**. A Pageant that lacks the selected key while the
pipe agent holds it is K1, with the pipe never connected to; that is X9's rule for a key that is
missing, and it needs its own row (X9e below). Both agents holding the key selects Pageant, and the
pipe sees no connection (B06e). A wrong `SSH_AUTH_SOCK` stays its existing refusal. Nothing here lets a
missing key send the open to another agent.

## 5. The protocol and the capability

**No wire change, and no change to the public protocol.** The wire `Identity` is `{ExplicitKey,
key_path, path_base}` and stays: the endpoint resolves the path and the content decides (rule C), as
it does for a private key. The pool's `Identity::Explicit(token)` stays: the token is the registry's
opaque string for the admitted entry. The failure words use the existing `Failure` codes
(`Authentication`, `UnsupportedOperation`) and `Facts` (`method`, `credential_offered`,
`authenticated`, `key_fingerprint`), all in the schema. `TransportOptions`, `RemoteSshIdentity` and
`RemoteIdentityRequest` keep their fields (`private_key_path` now also holds a `.pub` path).
`gwz-transport` is read only. Checked: gwz-transport's validation requires `credential_offered` to be
false on a reused SSH open (`codec/validate.rs:207-211`), and the reuse row (section 6.1) must assert that for an
exact-agent connection. Because nothing on the wire changes, this design
does not call for a wire dual review; if the review or the operator chooses option B, that changes.

**Capability.** `exact_agent_identity` means: the route the build carries by default can select one
agent key from a public-key file. It becomes true on all three platforms in one step (Phase E), after
D30's Windows row passes, and not before:

- the static handler (`protocol/transport_capabilities.rs`, read by gwz-py) answers true;
- the session projection (`cli_placement.rs:129`) drops its `!cfg!(windows)` split for this field;
- the qualification test that asserts false on Windows is rewritten to assert true (and `file_identity`
  stays whatever lane `cap72` decides; the capability step is sequenced after that merge and touches
  only this field).

The capability describes the default route. A process that selects `--transport native` at run time
refuses a `.pub` itself (K5); the capability request carries only a schema version, so it cannot say
"native". This is open question OQ4.

## 6. Tests

**Rules for the tests.** TDD: each row is written first and seen red against the ambient behaviour, and
the red logs are kept. A new Unix-only gate needs a Windows parity inventory row
(`scripts/checks/windows_parity/<step>.json`); the in-crate rows are written against the existing
`login_fixture` alias, which resolves to the Unix proxy-agent fixture or the Windows pipe-agent fixture,
so one test body covers both and no Unix-only gate is added. No test uses the user's agent or keys.

**The oracle.** An agent cannot witness "never offered": a key that is queried at the server but not
signed leaves no trace in the agent. The witness is the **server's log** of the public keys it was
asked about. The fixture's sshd already logs (`SshdFixture::new_debug`, `with_config("LogLevel ...")`).
Each exact row therefore (a) asserts the log shows the selected key's probe and no other key's, and
(b) has a **negative control**: the same scenario with the ambient step (no selection) shows both
keys in the log. Without (b) a silent log would pass every row. Step 2.0 first proves the control on the Unix
fixture, and step 4.2 on Windows OpenSSH 9.5p2 (whose log is held until sshd stops).

### 6.1 D30's row, and its companions

Agent holding two keys, one authorized and one not (the fixture's `bad` and `key`).

| Row | Setup | Expected |
| --- | --- | --- |
| **X1** `agent-exact-selected-authorizes` (D30) | select the authorized key's `.pub`; run with the agent listing it first, then second | authenticates; the server log shows only that key; the agent saw one list and one sign; the other key is never queried |
| **X2** `agent-exact-selected-unauthorized-no-fallthrough` | select the unauthorized key's `.pub` while the authorized key is in the agent, both orders | K3; the server log shows only the selected key; the authorized key is never queried; no authentication |
| **X3** `agent-exact-absent` | select a third key the agent lacks | K1; zero server queries, zero signs |
| X4 | empty agent; locked agent | K1; zero queries |
| X5 | agent not running / refused `SSH_AUTH_SOCK` | the existing words (`AgentFault`), no offer |
| X6 `agent-exact-declined` | the agent declines to sign the selected key | K2; no other key is offered |
| X7 | `-cert.pub` of an Ed25519 certificate (CA trusted by the fixture); plain `.pub` when the agent holds only the certificate | the certificate authenticates; the plain `.pub` is K1 |
| X8 | RSA `.pub` against a server accepting `rsa-sha2-256` only, `rsa-sha2-512` only, `ssh-rsa` only | as the ambient RSA rows, with no other key |
| X9e (Windows) | Pageant is the source and lacks the selected key; the pipe agent holds it | K1; the pipe agent sees no connection |
| B06e (Windows) | Pageant and the pipe agent both hold it | authenticates through Pageant; the pipe sees no connection |
| Pageant exact | Pageant holds two keys, select the authorized one, then the unauthorized one | as X1 and X2 |
| Fan-out | 16 members, one `.pub`, capacity one connection | one connection; one list; no open reads the private file (none exists) |
| Reuse isolation | exact K1 open, then ambient open, then exact K2 open, same host | three connections; an exact open is reused only by the same bytes; the reused open's facts validate |
| No fallback after failure | exact key rejected, with a configured helper and a URL password present | no helper and no further credential after the key (the URL password is offered first, as today) |
| Native route | `--transport native --identity k.pub` | K5, before any key is offered |
| Surface | `auth identity <remote> --set k.pub`, `--remote-identity`, the stored setting, gwz-py `default_identity` | each selects exactly; gwz-py's advisory check on an old core |

### 6.2 Unit rows

- Parser: the accepted forms (each key type; a certificate; comment with spaces; CRLF; trailing
  blank lines) and the refused (empty; two keys; options before the type; type word not equal to the
  blob's; invalid base64; blob 65,537 bytes; file over 128 KiB; non-UTF-8; unknown type; DSA
  certificate; armor plus a public line is a private candidate). Random bytes never panic.
- Matching: blob equality, and a blob that differs in one byte (same type, same length) does not match; a
  duplicate listing is offered once; matching ignores order and comment; the match is the first and
  only offer.
- The filter with a scripted `Channel` (no sshd): list `[A, B, C]` select `B` gives one `sign`, for `B`; none
  gives zero `offered()` calls and the K1 error kind; cancellation between list and offer gives no offer.
- Admission: interning by blob (two files, one key, one token); a changed file gives a new token;
  a private file is not classified as public; FIFO, missing and oversize files refuse as today.
- Words: the K texts pinned verbatim; selection of the K text by (`code`, `method`, `offered`,
  `authenticated`) including the Pageant precedence (K1 over `NO_KEY_WORDS`; T6, T6b and T10 unchanged).
- The capability answers on each platform; the qualification test; gwz-py's advisory check.

### 6.3 Gates and hosts

The lane gates of the common brief apply to each code step (fmt, the `scripts/checks` sources checks,
the boundary job, the ordinary and transport legs). Windows triggers apply (`src/git/endpoint` is an
inventory root): code commits need a `Windows-receipt`, from `windows_lane_check.py` on dabeest. The
CLI-level rows on macOS and Linux use the repository's runner shape (`tests/transport_backend`'s
`key_agent.py`); on Windows they use the parityrows runner's `rows_ssh.py` shape and its owned pipe
agent. Pageant needs a window on the desktop of the process that runs the CLI, so the Pageant rows
need one more **operator RDP session**, like B6's (`runner/run-pageant-rows.ps1`), unless the operator
accepts the in-crate Pageant fixture rows as sufficient (OQ5). Evidence goes to
`gwz-core-evidence/campaigns/transport-qualification/runs/<date>-exactagent/`, with README and
manifest, and `tools/verify_archive.py` passing.

## 7. Phased plan

Phases are milestones, foundational first. Steps are one goal each, aimed at under 500 production lines
(an aspiration). Within a phase the steps are independent unless a dependency is named, so separate
agents can take them. Each step writes its tests first.

| Phase | Milestone | Steps |
| --- | --- | --- |
| **A. The core, pure code** | Parser, filter and words exist and are unit-proven; nothing user-visible | 1.1, 1.2, 1.3 (parallel) |
| **B. The endpoint route** | In the crate, on macOS and Linux, `--identity k.pub` selects exactly: rows X1 to X8 written red first, then green | 2.0, 2.1, 2.2, 2.3 (2.0 needs 1.1 and 1.2; 2.2 needs 1.2 and 2.1's entry type; 2.3 needs 1.3) |
| **C. Surface** | Help, docs, native refusal, revised D17 words | 3.1, 3.2 (parallel; after B's words) |
| **D. Evidence on three platforms** | D30's row green through the CLI on macOS, Linux and Windows (OpenSSH pipe agent and Pageant), archived | 4.1, 4.2 |
| **E. Capability and record** | The capability answers true; the ledger and evidence closed | 5.1, 5.2 |

| Step | Goal | Files (indicative) | Lines (production + tests) |
| --- | --- | --- | --- |
| 1.1 | `agent_pubkey.rs`: parse a public-key file to `{blob, fingerprint}`, bounded; all accept/refuse rows | new `git/endpoint/agent_pubkey.rs` | 250 + 350 |
| 1.2 | Exact choice in `agent_auth`: filter, early end, one offer; scripted-channel rows | `git/endpoint/agent_auth.rs` | 120 + 250 |
| 1.3 | K1 to K4 and K6 texts as pure functions with the selection rule; pins | new `agent_source/exact_words.rs` | 250 + 250 |
| 2.0 | The in-crate rows X1 to X8, fan-out and reuse isolation on the shared fixture API (Unix proxy agent and Windows pipe agent), the oracle and its negative control; written red against the ambient step | `ssh_tests/*`, the two agent fixtures | 0 + 450 |
| 2.1 | Admission: rule C in `read_with`, Agent entry kind, interning by blob, `SshLimit` variant for K4, `of_failure` arm | `ssh_key_snapshot.rs`, `ssh_limits.rs`, `ssh_admission.rs` | 350 + 300 |
| 2.2 | Routing: Agent entry to the exact agent step, early `SshAgent` mark, success facts with fingerprint; never helpers or ambient | `ssh_local.rs`, `ssh_setup.rs` | 250 + 250 |
| 2.3 | Host: `open_failure_io` chooses K1/K2/K3/K4, Pageant precedence, source name | `transport_host/session/driver.rs`, `agent_source` | 200 + 250 |
| 3.1 | Native refusal K5; fix the native explicit-credential text (`transport_support.rs:248`) | `git/gitbackend/transport_support.rs` | 150 + 150 |
| 3.2 | Help (gwz-cli, gwz-py), `auth identity` help, troubleshooting entry and pins, revised D17 and `KeyFile` words (K6), migration note; gwz-py advisory check | gwz-cli, gwz-py, `ssh_limits.rs` | 300 + 200 |
| 4.1 | CLI-level rows X1 to X3, X6 and the native refusal on macOS and Linux (runner), archived | `tests/transport_backend`, evidence | 0 + 400 |
| 4.2 | Windows: the same rows against the OpenSSH pipe agent on dabeest (runner), X9e and B06e, and the Pageant operator-session script; archived | evidence runner | 0 + 500 |
| 5.1 | Capability flip (static handler, session projection, qualification test, gwz-py), after 4.2 passes and after `cap72` merges | `transport_capabilities.rs`, `cli_placement.rs`, tests | 60 + 100 |
| 5.2 | Route ledger cell 9.7, S5.6 table, checkpoint, decision record; evidence archive check | docs only | docs |

Total about 1,930 production lines and 3,450 test and runner lines. Step 2.0 starts as soon as 1.1 and
1.2 land, so that Phase B is written against failing rows; the Windows twin of those rows runs in the
Windows receipt of each Phase B step. 4.1 and 4.2 can run while 3.x is reviewed. Step 4.2 shares
dabeest under its lock; the Pageant rows wait for an operator session. Reviews: one Code and Safety review after Phase B on the settled tree (the new user-facing
route and the no-fall-through claim), then Surface on 3.1 and 3.2 (folded into the same review per
the lane owner), per the one-review-per-phase rule.

## 8. Risks and non-goals

- **Non-goals.** Reading `~/.ssh/config` (option D), selection by fingerprint (option C), agent
  forwarding, adding or removing agent keys, passphrase prompts, native-route support, a cross-operation
  possession proof (below), more key types.
- **For 1.2.0, not removed.** The connection reuse design's possession proof across operations
  (`GwzConnectionReuseDesign.md` §4) is not implemented in this tree. When it lands, an exact-agent
  connection's recorded key is the selected blob, and its proof is the ambient proof (the agent signs a
  nonce with that key), not "token equality implies the key", which holds only for file keys. Nothing
  here removes code for it.
- **A wrong-account success is still possible with a wrong `.pub`.** Selection is exact, and the
  server decides the account. The fingerprint in the observation and in K3 is how the user checks.
- **Content classification surprises.** A file named `.pub` that holds a private key is a private key;
  a file of any name holding one public line selects an agent key. Both follow content, stated in the
  help and covered by the rule C rows.
- **Slot charge.** A public entry uses a full registry slot and its 1 MiB charge like a private one
  (64 slots in all). Fan-out shares one entry. Shrinking the charge after the read is a possible
  later change, not needed.

## 9. Open questions for the operator

1. **OQ1, the surface (Surface).** Option A (a `.pub` in `--identity`, no schema or wire change) or B
   (new `--agent-identity` options, a schema and wire change, a dual review)? Recommendation: A.
2. **OQ2, classification.** Content only (recommended), or require the `.pub` suffix as well?
3. **OQ3, the fingerprint selector.** Defer option C past 1.1.0 (recommended)?
4. **OQ4, the capability.** The static handler answers for the default route (true) and the native
   route refuses by itself (K5). Or should the capability reflect the process's transport setting,
   which needs a protocol change (the request carries a schema version only)?
5. **OQ5, Pageant evidence.** One more operator RDP session for the Pageant rows (like B6), or accept
   the in-crate Pageant fixture plus the pipe-agent rows? Pageant shares the exact filter with the
   other sources, so the rows add evidence of Pageant's list and sign, not new code.
6. **OQ6, the observation.** Set `public_key_fingerprint` for the exact route when the key is
   offered (this fills a field that exists, and the N2 design said fingerprints stay absent unless derived
   from proven public data; a user-supplied public file is not private bytes)? Or only after
   authentication succeeds, which loses it from K3?
7. **OQ7, D17's words.** Approve revising the accepted passphrase text (K6), and the Windows
   `KeyFile` words, to name `--identity <file>.pub`.
8. **OQ8, certificates.** Exact blob equality (recommended: a certificate is selected by its file),
   or also offer the matching certificate when a plain `.pub` is selected, as OpenSSH's `CertificateFile`
   does? The second offers more than was selected.
9. **OQ9, the estimate.** The 5 to 10 day estimate in `GwzDebtRecoveryAuthCapability.md` is replaced
   by section 0's 4 to 7 days. Confirm that the library work it priced (a git2 custom credential) is
   not needed and stays untouched, per the standing rule on git2-rs.

## 10. Findings outside the scope (not authorized, not blocking)

| # | Observation and evidence | Impact | Suggestion | Blocks D38? |
| --- | --- | --- | --- | --- |
| F1 | The session projection answers `exact_agent_identity: !cfg!(windows)` (`cli_placement.rs:129`) while the static handler answers false (`transport_capabilities.rs:15`) and nothing selects an agent key. The projection is `gwz_session_candidate` code (1.2.0), so no 1.1.0 build ships it. Confirmed by reading. | An overclaim in code that is not yet released | Step 5.1 makes it true for a reason; until then, treat it as wrong if it is ever shipped | No |
| F2 | `TransportObservations::begin` starts a selected-identity row as `credential_method: file` (`transport_observations.rs:68`). A selection refused before any facts (K4) keeps `file`. | A misleading method on a refused exact row | Start as `unknown` and let the facts set it | No |
| F3 | The estimate and the "Windows needs its own investigation" sentence in `GwzDebtRecoveryAuthCapability.md` (2026-09-07) are stale for the transport route: the Windows agent sources are now implemented and run. | The document misleads a reader | A one-line note pointing here | No |
| F4 | `GwzV110RemainingPlan.md` C5 says the static handler answers `exact_agent_identity` true on Windows; the code answers false. The route ledger draft already follows the code. | Stale plan text | Correct it when the capability step lands | No |

## Review request

Code and Safety, with the Surface concerns folded in. Please check: (1) rule 2 and rule 3 against the
code path, in particular that no path from an exact open reaches `Any` or the helper branch;
(2) the oracle and its control, which is what makes "never offered" testable; (3) the claim that no
wire or public-protocol change is needed (section 5); (4) rule C's classification and its refusals;
(5) the proposed texts K1 to K6 and the help text, as new user-facing wording and one revised accepted
text (D17); (6) the Pageant and pipe precedence rows X9e and B06e; (7) requirement provenance: every
constraint above traces to D30, D38, the 1.0.17 behaviour or the existing source-precedence rows, and
the optional items (options C and D, the fingerprint in the observation) are marked as choices.
