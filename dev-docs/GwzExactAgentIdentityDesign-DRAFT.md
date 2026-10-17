# Exact SSH agent-key selection (operator decision D38)

Date: 2026-10-18. Status: **DRAFT, revision 1** (the first draft is gwz-core `de8e4788`; both round-1
reviews returned NO-GO and section 11 maps every finding to its closure). A design only: no product
code is written, and nothing here is accepted until the re-review the lane owner runs (Code and Safety,
with the Surface concerns folded in, because the feature adds a user-facing way to name a key).
Authority: operator decisions D30 and D38 (`GwzTransportTR18-OperatorDecisions.md`). Read with
[`GwzDebtRecoveryAuthCapability.md`](../../dev-docs/GwzDebtRecoveryAuthCapability.md),
[`GwzRemoteTransportSshAgentDesign.md`](GwzRemoteTransportSshAgentDesign.md),
[`GwzTransportSshKeyTypes.md`](GwzTransportSshKeyTypes.md) and
[`GwzTransportWindowsParityDesign.md`](GwzTransportWindowsParityDesign.md) §4, §5 and §14.

## 0. Scope and summary

**Deliverable of this step:** this document. **At implementation** the repositories that may change
are gwz-core (the SSH endpoint, the driver's failure words, the native route's refusal, the capability
answers), gwz-cli (help text, docs, pins), gwz-py (help text, docstring, one advisory check) and
gwz-core-evidence (run archives). **Reference only, not changed:** gwz-transport (wire, pool),
gwz-sspi, gwz-git, git2-rs and libgit2. Section 5 shows why no gwz-transport change is needed.

**Recommendation in one paragraph.** `--identity PATH` (and `--remote-identity NAME=PATH`, and the
repository's `remote.<name>.gwzSshIdentity` setting) may name a **public-key file**, the `.pub` file
`ssh-keygen` writes. gwz then offers **that one key**, taken from the SSH agent, and no other: no
other agent key, no helper, no password, no file. This is OpenSSH's `-i key.pub` with
`IdentitiesOnly yes`. The endpoint tells the two kinds of file apart by content, so the public
protocol, the wire, the config key and the Python types do not change. `exact_agent_identity` becomes
true when the last Windows row passes (D30's condition), and only in a build that contains the
transport route (section 5).

**The estimate, like for like.** The 2026-09-07 estimate (5 to 10 engineering days) priced a macOS and
Linux library route: a git2 custom-credential API (1 to 2 days), agent enumeration and signing (2 to 4)
and fixtures and validation (2 to 4), and it left Windows to "its own capability investigation". The
transport route already has its own agent client and signer, so the first two items are not needed;
what is needed instead is the selection surface (parser, admission, routing, words, docs) and the
evidence. For the same scope (Phases A to C and step 4.1, macOS and Linux) the budget in section 7 is
about 2,000 production lines and 3,300 test lines, which I estimate at 6 to 9 engineering days, no
smaller than the old figure. Windows (step 4.2, Pageant, receipts) adds 3 to 5, and two review rounds
are on top: **9 to 14 days in all**, with dabeest bookings and one optional operator Pageant session
setting the calendar. These are estimates, not measurements, and they replace the first draft's
"4 to 7 days".

## 1. What exists today

| Fact | Where |
| --- | --- |
| `--identity` takes a private-key file: "Use only this SSH private-key file; no agent fallback". Same text in gwz-py. | `gwz-cli/src/globalargs/parser.rs:52`, `gwz-py/src/gwz/cli_shared.py:309` |
| The wire identity is `{mode: ExplicitKey, key_path, path_base}`. The endpoint, not core, resolves `~/` and relative paths, and reads the file at **every** open. | `gwz-transport/src/protocol.rs:1445`, `placement_endpoint/admission.rs:578`, `ssh_key_snapshot.rs` |
| The endpoint reads the file under a supervised job, checks the container, and authenticates from memory. A key with a passphrase is refused (D17); an agent is never tried after an explicit key. Interning is per (pool key, text); tokens are `selected-N`. | `ssh_key_snapshot.rs:128-143`, `:171-193`, `ssh_key_container.rs`, `ssh_key_auth.rs`, `ssh_local.rs:145` |
| The ambient agent step lists the agent's keys once (`agent.identities()`), then offers each supported key in order and stops at the first the server accepts. A key of an unsupported type is skipped. A refused signature and `METHOD_NONE` end a key the same way; only `AUTHENTICATION_FAILED` calls `rejected()`. | `agent_auth.rs:90`, `:93`, `:96`, `:149-160`; `ssh_local.rs:98-103` |
| All agent sources are `Agent<Box<dyn Channel>>` by the time `agent_auth` sees them. The key list and the signing are source-independent. | `agent_source.rs` `AgentSource::open` |
| The failure vocabulary on the wire is closed and carries no text. A limit travels as a code plus the open's `Facts` (`method`, `credential_offered`, `authenticated`), and the host words it. `credential_offered` and `key_fingerprint` are merged across an open's attempts (a later value wins where it has one, an offer in any attempt stays offered). | `ssh_limits.rs` header, `session/driver.rs:265`, `:333`, `setup_retry.rs:140-158`, `placement_endpoint/admission.rs:360-370` |
| The agent source words exist for the Windows pipe, Pageant and a refused `SSH_AUTH_SOCK`. A Unix `None` or `Path` source has none: the open ends as the bare `ssh setup failed: Unavailable`. | `agent_source/words.rs:12`, `driver.rs:157`, `agent_source.rs:70-78` |
| `Facts.key_fingerprint` and `TransportObservation.public_key_fingerprint` exist and are never filled; the CLI prints `fingerprint=` from the observation. | `gwz-transport/src/protocol.rs:1650`; `transport_observations.rs:128`; `gwz-cli/src/response_meta_json.rs:67` |
| `exact_agent_identity`: the static handler answers false everywhere and is documented as "Capabilities of the native backend"; the session-candidate projection answers `!cfg!(windows)`, true on macOS and Linux with nothing behind it (finding F1). gwz-py reads only `file_identity`. | `protocol/transport_capabilities.rs:4-15`, `transport_host/cli_placement.rs:129`, `gwz-py/src/gwz/client.py:371` |
| The transport route exists only under `cfg(all(gwz_transport_candidate, any(unix, windows)))`; a backend with no host context takes the native route for everything. | `git/gitbackend/transport_binding.rs:8-9`, `:243-247` |
| The native route's explicit identity is `Cred::ssh_key(user, None, path, None)`; its refusal text says exact-agent support "is unavailable". | `gitbackend/transport_support.rs:236-250` |
| gwz reads no `~/.ssh/config`. `IdentityFile` and `IdentitiesOnly` are not honoured anywhere. | no hit for either word in `gwz-core/src`; the SSH home serves `known_hosts` and `~/` paths only |
| `gwz auth identity` defaults to the active members; the root needs `--member @root` or `--all`. | `gwz-cli/docs/commands/auth.md` (finding F5) |

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

| | A. A public-key file in `--identity` (recommended) | B. New options (`--agent-identity`, `--remote-agent-identity`) | C. A fingerprint (`--identity SHA256:...`) | D. `IdentityFile` plus `IdentitiesOnly` from ssh config | E. The public key inside an encrypted private file |
| --- | --- | --- | --- | --- | --- |
| How it reads | `gwz fetch --identity ~/.ssh/id_work.pub` | `gwz fetch --agent-identity ~/.ssh/id_work.pub` | `gwz fetch --identity SHA256:Zk3...` | gwz parses `Host` blocks | `gwz fetch --identity ~/.ssh/id_work` (the private file) |
| Precedent | OpenSSH `-i key.pub`; `IdentityFile key.pub` | none | `ssh-add -l` output | OpenSSH | OpenSSH `IdentityFile id_x` with the key loaded in the agent |
| Public protocol (taut), generated code, gwz-py types | **unchanged** | `TransportOptions` and `RemoteIdentityRequest` gain fields: schema, two generated pairs, gwz-py, a dual review | `default_identity` unchanged | none, but a new configuration reader | unchanged |
| Wire (gwz-transport) | **unchanged** | an `IdentityMode` value and an `AuthPolicy` pair, or a new field | unchanged, or a new mode | unchanged | unchanged |
| Surface cost | one help sentence on three existing options; the stored setting keeps its name | three new options in two CLIs, config key, `auth identity` flag, help, gwz-py parity | a second meaning for the same string; `SHA256:x` is also a legal NTFS alternate-data-stream file name | host aliases, `Match`, `Include`, tokens: a second configuration language and a behaviour change for every SSH remote | the same path would mean a file or an agent key by whether the file is encrypted |
| No file on disk needed | no | no | **yes** | no | no |
| Capability gate by the caller | advisory only (see 2.3) | exact, by field | advisory | n/a | none possible |

**Recommendation: A.** The string stays "a path to a key file"; what gwz does is decided by what the
file holds, as OpenSSH does. It is the smallest surface that names the key, it keeps `--identity` a
statement of "this key, nothing else", and it needs no schema or wire change, so there is no dual
review of a wire. C is a possible later addition and is not needed (every key `ssh-keygen` made has a
`.pub`, and `ssh-keygen -y -f KEY` rebuilds one). D is a new feature of its own and is out of scope
(section 8). **E was weighed and is not recommended:** an OpenSSH-format encrypted private key stores
its public blob in clear, so `--identity id_x` could select the agent key with no `.pub`. But it would
make one path mean two credential sources depending on whether the file is encrypted (a behaviour
change for the existing option, today refused by D17), it works for no other container (PEM and
PKCS#8 keep no clear public blob), and it needs the private file to be read to find out. It is in OQ1.
If the operator prefers B, sections 3, 4, 6 and the endpoint steps are unchanged; section 5 and Phase C
change, and the wire becomes a dual review.

### 2.2 How a file is classified (rule C)

The endpoint reads the file as it does today (same job, same cap, same supervision). The classifier is
one pure function, `agent_pubkey::classify(bytes)`, used by admission and by the native route's
refusal (K5). It runs on the bytes **before** the UTF-8 check, and the first matching step wins:

1. **Armor.** Some line (split on CR and on LF, exactly as `ssh_key_container::line` splits, so the
   two rules cannot disagree) starts `-----BEGIN ` (ignoring one leading UTF-8 byte-order mark for this
   test only, so that a private key saved with a mark keeps today's refusal instead of K4): a private-key
   candidate, and today's container check runs unchanged, D17 included, whatever else the file holds. A
   file with a private block and a public line is a private key.
2. **Encoding near-miss.** The file starts with a UTF-16 byte-order mark (`FF FE`, `FE FF`) or a
   UTF-8 one (`EF BB BF`), or has a NUL byte in its first 256 bytes: refusal K4. (Windows PowerShell's
   `>` writes UTF-16 with a mark, section 3.1.)
3. **Not UTF-8.** Refused as today (the generic refusal), unchanged.
4. **Foreign formats.** The first non-blank line starts `---- BEGIN SSH2 PUBLIC KEY ----` (RFC 4716,
   which PuTTYgen's "Save public key" writes) or `PuTTY-User-Key-File-` (a `.ppk`): refusal K4.
   RFC 4716 is classified, not accepted: one more format to bound and parse buys nothing when K4
   names the fix, and the OpenSSH line is what every platform's tools write.
5. **Public-key territory.** The first non-blank line's first word is a key-type word (`ssh-`,
   `ecdsa-`, `sk-`, or ends `-cert-v01@openssh.com`), **or** a later word of that line (one of the
   first five) is a supported type name followed by a base64-looking word (an `authorized_keys` line
   with options in front): from here every failure is refusal K4, and a success is the selection.
6. **The selection.** Exactly one non-blank line (more is K4); the type word is a supported type
   (`agent_keys::TYPES`, or its certificate); base64 decodes to a blob of at most 65,536 bytes
   (the agent codec's bound) whose type name equals the type word and whose field layout passes
   `agent_keys::KeyType::key` (made crate-visible; no second layout checker); the rest of the line
   (the comment) holds no control character other than TAB; the file is at most 128 KiB.
7. **Anything else** is refused as today.

The classification uses content only, never the file name: gwz does not derive a private-key path from
a `.pub` name, in either direction (no "use `id_x` because `id_x.pub` was named").

### 2.3 Interactions

- **`--identity` today.** A private-key path behaves exactly as before. A public-key file is the only
  new input. Precedence is unchanged: `--remote-identity` for the remote, then `--identity`, then the
  repository's `remote.<name>.gwzSshIdentity`, then the ambient agent (all keys). A public key selected
  at any level is exact at that level.
- **`--remote-identity NAME=PATH` and two accounts on one host.** `origin=~/.ssh/work.pub` and
  `upstream=~/.ssh/home.pub` select different pool identities, so the two remotes use separate
  connections to the same host and each offers one key. This is the main use.
- **Workspace and repository configuration.** `gwz auth identity <remote> --set PATH` stores the
  resolved path in `remote.<name>.gwzSshIdentity`; availability is all it checks, so a `.pub` passes
  and nothing changes. Its help line ("a remote's local key path") says the path may be a public-key
  file. The names `private_key_path` (schema, Python types, `auth identity --json`) stay; the docs say
  that the field may hold a public-key path (step 3.3 lists the files).
- **The transport setting.** Exact selection is a feature of gwz's own transport. When the operation
  uses libgit2's native transport (`--transport native`, `GWZ_TRANSPORT=native`, `gwz.transport`, or a
  backend with no host context), a public-key file is refused before any key is offered (K5); gwz never
  moves an operation between transports. The native transport already prints which setting chose it and
  how to undo that (gwz-cli `globalargs/transport.rs:52-63`, gwz-py `route/transport.rs:103-117`), so
  K5 refers to that note and does not repeat or guess the setting.
- **HTTPS remotes.** `--identity` is ignored for them, as today. A `--remote-identity` that names a
  remote reached over HTTPS is refused, as today (gwz-cli `Troubleshooting.md`).
- **gwz-py.** The same string options (`default_identity`, `remote_identities`). Step 5.1 adds an
  advisory check to `_require_transport_capability`: when a path ends in `.pub` it also requires
  `exact_agent_identity`, so that an older core says "update the core". It is advice only: the suffix
  never decides behaviour, and the check cannot see a stored `gwzSshIdentity` setting. It lands with the
  capability (step 5.1), not before, because until then the capability is false.
- **Placements.** Local and Cli placements are the same: the endpoint resolves `~/` and a relative path
  (under `endpoint_path_base`), and talks to its own process's agent.
- **A URL password.** Unchanged: libgit2 offers it before any key when the server offers password
  authentication, and a selected key follows. So "this key and no other" describes the **keys** gwz
  offers; a password written in the remote's URL is still offered first and can authenticate another
  account. Nothing is offered after the selected key fails. The long help and the troubleshooting entry
  say so.

### 2.4 Help text (proposed, for the Surface look)

`--identity PATH`: "Use only this SSH key and no other: a private-key file without a passphrase, or a
public-key file (for example ~/.ssh/id_work.pub) to use that key from your SSH agent. gwz's own transport
only." `--remote-identity NAME=PATH`: "Override the SSH key for this remote name across selected
repositories, as --identity; repeatable." The long help adds: whether a file is a private or a public key
is decided by what it holds, not by its name; the agent must already hold the key (`ssh-add`); on
Windows the agent is Pageant if a Pageant window is running, else the OpenSSH agent; `--transport
native` refuses a public-key file; a password written in the remote's URL is still offered first when
the server accepts passwords.

## 3. Semantics

### 3.1 The public-key file

The shape is rule C steps 5 and 6 (section 2.2): exactly one OpenSSH public-key line
`TYPE BASE64 [COMMENT]`, with optional trailing whitespace and CR or LF line ends. Certificates
(`id_x-cert.pub`) are accepted and select that certificate; a DSA certificate is skipped by the
transport and is refused here. The comment is ignored and is never stored, logged or compared. The
selection holds the **blob bytes** and the SHA-256 fingerprint (`SHA256:` plus unpadded base64, as
`ssh-keygen -l` prints), not the file text.

**What the usual tools write** (these decide K4's words). `ssh-keygen -y`, `ssh-keygen` itself and
`ssh-add -L` write the accepted form. `ssh-keygen -e` and PuTTYgen's "Save public key" write RFC 4716
(step 4, K4). PuTTYgen's "Public key for pasting into OpenSSH authorized_keys file" box holds the
accepted line. **Measured** on dabeest (Windows PowerShell 5.1.26100.9711, OpenSSH for Windows 9.5p2,
2026-10-18; files deleted): Windows PowerShell 5.1's redirect `>` and `Out-File` with no encoding write
UTF-16 little-endian with a byte-order mark (`ff fe`; the 97-byte line became 196 bytes), which rule C
step 2 classifies as K4; `Set-Content -Encoding ascii` and `Out-File -Encoding ascii` write plain ASCII
with a CR LF line end (`0d 0a`, 97 bytes), which is accepted; `Set-Content -Encoding utf8` writes UTF-8
**with** a byte-order mark (`ef bb bf`), which step 2 also classifies although `ssh-keygen -l` reads it;
`ssh-keygen -i -f` of a PowerShell-saved RFC 4716 file, written back with `Set-Content -Encoding ascii`,
is accepted. `ssh-keygen -l -f` on the UTF-16 file did not return (more than five minutes; killed), so
the stock tool gives the user no help with this file and K4 has to. PowerShell 7's `>` writes UTF-8 with no
mark; it was not available on the host and was not measured.

### 3.2 Rules

1. **Exact match.** A listed agent identity matches when its blob is **byte-equal** to the selected
   blob. Never a prefix, a fingerprint comparison, a comment or a type comparison. A certificate does not
   match its plain key, and a plain key does not match its certificate; select the file you mean. (A
   certificate and its key have the same fingerprint, so a fingerprint comparison would confuse them.)
2. **Only the match is offered.** The list is filtered before the loop that calls `offered()`. A
   non-matching key is never passed to libssh2, so it is never queried at the server and never signed
   with. At most one offer is made (a duplicate listing is offered once).
3. **No fall-through, in any direction.** After a failure there is no other agent key, no other agent
   source (the source rules of section 4 stand), no password helper, no key file, no URL password after
   the key, no derived private-key path, and no retry that offers a different key. The setup retry
   rules are unchanged (they retry a failed key exchange while no credential was offered, not an
   authentication failure).
4. **The agent lacks the key** (the list has no match, is empty, the agent is locked, or it refuses to
   list): the step ends **before** `offered()` and before the fingerprint is set, and the open ends
   with refusal K1.
5. **An agent that cannot be reached.** On Unix, no `SSH_AUTH_SOCK` (source `None`) or a socket that
   does not answer (source `Path`) is refusal K0. On Windows the pipe, Pageant and a refused
   `SSH_AUTH_SOCK` keep their existing source words (`unavailable_reason`: the stopped service, the
   missing pipe, a gone or refusing Pageant). K1 is only for a reachable agent. Everything before the
   agent step is unchanged: the host key is checked first, then the agent is listed.
6. **Algorithm and types are as for the ambient agent step.** Ed25519, ECDSA (P-256, P-384, P-521),
   RSA (SHA-2 first, `ssh-rsa` in the three cases of the key-types document §3), DSA where the server
   accepts it, security keys and certificates behave as TR2.8 lists. Selecting the key does not add or
   remove an algorithm. **`METHOD_NONE`** (the server accepts none of an RSA key's algorithms) is
   recorded by the exact step as a rejection (`rejected()` is called, as it is for
   `AUTHENTICATION_FAILED`), so the facts read `authenticated: false` and the words are K3, not K2. The
   ambient step is unchanged.
7. **Each open reads the file again** (as a private key file is). A changed `.pub` takes effect on the
   next open; the blob an open started with is immutable for that open.
8. **Pool eligibility.** An exact-agent open has an explicit pool identity, so a connection it made
   serves only an open that selected the same key bytes. It never serves an ambient open, a private-key
   open, or an open for another key, and the reverse (the existing rule: ambient never serves explicit).
   The registry interns an entry by **(pool key, kind, bytes)** where kind is File or Agent and the
   bytes are the file text or the blob, never by fingerprint alone.
9. **Retried opens.** An open's final facts merge its attempts (`setup_retry::merged_facts`). If an
   earlier attempt offered the key and a later one ended before offering it, the merged facts say the
   key was offered; the words are then K2 or K3 instead of K1. The words add the existing
   `(attempt N of M)` suffix for a retried open so the user is told there was more than one attempt;
   the limitation is recorded in section 8.

### 3.3 The texts, and the one table that selects them

The style is the parity design's T-texts: the cause, what gwz did not do, the next action. `<file>`
is the path as the user can give it to a command (`key_file_name`). None quotes a socket path, an
agent comment or key bytes. Shell lines are chosen by the host build's platform, as `WINDOWS_SHELLS`
does: on Windows both PowerShell and Git Bash forms are printed, on Unix the `>` form. They are
proposals until the steps that build them land.

**The selection table.** One pure function (step 1.3) takes the failure's `code`, `effect`,
`setup_cause`, the facts (`method`, `key_fingerprint`, `authenticated`), the agent source's kind, and
whether the open selected a key file; rows are tried in order and the first match wins. "Exact open" is
`key_file` set and `facts.method = SshAgent`. "Fingerprint" is `Facts.key_fingerprint.is_some()`: the
endpoint sets it only when the selected key is offered, so it is the bit "the selected key was offered"
(not `credential_offered`, which a refused URL password also sets).

| # | Outcome | Selected when | Text | Source of the words |
| --- | --- | --- | --- | --- |
| 1 | The key file cannot select a key | `UnsupportedOperation`, method `SshAgent`, no fingerprint | **K4** | new, step 1.3 |
| 2 | The agent cannot be reached, Unix | `Unavailable` (cause `NotFound` or `ConnectionRefused`), exact open, no fingerprint, source `None` or `Path` | **K0** | new, step 1.3 |
| 3 | The agent cannot be reached, Windows | `Unavailable`, exact open, no fingerprint, source `NamedPipe`, `Pageant` or `Refused` | existing source words, unchanged (`unavailable_reason`) | `agent_source/words.rs`, `pageant_words.rs` |
| 4 | Pageant did not answer | `Timeout` with cause stall, aggregate or none, exact open, source `Pageant` | T6, T6b, or the cap text, **without** the "use --transport native" clause | `pageant_words.rs`, step 2.3 |
| 5 | The agent lacks the key | `Authentication`, exact open, no fingerprint | **K1**, one remedy per source | new, step 1.3 |
| 6 | The agent holds it but did not sign | `Authentication`, exact open, fingerprint, `authenticated` not false | **K2** | new, step 1.3 |
| 7 | The server did not accept the key | `Authentication`, exact open, fingerprint, `authenticated` false (this includes `METHOD_NONE`, rule 6) | **K3** | new, step 1.3 |
| 8 | The native route met a public-key file | not a wire failure: core's `explicit_credential`, from `agent_pubkey::classify` | **K5** | new, step 1.3 builds, step 3.1 emits |
| 9 | A private key has a passphrase | `UnsupportedOperation`, method `SshKey`, no credential offered | **K6** (D17, revised) | `ssh_limits.rs`, step 3.2 |
| 10 | Any other code, or `effect` not `None` | otherwise | the generic words, unchanged | `driver.rs:157` |

Rows 2 to 7 add `(attempt N of M)` when the open was retried (rule 9). Row 5 replaces Pageant's "no key
this transport can use" (`NO_KEY_WORDS`) for an exact open, because Pageant may hold other keys.

- **K0, the agent cannot be reached** (Unix). Source `None` says "SSH_AUTH_SOCK is not set in this
  environment"; source `Path` says "the agent SSH_AUTH_SOCK names is not answering":

  ```
  GWZ could not reach an SSH agent, so it could not use the key in <file>, and it offered the server nothing. <SSH_AUTH_SOCK is not set in this environment. | The agent that SSH_AUTH_SOCK names is not answering.> Start an agent (eval "$(ssh-agent -s)" in a POSIX shell), add the key with ssh-add <the private key file>, and run the command again. A shell started by sudo, cron or an IDE terminal may not have the SSH_AUTH_SOCK of your login session.
  ```

- **K1, the agent does not hold the key** (a reachable agent; the key was not offered). Every variant
  begins "`<The SSH agent | The OpenSSH agent | Pageant>` does not hold the key in `<file>`. GWZ was told
  to use that key and no other, so it offered the server nothing and did not try the agent's other
  keys." and then gives that source's remedy:

  ```
  [Unix] Add the key with ssh-add <the private key file> (ssh-add asks for its passphrase once), then run the command again. ssh-add -l lists what the agent holds, and ssh-keygen -lf <file> prints this key's fingerprint to compare. The agent may also be locked (ssh-add -X unlocks it).

  [Windows OpenSSH agent] Add the key with ssh-add <the private key file> (ssh-add asks for its passphrase once; the OpenSSH Authentication Agent service must be running), then run the command again. ssh-add -l lists what the agent holds, and ssh-keygen -lf <file> prints this key's fingerprint to compare.

  [Pageant] Load the key into Pageant (its icon in the notification area, then Add Key; Pageant loads PuTTY .ppk files, which PuTTYgen makes from an OpenSSH key), then run the command again. View Keys lists what Pageant holds, and ssh-keygen -lf <file> prints this key's fingerprint to compare. ssh-add talks to the OpenSSH agent and cannot add a key to Pageant. Pageant is the agent GWZ uses because a Pageant window is running, and GWZ does not ask another agent; close Pageant to use the OpenSSH agent. [SSH_AUTH_SOCK was not consulted. | when it was set]
  ```

- **K2, the agent holds the key but did not sign:**

  ```
  The SSH agent holds the key in <file> but did not sign with it. A security key may be unplugged or not touched in time, or a confirmation (ssh-add -c, or Pageant's own) may have been declined. GWZ uses that key and no other. Fix that and run the command again.
  ```

- **K3, the server did not accept the key:**

  ```
  <host> did not accept the key in <file> (<fingerprint>). GWZ offered that key and no other, so it did not try the agent's other keys. Check that the key is added to your account on <host> (ssh-keygen -lf <file> prints the fingerprint to compare), or select another key. For an RSA key this includes a server that accepts none of rsa-sha2-512, rsa-sha2-256 or ssh-rsa.
  ```

- **K4, the file cannot select a key** (rule C steps 2, 4, 5 and 6; `UnsupportedOperation`, at admission):

  ```
  <file> cannot select an SSH agent key. It must hold exactly one OpenSSH public key on one line, as ssh-keygen writes to a .pub file, of type ssh-ed25519, ecdsa-sha2-nistp256, ecdsa-sha2-nistp384, ecdsa-sha2-nistp521, ssh-rsa, ssh-dss, sk-ssh-ed25519@openssh.com or sk-ecdsa-sha2-nistp256@openssh.com, or an OpenSSH certificate of one of those except ssh-dss, saved as plain ASCII or UTF-8 text with no byte-order mark. Common causes: a file from PuTTYgen's Save public key (it starts ---- BEGIN SSH2 PUBLIC KEY ----; copy the box Public key for pasting into OpenSSH authorized_keys file into a new file instead, or convert it with ssh-keygen -i -f <file>, saving the output the way the commands below do); a PuTTY .ppk private key file (save its public key from PuTTYgen the same way); a file that Windows PowerShell wrote with > (it is UTF-16); text in front of the key type; or more than one key. To write the public key of a private key you have: [Unix and Git Bash] ssh-keygen -y -f <the private key> > <file>   [PowerShell] ssh-keygen -y -f <the private key> | Set-Content -Encoding ascii <file>. To use a private-key file instead, select the private key itself.
  ```

- **K5, the native transport** (emitted by core's native `explicit_credential`, which reads at most
  128 KiB through `regular_file::open`, as the native route already reads the key through libssh2;
  before any key is offered). `<named by>` is `--identity`, `--remote-identity` or the remote's stored
  setting, from `SelectedIdentity.source`:

  ```
  <file>, a public-key file named by <named by>, selects one key from the SSH agent, and the native transport cannot do that (gwz 1.0 could not either). This operation is using libgit2's native transport; the note printed with the command, when there is one, says how it was chosen and how to turn that off. Use gwz's own transport for it, or name a private-key file instead of <file>. [stored setting only:] gwz auth identity <remote> --unset removes the stored setting.
  ```

- **K6, D17's words, revised** (a private key file with a passphrase; Surface re-reads an accepted
  text). The remedies become: load the key with `ssh-add "<file>"` and name its public key wherever you
  named `<file>` (`--identity`, `--remote-identity`, or `gwz auth identity <remote> --set`):
  `<file>.pub`, so that gwz offers that key only (always followed by the shell-safe `ssh-keygen -y`
  line of K4, because the host cannot see whether the `.pub` exists: the endpoint resolves the path,
  and in the Cli placement it is in another process); or drop those three to let gwz offer every key
  the agent holds; or remove the passphrase from a copy and select the copy. The Windows RSA-PEM
  limit's words (`SshLimit::KeyFile`, which end "load the key into an SSH agent ... instead") gain
  "and name its public key file, `--identity <key>.pub`".

Pageant's passphrase or confirmation wait, a refused Pageant and a gone Pageant keep T6, T6b, T10 and
the gone-words as they are, except that an exact open drops the clause "or use --transport native" from
T6, T6b and the cap text: on the exact route that advice ends in K5.

### 3.4 Encrypted keys

An encrypted private key is the case this feature exists for. The agent holds it (`ssh-add` asked for
the passphrase once), and `--identity key.pub` selects it. gwz never asks for a passphrase and never
reads the private file on this route. Pageant can hold a key whose passphrase it asks for on first use;
the wait is the existing Network-phase wait, and T6, T6b and the `--ssh-timeout` guidance apply
unchanged except for the clause above (parity design §5). An agent that confirms each use
(`ssh-add -c`) is K2 when declined.

### 3.5 Certificates and security keys

A certificate is selected by its own file and offered as that identity (rule 1). `ssh-add id_x` loads
`id_x` and `id_x-cert.pub` together by default, so an agent often holds both: selecting `id_x.pub`
offers the plain key only, and selecting `id_x-cert.pub` offers the certificate only (rows C1 to C4).
The outcomes per type are the ambient step's (`GwzTransportSshKeyTypes.md` §4): the transport signs
Ed25519 and ECDSA certificates, and an RSA certificate as the table says; a DSA certificate is refused
at admission (K4). A security key (`sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com`)
is selected by its `.pub`; the agent signs and asks for the touch, and a missing device or declined
touch is K2. Real security-key and Pageant-held-certificate signatures remain unqualified (parity
design §13), as they are for the ambient step. Exact selection does not widen that claim.

### 3.6 What the user sees afterwards

On the exact route the endpoint sets `Facts.method = SshAgent` before the agent step (so the host can
word an early end), and `Facts.key_fingerprint` when the key is offered. `TransportObservation`
`credential_method` then reads `agent`, `selection_source` names where the key was chosen, and
`public_key_fingerprint` is filled (a field that exists and was never set). This answers the original
complaint that nothing in gwz's output says which identity authenticated.

## 4. Implementation shape, per platform

### 4.1 The shared part

Everything exact is **above** the `Channel` trait, so it is one implementation for all three agent
sources.

- **Parsing and classifying** (new `agent_pubkey.rs`): rule C, the bytes to `Public{blob, fingerprint}`,
  `NearMiss`, `Private` or `Other`; bounded, no allocation sized by the input; uses the existing `base64`
  and `sha2` dependencies and `agent_keys::KeyType::of` and `::key`. One function serves admission and
  K5.
- **Admission** (`ssh_key_snapshot.rs`): `Reservation::read_with` calls the classifier before the UTF-8
  check. A public key yields an entry of kind Agent holding the blob and fingerprint (not the file
  text), interned by (pool key, kind, blob) (comment-insensitive), under the same registry, caps,
  supervision and per-open re-read. The pool token stays the registry's opaque token, so
  `Identity::Explicit` is used as it is, and no private bytes are involved. K4 travels as a new
  `SshLimit` variant: code `UnsupportedOperation`, marked `Facts.method = SshAgent`, no fingerprint
  (`sanitized` in `ssh_admission.rs:182-193` marks every limit `SshKey` today and gains the variant's
  own method; `of_failure` gains one arm). **Invariant for step 2.1, tested:** until step 2.2 routes it,
  an Agent entry that reaches the selected-key branch is **refused** (it fails closed as a private-key
  read of a non-key), never filtered out of `selected` and so never sent to the helper or ambient
  branches.
- **The agent step** (`agent_auth.rs`): `authenticate_*` takes a choice, `Any` (today) or
  `Exact(blob)`. Exact filters the list (first byte-equal match, once), ends before `offered()` if there
  is none, and otherwise runs the existing per-key body for that one key. A declined signature ends the
  step with the facts K2 reads; `METHOD_NONE` calls `rejected()` first (rule 6), so the facts read K3.
  There is no next key.
- **Routing** (`ssh_local.rs`): in the selected-key branch, an Agent entry goes to the exact agent
  step and returns; it never reaches the helper branch or the ambient agent call. `progress.method`
  is set to `SshAgent` before the step for every source (today only Pageant is marked early). The
  `offered` callback of the exact step sets `key_fingerprint`. The success facts are `method SshAgent`,
  `authenticated true`, `credential_offered true`, `key_fingerprint` set.
- **Words** (new module beside `agent_source/words.rs`, so `driver.rs` stays under 1,000 lines): the
  selection table of section 3.3 as one pure function, and the K texts. `open_failure_io` calls it
  first for an exact open, with Pageant's timeout rows taking the no-native variant. `AgentSource`
  gains a `kind()` for the K1 variants.

### 4.2 Per platform

| | Unix (macOS, Linux) | Windows OpenSSH agent (named pipe) | Pageant |
| --- | --- | --- | --- |
| Source | `SSH_AUTH_SOCK` socket path (`agent_socket.rs`) | the pipe `SSH_AUTH_SOCK` names, else `\\.\pipe\openssh-ssh-agent` (`agent_pipe.rs`) | the Pageant window found when the runtime was built (`agent_pageant.rs`), pinned to its process |
| Exact selection | the shared filter | the shared filter | the shared filter |
| What differs | nothing; a stale socket (`ECONNREFUSED`) is a refused connection to the retry machine and is retried like the ambient step's (F6) | nothing | the list may include a key whose passphrase Pageant asks for at first use, so the sign request can outlast the stall (T6, T6b); a request or reply over 8,188 payload bytes is refused (parity design §5), as for the ambient step |
| Unreachable agent | K0 | the existing service and pipe words | the existing gone and refused words |
| Evidence new here | two-key agent, both orders | two-key pipe agent (`agent_fixture_pipe`) | the in-crate synthetic Pageant (step 2.4) and, if the operator asks for it, one real-Pageant session (step 4.2) |

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
(`Authentication`, `Unavailable`, `UnsupportedOperation`) and `Facts` (`method`, `credential_offered`,
`authenticated`, `key_fingerprint`), all in the schema. `TransportOptions`, `RemoteSshIdentity` and
`RemoteIdentityRequest` keep their fields (`private_key_path` now also holds a `.pub` path).
`gwz-transport` is read only. Checked: gwz-transport ties neither `ExplicitKey` nor `SshExplicit` to
`Facts.method`; its validation requires `credential_offered` to be false on a reused SSH open
(`codec/validate.rs:207-211`), and the reuse row (section 6.1) must assert that for an exact-agent
connection. Because nothing on the wire changes, this design does not call for a wire dual review; if
the review or the operator chooses option B, that changes.

**Capability.** `exact_agent_identity` means: the route this build carries can select one agent key
from a public-key file. It is **tied to the compiled route**, not to a literal. The static handler is
compiled into every build, including the ones without the transport (today's release workflow, an
ordinary gwz-py extension, crates.io consumers), so a literal `true` would be an invented fact there.
Step 5.1 therefore:

- adds one function, `transport_route_installed()`, beside the route's own cfg in
  `git/gitbackend/transport_binding.rs`, with the same predicate that compiles the route
  (`all(gwz_transport_candidate, any(unix, windows))` today; the activation step C4 replaces that cfg
  and the function with it, in one place, so the two cannot drift);
- makes the static handler answer `exact_agent_identity: transport_route_installed()`, and rewrites its
  doc comment, which says "of the native backend";
- drops `!cfg!(windows)` from the session projection's field (`cli_placement.rs:129`);
- rewrites the qualification test that asserts false on Windows, and the static handler's unit test:
  the ordinary leg asserts **false**, the transport leg asserts **true**, on macOS, Linux and Windows.
  (`file_identity` stays whatever lane `cap72` decides; the step is sequenced after that merge and
  touches only this field.)

It is flipped in Phase E, after D30's Windows row passes, and not before. The capability describes
the build's route, not the embedder's wiring: a core embedder that builds a backend with no host
context takes the native route for everything (TR2.11) and meets K5. A process that selects
`--transport native` at run time also meets K5. See OQ4 for whether the answer should also read the
process's transport setting.

## 6. Tests

**Rules for the tests.** TDD: each row is written first and seen red against the ambient behaviour, and
the red logs are kept. A new Unix-only gate needs a Windows parity inventory row
(`scripts/checks/windows_parity/<step>.json`); the in-crate rows are written against the existing
`login_fixture` alias, which resolves to the Unix proxy-agent fixture or the Windows pipe-agent fixture,
so one test body covers both and no Unix-only gate is added. No test uses the user's agent or keys.

**The oracle.** An agent cannot witness "never offered": a key that is queried at the server but not
signed leaves no trace in the agent. The witness is the **server's log** of the public keys it was
asked about, read from `SshdFixture::with_config` (which logs at `VERBOSE` to `sshd.log`,
`ssh_fixture.rs:74-82`; `with_config` is cited without a `LogLevel` line because the fixture's own
comes first and sshd keeps the first value, and `new_debug` is not used: on Unix it logs to nowhere and
serves one connection). Observed on OpenSSH_10.3p1 (macOS), with a certificate and its own plain key
held in one agent, a rejected plain key, an authorized plain key and an authorized certificate:

| Query | Line sshd writes (fingerprints shortened) |
| --- | --- |
| a plain key, rejected | `Failed publickey for U ssh2: ED25519 SHA256:aaa` |
| a plain key, accepted | `Accepted key ED25519 SHA256:bbb found at <authorized_keys>:1`, then `Accepted publickey for U ssh2: ED25519 SHA256:bbb` |
| a certificate, accepted | `Accepted certificate ID "id" (serial 0) ...`, then `Accepted publickey for U ssh2: ED25519-CERT SHA256:aaa ID id (serial 0) CA ED25519 SHA256:ca` |

A certificate and its own key log the **same fingerprint**; only the type token (`ED25519` against
`ED25519-CERT`) and the certificate ID tell them apart. So the oracle parses each line into a
(**type token**, **fingerprint**) pair plus the certificate ID where there is one, and every assertion is
on that pair, **never on the fingerprint alone**. The line shapes are per sshd version, so step 2.0
pins them on each fixture (macOS, Linux, and Windows OpenSSH 9.5p2, whose log is held until sshd
stops) before any green step; a rejected certificate query's line was not observed here and is part of
that step.

Each exact row has a **negative control**: the same scenario under the ambient step shows every
agent key's pair in the log. Without it a silent log would pass every row. The Windows control is
proven in Phase B (step 2.0's Windows receipt), not only in step 4.2.

### 6.1 D30's row, and its companions

The agent holds keys A (authorized by the server) and B (not); the fixture adds B first, then A, and
the rows run both list orders. Keys are named A and B here, and K0 to K6 are texts only.

| Row | Setup | Expected |
| --- | --- | --- |
| **X1** `agent-exact-selected-authorizes` (D30) | select A's `.pub`; the agent lists A first, then second | authenticates; the log shows only A's pair; the agent saw one list and one sign; B is never queried |
| **X2** `agent-exact-selected-unauthorized-no-fallthrough` | select B's `.pub` while A is in the agent, both orders | K3; the log shows only B's pair; A is never queried; no authentication |
| **X3** `agent-exact-absent` | select a third key the agent lacks | K1; zero queries, zero signs; the facts carry no fingerprint |
| X4 | empty agent; locked agent | K1; zero queries |
| **X5u** (Unix) | `SSH_AUTH_SOCK` unset or empty; then a socket path whose agent is gone | K0 (the first says "not set", the second "not answering"); no offer; the second shows its attempts |
| **X5w** (Windows) | the OpenSSH service stopped (default pipe absent); a pipe `SSH_AUTH_SOCK` names that does not exist; a refused `SSH_AUTH_SOCK`; a gone Pageant | the existing words of each source, unchanged, no offer |
| X6 `agent-exact-declined` | the agent declines to sign the selected key | K2; no other key is offered |
| **C1** | the agent holds A and A's certificate; select A's plain `.pub`; both list orders | the log shows `ED25519 <A>` and no `ED25519-CERT` line |
| **C2** | the same agent; select the `-cert.pub`; both orders | the log shows the `ED25519-CERT` pair with its ID and no plain-key line |
| **C3** | only A's certificate is authorized (CA trust), the plain key is not; select the plain `.pub` | K3; the certificate is never offered, the open never authenticates through it |
| **C4** | the agent holds only the certificate; select the plain `.pub`; then the agent holds only the plain key and the certificate is selected | K1 in both |
| X8 | RSA `.pub` against a server accepting `rsa-sha2-256` only, `rsa-sha2-512` only, `ssh-rsa` only | as the ambient RSA rows, with no other key |
| **X8n** | an RSA `.pub` against a server whose `PubkeyAcceptedAlgorithms` lists none of the three RSA algorithms (`with_config`) | K3 (pinned), with the ambient control showing `METHOD_NONE` in the ambient step's facts |
| X9e (Windows, in-crate synthetic Pageant) | Pageant is the source and lacks the selected key; the pipe agent holds it | K1 with the Pageant remedy; the pipe agent sees no connection |
| B06e (Windows, in-crate) | Pageant and the pipe agent both hold it | authenticates through Pageant; the pipe sees no connection |
| PX (Windows, in-crate) | the synthetic Pageant holds A and B; select A, then B | as X1 and X2 |
| **UP** | a server with password and public-key authentication, a URL password it refuses, the agent lacks the selected key | K1, not K3: the facts have `credential_offered` and `authenticated: false` from the password, and no fingerprint |
| Fan-out | 16 members, one `.pub`, capacity one connection | one connection; one list; no file is read for a key |
| Reuse isolation | an open selecting A, then an ambient open, then an open selecting B, same host | three connections; an exact open is reused only by the same key bytes; the reused open's facts validate |
| **NF1** | a password-only server (`PubkeyAuthentication no`), a helper configured, the exact key selected | K3; the helper runs zero times; the ambient control runs the helper |
| **NF2** | a password-and-public-key server, a URL password, a helper | the URL password is offered first (as today), then the exact key, and then nothing; the helper runs zero times; the ambient control differs |
| Retry spanning | attempt 1 offers the key and stalls (retried), attempt 2 finds the agent without it | the words carry `(attempt 2 of N)`; the limitation of rule 9 is pinned, not hidden |
| Native route (K5) | a `.pub` named by `--identity`, by `--remote-identity`, by the stored setting, and a backend with no host context | K5 with the right "named by" clause and the unset line only for the stored setting; no key is offered |
| Intermediate state (2.1) | an Agent entry reaches the selected-key branch before 2.2 | refused; no helper and no ambient agent are touched |
| Surface | `auth identity <remote> --set k.pub`, `--remote-identity`, the stored setting, gwz-py `default_identity` | each selects exactly; gwz-py's advisory check on an old core (step 5.1) |
| Capability | the static handler and the session projection on each platform | false without the transport route, true with it (step 5.1) |

### 6.2 Unit rows

- Parser and classifier, one row each: each key type; a certificate; a comment with spaces; CRLF and
  CR-only line ends; trailing blank lines; and the refusals: empty; two keys; a line with
  `authorized_keys` options in front; the type word not equal to the blob's; invalid base64; a
  truncated blob per family (the layout check); a blob of 65,537 bytes; a file over 128 KiB; a control
  character in the comment; a CR-separated armor block hidden in a comment (armor wins, so it is a
  private candidate); non-UTF-8 bytes (generic refusal); a UTF-8 BOM; a UTF-16LE file with a BOM;
  a NUL in the first 256 bytes; RFC 4716 as `ssh-keygen -e` writes it and as PuTTYgen writes it; a
  `.ppk` header; an unknown type (`ssh-xmss@openssh.com`); a DSA certificate. Random bytes never panic.
- Matching: blob equality; a blob that differs in one byte (same type, same length) does not match; a
  certificate does not match its plain key and the reverse; a duplicate listing is offered once;
  matching ignores order and comment.
- The filter with a scripted `Channel` (no sshd): list `[A, B, C]` select `B` gives one `sign`, for `B`;
  none gives zero `offered()` calls and the K1 error kind; cancellation between list and offer gives no
  offer; `METHOD_NONE` calls `rejected()`.
- Admission: interning by (pool key, kind, blob): two files, one key, one token; a changed file gives a
  new token; a plain key and its certificate get different tokens; a private file is not classified as
  public; FIFO, missing and oversize files refuse as today.
- Words: the selection table, one row each, in order, including the Pageant precedence (K1 over
  `NO_KEY_WORDS`), the no-native variants of T6, T6b and the cap text, the three K1 remedies, the
  retried-open suffix, the Windows two-shell lines and the Unix one; every K text pinned verbatim.
- The capability answers on each platform; the qualification test; gwz-py's advisory check.

### 6.3 Gates and hosts

The lane gates of the common brief apply to each code step (fmt, the `scripts/checks` source checks,
the boundary job, the ordinary and transport legs). Windows triggers apply (`src/git/endpoint` is an
inventory root): code commits need a `Windows-receipt`, from `windows_lane_check.py` on dabeest. The
CLI-level rows on macOS and Linux use the repository's runner shape (`tests/transport_backend`'s
`key_agent.py`); on Windows they use the parityrows runner's `rows_ssh.py` shape and its owned pipe
agent. The Pageant rows are in-crate and unattended (the synthetic Pageant of `pageant_endpoint.rs`,
step 2.4). Real Pageant 0.83 needs a window on the desktop of the process that runs the CLI, so rows
against it (`real_pageant.rs`'s ignored rows, extended) need one more **operator RDP session**, like
B6's, unless the operator accepts the in-crate rows as sufficient (OQ5). Evidence goes to
`gwz-core-evidence/campaigns/transport-qualification/runs/<date>-exactagent/`, with README and
manifest, and `tools/verify_archive.py` passing.

## 7. Phased plan

Phases are milestones, foundational first. Steps are one goal each, aimed at under 500 production lines
(an aspiration). Each step writes its tests first. The graph below lists every dependency edge; a step
with no edge to another can be taken by a separate agent at the same time.

| Phase | Milestone | Steps |
| --- | --- | --- |
| **A. The core, pure code** | Classifier, filter and words exist and are unit-proven; nothing user-visible | 1.1, 1.2, 1.3 |
| **B. The endpoint route** | In the crate, on all three platforms, an open that names a `.pub` selects exactly; rows X1 to X9e, C1 to C4 written red first, then green | 2.0, 2.1, 2.2, 2.3, 2.4 |
| **C. Surface** | The native refusal, the revised D17 words, help and docs | 3.1, 3.2, 3.3 |
| **D. Evidence through the CLI** | D30's row green through the CLI on macOS, Linux and Windows, archived | 4.1, 4.2 |
| **E. Capability and record** | The capability answers true in a transport build; the ledger closed | 5.1, 5.2 |

**Dependency edges** (a step starts when everything after "needs" has landed):

| Step | Needs |
| --- | --- |
| 1.1, 1.2, 1.3 | nothing (parallel) |
| 2.0 | 1.1, 1.2 |
| 2.1 | 1.1 |
| 2.2 | 1.2, 2.1 |
| 2.3 | 1.3, 2.1, 2.2 |
| 2.4 | 2.2, 2.3 |
| 3.1 | 1.1, 1.3 |
| 3.2 | 1.3 |
| 3.3 | 2.3, 3.1, 3.2 |
| 4.1 | 2.3, 3.1 |
| 4.2 | 2.3, 2.4, 3.1 |
| 5.1 | 3.3, 4.1, 4.2, and lane `cap72`'s merge |
| 5.2 | 5.1 |

Steps 2.0 and 2.1 are parallel; 3.1 and 3.2 are parallel with Phase B.

| Step | Goal | Files (indicative) | Lines (production + tests) |
| --- | --- | --- | --- |
| 1.1 | `agent_pubkey.rs`: the classifier (rule C) and the selection parser, bounded; `KeyType::key` made crate-visible; every accept and near-miss row | new `git/endpoint/agent_pubkey.rs`, `agent_keys.rs` | 300 + 450 |
| 1.2 | Exact choice in `agent_auth`: filter, early end, one offer, `METHOD_NONE` as rejected; scripted-channel rows | `git/endpoint/agent_auth.rs` | 140 + 300 |
| 1.3 | The selection table as one pure function; K0 to K5 texts, the K1 variants, the shell-safe `ssh-keygen -y` lines, `AgentSource::kind()`; every text pinned | new `agent_source/exact_words.rs` | 300 + 300 |
| 2.0 | The in-crate rows (section 6.1 except the Windows-Pageant and capability rows) on the shared fixture API (Unix proxy agent and Windows pipe agent), with a server log oracle on (type, fingerprint), its negative controls proven on each fixture's sshd including Windows; written red against the ambient step | `ssh_tests/*`, the two agent fixtures, `ssh_fixture.rs` | 0 + 550 |
| 2.1 | Admission: rule C in `read_with`, Agent entry kind, interning by (pool key, kind, blob), the `SshLimit` variant for K4 with its own method mark, the `of_failure` arm, the refusal invariant of section 4.1 | `ssh_key_snapshot.rs`, `ssh_limits.rs`, `ssh_admission.rs` | 350 + 300 |
| 2.2 | Routing: Agent entry to the exact agent step, early `SshAgent` mark for every source, fingerprint at the offer, success facts; never helpers or ambient | `ssh_local.rs`, `ssh_setup.rs` | 250 + 250 |
| 2.3 | Host: `open_failure_io` calls the table first for an exact open; the no-native variants of T6, T6b and the cap text; the retried-open suffix | `transport_host/session/driver.rs`, `agent_source` | 200 + 250 |
| 2.4 | Windows in-crate rows X9e, B06e and PX, driving the real `PageantChannel` and source selection against the synthetic Pageant window, and the Pageant K1 remedy, as twins of `pageant_endpoint.rs`'s B06 and X9 | `ssh_tests/pageant_endpoint.rs` | 0 + 250 |
| 3.1 | Native refusal K5 from the shared classifier, with the named-by clause; the native explicit-credential text (`transport_support.rs:248`) | `git/gitbackend/transport_support.rs` | 180 + 200 |
| 3.2 | K6 (D17 revised) and the `KeyFile` words, sole owner of the encrypted-key and Windows-limit texts | `git/endpoint/ssh_limits.rs` | 120 + 150 |
| 3.3 | Help (gwz-cli, gwz-py), `auth identity` help, and the docs: `gwz-cli/docs/commands/auth.md` (its "remains unsupported" paragraph), `Releases.md` (the two "remains unsupported" lines and the migration note), `Troubleshooting.md` (the D17 entry and a new one for K0 to K5), `MachineOutput.md` and `gwz-core/docs/MessageCatalog.md` (the field may hold a public-key path; the fingerprint is now filled), the gwz-py `RemoteSshIdentity` docstring; pins | gwz-cli, gwz-py, gwz-core docs | 150 + 150 |
| 4.1 | CLI-level rows X1 to X3, X5u, X6, C1 to C3 and the native refusal on macOS and Linux (runner), archived | `tests/transport_backend`, evidence | 0 + 400 |
| 4.2 | Windows: the same rows against the OpenSSH pipe agent on dabeest (runner), X5w, and the real-Pageant operator-session script if OQ5 asks for it; archived | evidence runner | 0 + 500 |
| 5.1 | The capability tied to the route (section 5), the doc comment, the qualification tests, and gwz-py's advisory check | `transport_binding.rs`, `transport_capabilities.rs`, `cli_placement.rs`, gwz-py `client.py`, tests | 100 + 150 |
| 5.2 | Route ledger cell 9.7, S5.6 table, checkpoint, decision record; evidence archive check | docs only | docs |

Total about 2,090 production lines and 4,200 test and runner lines (the like-for-like scope of section 0
is the sum without 2.4, 4.2 and 5.1: about 1,990 and 3,300). Step 2.0 starts as soon as 1.1 and 1.2
land, so that Phase B is written against failing rows; the Windows twin of those rows runs in the
Windows receipt of every Phase B step.

**Reviews, placed.** One Code and Safety review after Phase B on the settled tree (the no-fall-through
claim and the oracle; it also re-reads rule C and the table); one Surface and Consistency review after
Phase C, on the texts and help as built (K0 to K6, the help, the docs, the shell lines), before the
evidence steps 4.1 and 4.2 pin them; the evidence and capability steps (Phases D and E) take the lane
owner's S7.5 review. This is the one-review-per-phase rule, with Surface not folded into a review that
precedes the texts it reads.

## 8. Risks and non-goals

- **Non-goals.** Reading `~/.ssh/config` (option D), selection by fingerprint (option C), the private
  file's embedded public key (option E), agent forwarding, adding or removing agent keys, passphrase
  prompts, native-route support, a cross-operation possession proof (below), more key types.
- **For 1.2.0, not removed.** The connection reuse design's possession proof across operations
  (`GwzConnectionReuseDesign.md` §4) is not implemented in this tree. When it lands, an exact-agent
  connection's recorded key is the selected blob, and its proof is the ambient proof (the agent signs a
  nonce with that key), not "token equality implies the key", which holds only for file keys. Nothing
  here removes code for it.
- **A wrong-account success is still possible with a wrong `.pub` or a URL password.** Selection is
  exact, and the server decides the account. The fingerprint in the observation and in K3 is how the
  user checks; the URL password is stated in the help.
- **Content classification surprises.** A file named `.pub` that holds a private key is a private key;
  a file of any name holding one public line selects an agent key. Both follow content, stated in the
  help and covered by the rule C rows.
- **Retried opens can misword K1 as K2 or K3** (rule 9): the merged facts of an open keep an earlier
  attempt's offer. The suffix and a pinned row make it visible; separating the last attempt's facts
  would need a wire change.
- **Slot charge.** A public entry uses a full registry slot and its 1 MiB charge like a private one
  (64 slots in all). Fan-out shares one entry. Shrinking the charge after the read is a possible
  later change, not needed.
- **The capability describes the build, not the embedder** (section 5).

## 9. Open questions for the operator

1. **OQ1, the surface (Surface).** Option A (a `.pub` in `--identity`, no schema or wire change), B
   (new `--agent-identity` options, a schema and wire change, a dual review) or E (the public blob of an
   encrypted OpenSSH-format private file, no `.pub` needed, with the costs in section 2.1)?
   Recommendation: A.
2. **OQ2, classification.** Content only (recommended: a suffix rule would not fix RFC 4716), or require
   the `.pub` suffix as well?
3. **OQ3, the fingerprint selector.** Defer option C past 1.1.0 (recommended)?
4. **OQ4, the capability.** The static handler answers for the build's route (recommended), tied to
   `transport_route_installed()`, and a process that selects native at run time meets K5. Or should it
   also read the process's transport setting? That needs no protocol change: gwz-py's capability call
   runs in-process and `transport_setting::resolve` over the environment snapshot is available there. The
   costs are a new input to the handler and a race (the environment can change between the probe and the
   operation), and gwz-cli never calls the capability.
5. **OQ5, Pageant evidence.** In-crate synthetic-Pageant rows (step 2.4) plus the pipe-agent CLI rows
   (recommended: D30 asks for one Windows exact row from any agent, and real Pageant's list and sign
   were already proven for the ambient step on 2026-10-17), or one more operator RDP session against
   real Pageant 0.83?
6. **OQ6, the observation.** Set `public_key_fingerprint` for the exact route when the key is
   offered (recommended: the blob is byte-equal to the agent's listed key, public data the user named;
   the N2 design's rule that fingerprints stay absent unless derived from proven public data,
   `GwzRemoteTransportSshSelectedIdentityDesign.md:47` and `GwzRemoteTransportSshProductionSetup.md:102`,
   was written for fingerprints derived from private snapshots)? Or only after authentication succeeds,
   which loses it from K3 and from the K1/K2/K3 selection bit?
7. **OQ7, D17's words.** Approve revising the accepted passphrase text (K6), and the Windows
   `KeyFile` words, to name `<file>.pub` wherever `<file>` was named.
8. **OQ8, certificates.** Exact blob equality (recommended: a certificate is selected by its file),
   or also offer the matching certificate when a plain `.pub` is selected, as OpenSSH's `CertificateFile`
   does? The second offers more than was selected, possibly under different principals.
9. **OQ9, the estimate.** Section 0 restates it like for like: 6 to 9 days for the old scope, 9 to 14 in
   all. Confirm that the library work the old estimate priced (a git2 custom credential) is not needed
   and stays untouched, per the standing rule on git2-rs.

## 10. Findings outside the scope (not authorized, not blocking)

| # | Observation and evidence | Impact | Suggestion | Blocks D38? |
| --- | --- | --- | --- | --- |
| F1 | The session projection answers `exact_agent_identity: !cfg!(windows)` (`cli_placement.rs:129`) while the static handler answers false (`transport_capabilities.rs:15`) and nothing selects an agent key. The projection is `gwz_session_candidate` code (1.2.0), so no 1.1.0 build ships it. Confirmed by reading. | An overclaim in code that is not yet released | Step 5.1 makes it true for a reason, tied to the route; until then, treat it as wrong if it is ever shipped | No |
| F2 | `TransportObservations::begin` starts a selected-identity row as `credential_method: file` (`transport_observations.rs:75`). A refusal that arrives with no facts (K5, and a native failure) keeps `file`; K4 carries facts with method `SshAgent`, which `facts()` maps to `agent`. | A misleading method on a row that never got facts | Start as `unknown` and let the facts set it | No |
| F3 | The estimate and the "Windows needs its own investigation" sentence in `GwzDebtRecoveryAuthCapability.md` (2026-09-07) are stale for the transport route: the Windows agent sources are now implemented and run. | The document misleads a reader | A one-line note pointing here | No |
| F4 | `GwzV110RemainingPlan.md` C5 says the static handler answers `exact_agent_identity` true on Windows; the code answers false. The route ledger draft already follows the code. | Stale plan text | Correct it when the capability step lands | No |
| F5 | `gwz auth identity <remote> --set` defaults to the active members; the root needs `--member @root` or `--all` (`gwz-cli/docs/commands/auth.md`). A user who stores a `.pub` without `--all` leaves the root on the all-keys behaviour. Existing behaviour, read from the docs. | It undercuts the hazard this feature closes | An operator decision on the default, or a hint in the command's output; the help of step 3.3 mentions `--all` | No |
| F6 | A stale agent socket (`ECONNREFUSED`) maps to `Unavailable` with cause `ConnectionRefused`, which `setup_retry::classify` retries (up to the open's attempts, about 7.75 s of waits) like a refused server connection (`ssh_setup.rs:595-625`, `setup_retry.rs:63-77`). By reading; not run. | A dead agent is retried before it is reported, for the ambient step as well | Classify an agent-socket refusal as final; not needed for D38 | No |

## 11. Revision 1: the findings and their closures

Both reviews are in `GwzExactAgentIdentityDesign-ReviewConsistency.md` (C) and `-ReviewSafety.md` (S);
the plan is `-RemPlan.md`. Line numbers are of this file.

| Finding | What it said | Closed in |
| --- | --- | --- |
| C P2-1 / S P3-2 | K1's remedy is wrong for Pageant | §3.3 K1, three remedies (line 285); §3.3 table row 5 (line 268); §4.2 evidence row (line 413); row X9e (line 517); step 1.3 (line 607); step 2.4 (line 612) |
| C P2-2 | Unix no-agent gets the bare Unavailable text | §1 facts row (line 52); §3.2 rule 5 (line 222); §3.3 table rows 2 and 3 (line 265); K0 (line 278); rows X5u, X5w (line 508); F6 (line 700) |
| C P2-3 / S P2-4 | Rule C near-misses (RFC 4716, .ppk, BOM, UTF-16LE, options) and the PowerShell remedy | §2.2 steps 2 to 6 (line 112); §3.1 measured behaviour (line 191); K4 (shell-safe lines) (line 310); §6.2 parser rows (line 533); step 1.1 (line 605) |
| S P2-1 / C P3-4 | The capability is not tied to the compiled route; the doc comment and OQ4 | §1 route row (line 55); §5 capability (line 439); row Capability (line 529); OQ4 (line 666); step 5.1 (line 618) |
| S P2-2 / C P3-1 | METHOD_NONE is worded K2 | §3.2 rule 6 (line 227); §3.3 table row 7 (line 270); §4.1 agent step (line 390); row X8n (line 516); step 1.2 (line 606) |
| S P2-3 | The oracle cannot tell a certificate from its key; no both-loaded row | §3.2 rule 1 (line 207); §3.2 rule 8 (line 236); §3.5 (line 348); §6 oracle table (line 471); rows C1 to C4 (line 511); §6.2 matching and admission rows (line 541) |
| C P3-2 | K5's native advice is not source-aware; T6, T6b and the cap text | §2.3 transport bullet (line 148); §3.3 table rows 4 and 8 (line 267); K5 (line 316); row Native route (line 526); steps 2.3 and 3.1 (line 613) |
| C P3-3 | K6 names only --identity; the no-.pub branch; one owner | K6 (line 325); step 3.2 (sole owner) (line 614) |
| C P3-5 | Step graph, ownership, Pageant rows, Windows control, Surface review, estimate | §0 estimate (line 30); §6 oracle controls (line 493); §7 dependency edges (line 583); steps 2.0 and 2.4 (line 608); reviews placed (line 626) |
| C P3-6 | Unlisted user docs and the private_key_path name | §2.3 configuration bullet (line 143); step 3.3 file list (line 615) |
| C P3-7 | Drift and citations | §2.3 HTTPS bullet (line 154); §2.1 option C (line 84); §1 citations (client.py:371, :75 in F2) (line 54); F2 (line 696); OQ6 citations (line 676); row Reuse isolation (keys A and B) (line 522) |
| S P3-1 | K3 after a refused URL password | §3.3 'Fingerprint' bit (line 258); row UP (line 520) |
| S P3-3 | K5's classifier, read discipline and wording | §2.2 intro (one function) (line 104); K5 (line 316); step 3.1 (line 613) |
| S P3-4 | CR/LF line rule and control characters | §2.2 step 1 (line 107); §2.2 step 6 (line 124); §6.2 parser rows (line 533) |
| S P3-5 | Blob structure not validated | §2.2 step 6 (KeyType::key) (line 127); §6.2 parser rows (line 533) |
| S P3-6 | The no-fallback row can pass vacuously | row NF1 (line 523); row NF2 (line 524) |
| S P3-7 | Step 2.1's intermediate state | §4.1 admission invariant (line 386); row Intermediate state (line 527) |
| S P3-8 | gwz-py's advisory check precedes the flip | §2.3 gwz-py bullet (line 156); step 5.1 (line 618) |
| S P3-9 | A URL password is still offered first | §2.3 URL password bullet (line 163); §2.4 long help (line 175); step 3.3 (Troubleshooting entry) (line 615) |
| Surface note (root) | The root is not covered by `auth identity --set` without `--all` | F5, recorded as a separate operator finding, no change to this design (line 699) |

Experiments for this revision, all deleted afterwards: a local sshd (OpenSSH_10.3p1) with a private
agent on macOS, to read how it logs a plain key and a certificate (section 6, the oracle table); and
one dabeest booking (the lock `E:/gwz-tests/BUSY.lock`, the new directory
`E:/gwz-tests/exactagent-ps-20261018`, released and removed at the end) that wrote a public-key file by
each redirect form with Windows PowerShell 5.1 and read its bytes (section 3.1). Two `ssh-keygen -l -f`
runs on UTF-16 files hung and were stopped by exact PID (74924 and 43520, with their parent
`powershell.exe` 76132); no other process was touched.

## Review request

Code and Safety, then Surface and Consistency after Phase C, as section 7 places them. Please check:
(1) rules 2, 3 and 6 against the code path, in particular that no path from an exact open reaches `Any`
or the helper branch, and `METHOD_NONE` is K3; (2) the oracle on (type, fingerprint) and its controls,
which is what makes "never offered" testable, including certificates; (3) the claim that no wire or
public-protocol change is needed (section 5), and the capability's tie to the compiled route; (4) rule
C's classification and every near-miss; (5) the selection table, K0 to K6 and the help text, as new
user-facing wording and one revised accepted text (D17); (6) the Pageant and pipe precedence rows X9e
and B06e; (7) requirement provenance: every constraint above traces to D30, D38, the 1.0.17 behaviour
or the existing source-precedence rows, and the optional items (options C, D and E, the fingerprint in
the observation) are marked as choices.
