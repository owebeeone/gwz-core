# Consistency + Surface review: exact SSH agent-key selection design (DRAFT)

- **Object:** `gwz-core/dev-docs/GwzExactAgentIdentityDesign-DRAFT.md` at gwz-core `de8e4788`, lane `/Volumes/projects/limbo/gwz-dev-exactagent`.
- **Date:** 2026-10-18
- **Axis:** Consistency, with the Surface concerns folded in. Read-only.
- **Verdict:** **NO-GO.** P0: 0, P1: 0, P2: 3, P3: 7.

I checked the commit at the start and the end: HEAD is `de8e478848298f5ef257aa799a7afa657729374a` both times. One untracked file was already there (`dev-docs/GwzRemoteTransportBugReport.md`). I modified nothing and built nothing, so no `CARGO_TARGET_DIR` was created.

## §0 Evidence base

**Controlling texts read**
- D17, D30 and D38 in `gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md` (lines 60, 73, 81).
- `GwzV110Plan.md` S5.6 and S7.2 (lines 264–275, 317–323).
- `GwzV110RemainingPlan.md` C5 (line 85).
- `GwzTransportWindowsParityDesign.md` §5 (line 112) and the §14 T-texts (lines 395–475).
- `dev-docs/GwzDebtRecoveryAuthCapability.md` (the estimate, lines 87–95).
- Evidence README for run `2026-10-17-pageant-operator` (B06 and X9, lines 27–28).

**Design claims checked in code**

| Design claim | Verified at | Result |
| --- | --- | --- |
| `--identity` help text | `gwz-cli/src/globalargs/parser.rs:52`; the same text in `gwz-py/src/gwz/cli_shared.py:309` | ✓ |
| `--identity` reaches users | Root help is replaced by `ROOT_HELP` (`help.rs:7–37`), which does not list `--identity`. `gwz help CMD` renders long help (`help.rs:72`). | Note only |
| `auth identity` help reads "a remote's local key path" | `parser.rs:386` | ✓ |
| `--set` checks only that the file is available | `handle_remote_identity.rs:36–40` → `validate_file` (`transport_support/identity.rs:265–268`, which opens the file and reads nothing) | ✓ |
| Precedence: remote override, then default, then stored setting | `identity.rs:171–195` | ✓ |
| Wire `Identity` shape | `gwz-transport/src/protocol.rs:1445` | ✓ |
| The endpoint resolves `~/` and relative paths | `placement_endpoint/admission.rs:578–600` | ✓ |
| Key read and checks | `ssh_key_snapshot.rs:171–193`: read, UTF-8, `ssh_key_container::check`. Interning is per (pool key, text) (`:128–131`), and the opaque token is `selected-N` (`:142`). | ✓ |
| Agent step lists once, loops, skips unsupported types | `agent_auth.rs:90` (list), `:93` (loop), `:96` (type skip) | ✓ |
| The step's outcomes | `:149–160`: AUTHENTICATION_FAILED, a refused sign, or METHOD_NONE move on to the next key. The final result is `PermissionDenied`. | ✓ |
| Routing | `ssh_local.rs:145` (selected-key branch returns); `:184–187` (only Pageant is marked early); `:98–103` (`rejected` is set only on AUTHENTICATION_FAILED); `:238` (no agent → `NotFound`) | ✓ |
| Agent sources | `agent_source.rs:70–77` (`None` → `NotFound`, `Refused` → `AgentFault`). Unix `open_path` can return an `AgentFault` only from the unreachable `NoChannel` arms (`:101`, `:106`). `words.rs:12`: `None` and `Path` have no words of their own. | Contradicts rule 5 on Unix |
| Host-side words | `driver.rs:265` (`agent_reason`), `:333–382` (`open_failure_io`), `:157` (generic `ssh setup failed: {code}`). Pageant texts: `pageant_words.rs:51–53` (`NO_KEY_WORDS`), `:12–33` (T6, T6b and the cap text all say "use --transport native"). | ✓ |
| Error mapping | `ssh_setup.rs:601–627`: `NotFound` → `Unavailable` | ✓ |
| `SshLimit` | `ssh_limits.rs:63–74` (`of_failure`: `SshAgent` → `None`); `:131–152` (the D17 words); `:118–122` (`KeyFile` words); HostKeys uses PowerShell `Add-Content -Encoding ascii` (`:108–113`) | ✓ |
| Admission marks limits | `ssh_admission.rs:181–193` marks every limit `SshKey`, so K4 needs a change here, and step 2.1 lists the file | ✓ |
| Native route | `transport_support.rs:236–250` (`Cred::ssh_key`, text "is unavailable" at :248) | ✓ |
| Capability answers | `protocol/transport_capabilities.rs:15` (false; doc comment at :4 says "of the native backend"); `cli_placement.rs:129` (`!cfg!(windows)`, gated `gwz_session_candidate` at :7, so F1 holds). The gwz-py check is at `client.py:371–395`, not :364 as cited. | ✓ apart from the line |
| Who calls the capability | gwz-py dispatch calls the static handler in-process (`native/src/dispatch/read.rs:24–36`). The route resolves the native switch from the process environment snapshot (`route/transport.rs:64–67`, `:122`). | ✓ |
| Facts and observations | `Facts.key_fingerprint` (`protocol.rs:1650`) is never set by anything. The observation copies it (`gitbackend/transport_observations.rs:128`), and the CLI prints `fingerprint=` (`gwz-cli/src/response_meta_json.rs:67`). | ✓ |
| Reuse rule | `gwz-transport/src/codec/validate.rs:207–211` | ✓ |
| No wire change | Nothing in gwz-transport ties `IdentityMode::ExplicitKey` or `AuthPolicy::SshExplicit` to `Facts.method`. `policy.rs:4–25` checks only (scheme, policy, mode). The v2 `key_fingerprint` is accepted outside SSPI facts (`validate.rs:250`). The gwz protocol (`protocol/gwz.taut.py:1065–1091`) carries no doc strings that would need to change. | **Holds** |
| No ssh-config reading | No hits for `IdentitiesOnly`, `IdentityFile` or `ssh/config` in `gwz-core/src` | ✓ |
| Fixtures and runners named in §6 | `login_fixture` alias (`ssh_tests/mod.rs:28–33`), `SshdFixture::new_debug` (VERBOSE, `ssh_fixture.rs:63–77`), `agent_fixture_pipe`, an in-crate synthetic Pageant (`pageant_exchange/receiver.rs`, `ssh_tests/pageant_endpoint.rs`), `windows_parity/*.json`, `key_agent.py`, `windows_lane_check.py`, `rows_ssh.py`, `run-pageant-rows.ps1`, `verify_archive.py` | All exist |
| Docs that say the opposite today | gwz-cli `docs/commands/auth.md:37–39`; `Releases.md:156`, `:408`; `Troubleshooting.md:297–339` (the D17 entry) | Not enumerated by the design |

## Findings

### P2-1: K1's Pageant remedy sends the user to the wrong agent

- **Location:** §3.3 K1 (line 190); line 227–229 (K1 replaces `NO_KEY_WORDS`); §6.1 X9e (line 367).
- **Root cause:** K1's next action is written for the OpenSSH agent: `ssh-add …`, `ssh-add -X`. Pageant gets only one extra sentence, and that sentence gives no remedy.
- **Violated:** the parity design's T-text rule (the next action must work). Also D38, which names Pageant as one of the platforms.
- **Code today:** `NO_KEY_WORDS` (`pageant_words.rs:51–53`) and `pageant_selected` (`words.rs:50–59`) both tell the user to load the key into Pageant or close Pageant.
- **Consequence:** in the X9e setup (Pageant running without the key, the OpenSSH agent holding it), `ssh-add` on Windows writes to the OpenSSH agent. gwz never asks that agent while Pageant runs (§4.2), so the next run fails with K1 again. `ssh-add -X` means nothing to Pageant.
- **Correction:**
  - Give each source its own remedy line, and pin each one in step 1.3.
  - Pageant: "Load the key into Pageant (Add Key), or close Pageant so that GWZ uses the OpenSSH agent."
  - OpenSSH pipe: name the service as `STOPPED_AGENT` does.
  - Unix: `ssh-add`.
  - Drop the lock hint for Pageant.

### P2-2: on Unix, a missing or dead agent on the exact route gets the bare text `ssh setup failed: Unavailable`

- **Location:** §3.2 rule 5 (lines 166–168), §6.1 X5 (line 363), §4.1 Words (lines 285–289).
- **Root cause:** the design assumes the "existing words (`AgentFault`)" cover an unusable agent on every platform. On Unix:
  - `AgentSource::None` gives `NotFound`, and a dead socket gives a connect error. Neither is an `AgentFault` (`agent_source.rs:72`, `:95–97`).
  - `unavailable_reason` returns `None` for `None` and `Path` (`words.rs:12`), so the open falls through to `driver.rs:157`.
  - §4.1's K1 rule needs `Authentication`, but these cases arrive as `Unavailable`.
- **Violated:** the design's own text style (cause, what gwz did not do, next action). Also the claim about today's code.
- **Consequence:** the most common first-day Unix failure, a shell with no `SSH_AUTH_SOCK` (sudo, cron, an IDE terminal, tmux after re-attach), tells the user nothing on a route they chose explicitly. Row X5 cannot pass as written on macOS or Linux.
- **Correction:**
  - Add a text, say K0, for an exact open whose agent cannot be reached: no agent configured, or nothing listening at `SSH_AUTH_SOCK`. It should name the key file, say that nothing was offered, and say to start an agent and run `ssh-add`.
  - Select it on (`Unavailable`, method `SshAgent`, nothing offered, key file set) with a `None` or `Path` source.
  - Split X5 by platform.

### P2-3: rule C's near-miss detection is too narrow, and its own remedy fails in Windows PowerShell

- **Location:** §2.2 steps 3–4 (lines 97–99), §3.1 (lines 142–146), K4 (line 210), K6 (lines 220–221), §2.1 (line 84).
- **Root cause:** K4 fires only when the first word is a key-type name. Every other near-miss falls to step 4, "refused as today", which is the generic `ssh setup failed: InvalidRequest` (`ssh_admission.rs:191` returns the kind alone; `driver.rs:157`). The near-misses that fall through:
  - the RFC 4716 file PuTTYgen saves by default (`---- BEGIN SSH2 PUBLIC KEY ----`);
  - a `.ppk`;
  - a UTF-8 BOM;
  - a UTF-16LE file, which `read_with` refuses as non-UTF-8 (`ssh_key_snapshot.rs:185`) before rule C runs;
  - an `authorized_keys` line with options (which §3.1 lists among the refusals, implying K4).
- **Violated:**
  - D38's Pageant and Windows parity.
  - The existing Windows convention, where the HostKeys words give a PowerShell form with `-Encoding ascii` (`ssh_limits.rs:108–113`).
  - §3.1 against §2.2: the two sections disagree.
- **Consequence:**
  - A Pageant user's natural `.pub` from PuTTYgen gets an unexplained refusal.
  - K4's and K6's own fix, `ssh-keygen -y -f KEY > KEY.pub`, writes UTF-16LE in Windows PowerShell 5.1. The file it produces is then refused with the generic text.
- **Correction:**
  - Either accept RFC 4716, a BOM and UTF-16LE (all bounded), or classify them, plus `.ppk` and an options-prefixed line, into K4 with words that name the fix.
  - Give the shell-specific form: `ssh-keygen -y -f KEY | Set-Content -Encoding ascii KEY.pub` in PowerShell, and `>` in Git Bash.
  - Add each case as a unit row in §6.2.

### P3-1: METHOD_NONE gets K3 in rule 6 but K2 under §4.1's selection rule

- **Location:** §3.2 rule 6 (lines 172–173) against §4.1 (lines 278–279 and 288–289).
- **Root cause:** METHOD_NONE calls `offered()` but never `rejected()` (`agent_auth.rs:114`, `:151`; `ssh_local.rs:98–103`), so `authenticated` stays `None`. Under "offer and `authenticated` not false → K2", the result is K2.
- **Consequence:** for an RSA key the server accepts no algorithm for, the user is told a security key may be unplugged.
- **Correction:** say the exact step marks METHOD_NONE as rejected, or give K3 its own selection condition. Pin the case in X8.

### P3-2: the native-switch advice is not reconciled with exact selection

- **Location:** K5 (line 216); line 226 ("T6, T6b … unchanged").
- **Root cause:** K5 hard-codes `--transport native` and "unset GWZ_TRANSPORT or gwz.transport". It does not use the source-aware remedies that already exist (`gwz-cli/src/globalargs/transport.rs:52–63`; `gwz-py/native/src/route/transport.rs:103–117`) or the T11 flag-or-environment hint.
  - gwz-py has no `--transport` flag (`Source::Flag` is `unreachable!`, `transport.rs:107`).
  - K5 does not say where the `.pub` came from. A stored setting needs `gwz auth identity <remote> --unset`.
  - T6, T6b and the cap text (`pageant_words.rs:12–33`) still say "or use --transport native". On the exact route, that leads straight to K5.
- **Correction:**
  - Build K5's remedy from the transport source and the selection source, both of which the host holds (`SelectedIdentity.source`).
  - Drop the native clause from T6, T6b and the cap text when the open selected a public key.

### P3-3: K6 names only `--identity`, and its "no `.pub`" branch is unspecified

- **Location:** K6 (lines 219–224).
- **Root cause:**
  - If the encrypted key came from `--remote-identity`, adding `--identity <file>.pub` does nothing, because the remote override wins (`identity.rs:171–195`). The current D17 text names all three sources (`ssh_limits.rs:146–147`).
  - "With the `ssh-keygen -y` line when no `.pub` exists" needs the host to probe a path the endpoint resolves (`~/` and the base, at `admission.rs:578`). In the Cli placement that is another process.
  - "Can only come from an agent" is quoted from `encrypted_key_words`, not from `KeyFile` (`ssh_limits.rs:118–122`).
- **Correction:** "name `<file>.pub` wherever you named `<file>` (`--identity`, `--remote-identity`, or `gwz auth identity <remote> --set`)". Always print the `ssh-keygen -y` line, in shell-safe form (see P2-3). Fix the attribution.

### P3-4: the capability's meaning disagrees with the handler's doc comment, and OQ4 rests on a premise the code does not support

- **Location:** §5 (lines 323–335); OQ4 (lines 468–470); step 5.1.
- **Root cause:**
  - The static handler is documented as "Capabilities of the native backend shipped with this core" (`transport_capabilities.rs:4`). The native backend refuses a `.pub` (K5), so flipping the answer to true makes that comment false. Step 5.1 does not mention the comment.
  - OQ4 says reflecting the process's transport setting "needs a protocol change". It does not: the gwz-py capability call runs in-process (`dispatch/read.rs:24–36`), and the same `transport_setting::resolve` over the environment snapshot is available there (`route/transport.rs:64–67`).
- **Correction:** update the doc comment in 5.1, and restate OQ4 with the in-process option and its cost (the environment can change between the probe and the operation).

### P3-5: the plan's step graph is under-specified, and the estimate is not compared like for like

- **Location:** §7 (lines 412–441); §6 (line 351); §6.3 (lines 400–402).
- **Dependencies and ownership:**
  - K6 is owned by both 1.3 and 3.2. Both edit the words in `ssh_limits.rs`, so two agents would collide.
  - 2.3 needs 2.1 (the `SshLimit` variant and its `of_failure` arm) and 2.2 (the early `SshAgent` mark). Only 1.3 is named.
  - 5.1 names 4.2 but not 4.1, although the flip covers macOS and Linux too.
  - The rows "Pageant exact", X9e and B06e have no unattended owning step. 2.0 covers the Unix proxy agent and the Windows pipe agent only. 4.2's runner-level Pageant needs a desktop session (§6.3). An in-crate synthetic Pageant already exists (`pageant_exchange/receiver.rs`, `ssh_tests/pageant_endpoint.rs`) and is not used.
  - The oracle's negative control is proven on Windows only in 4.2, yet the Windows twin rows run in every Phase B receipt (line 438).
- **Review timing:** "Surface on 3.1 and 3.2 (folded into the same review)" cannot be folded into a review held after Phase B, before Phase C exists.
- **Estimate:** the old 5–10 days excluded Windows ("needs its own investigation", `GwzDebtRecoveryAuthCapability.md:91–93`). The new 4–7 days includes three agent sources and an operator session, so it is not a like-for-like reduction. A lower bound of 4 days looks optimistic for 12 code and evidence steps that each need a dabeest receipt, plus two reviews.
- **Correction:**
  - Give K6 a single owner.
  - Add the missing edges.
  - Add an in-crate synthetic-Pageant step for X9e, B06e and the Pageant exact row.
  - Prove the Windows control in Phase B.
  - Place the Surface review explicitly.
  - Restate the estimate's basis.

### P3-6: user docs that contradict the feature, and a misleading field name, are left unlisted

- **Location:** §2.3 (lines 113–117); step 3.2.
- **Root cause:** step 3.2 does not enumerate:
  - `gwz-cli/docs/commands/auth.md:37–39` ("Exact selection of an encrypted key unlocked in an SSH agent remains unsupported");
  - `Releases.md:156` and `:408`;
  - the D17 Troubleshooting entry;
  - `gwz-core/docs/MessageCatalog.md`.
  The JSON key `private_key_path` (output of `auth identity --json`) and the Python `RemoteSshIdentity(private_key_path=…)` will hold `.pub` paths. The design says "the docs say" so, but names no doc.
- **Correction:** list these files in 3.2. Add a sentence to `auth.md` and `MachineOutput.md` and a gwz-py docstring saying the field may hold a public-key path.

### P3-7: drift between claims and code or texts

- **"HTTPS remotes ignore the identity" (line 120):** true of `--identity` only. `--remote-identity` naming a remote reached over HTTPS is refused (gwz-cli `Troubleshooting.md:291–295`).
- **F2 against §4.1:** F2 says K4 "keeps `file`". But §4.1 marks K4's facts `SshAgent`, and `facts()` maps that to `agent` (`transport_observations.rs:121–124`) once the facts arrive.
- **Option C's rationale (line 76):** `SHA256:x` is not drive-relative, because a drive is one letter. The real ambiguity is an NTFS alternate data stream.
- **Citations:**
  - The "N2 design" fingerprint rule is in `GwzRemoteTransportSshSelectedIdentityDesign.md:47` and `GwzRemoteTransportSshProductionSetup.md:102`.
  - `client.py:364` should be `:371`.
  - `transport_observations.rs:68` should be about `:75`.
- **Naming clash:** the "Reuse isolation" row says "exact K1 open … exact K2 open" for keys, which collides with refusal names K1 and K2.
- **Correction:** fix each in the text.

## Attacks that failed

- **"No wire change" is false.** It held: no gwz-transport validation ties `ExplicitKey` or `SshExplicit` to `Facts.method`. `key_fingerprint` is allowed for non-SSPI v2 facts. The gwz schema is untouched. The reuse rule is cited correctly. An older endpoint given a `.pub` fails closed at the container check.
- **The `check_identity` pre-check refuses a `.pub` before any open.** It does not: it only opens the file (`ssh_worker/endpoint.rs:147–152`).
- **Words never reach the user for authentication-class failures.** They do: `AgentFailure` with the Authentication class sources `AuthenticationRejected`, and `ssh_remote.rs:77–84` carries `error.to_string()` into the git2 error.
- **Pool isolation leaks across key kinds.** It does not: entries intern per (pool key, text or blob) and get distinct `selected-N` tokens (`ssh_key_snapshot.rs:128–143`).
- **The precedence claim is wrong.** It matches `identity.rs:171–195`.
- **The D30 row cannot be expressed.** X1 matches D30's wording exactly.
- **S7.2 is violated.** The flip is sequenced after evidence, and 5.2 updates the S5.6 table.

## Surface notes

**Placement and name.** Reusing `--identity` is honest ("this key, nothing else") and is the most discoverable route for the D17 user, since K6 points there.

**Content sniffing against OpenSSH `-i`.** Classifying by content matches OpenSSH. The design's precedent is accurate for `-i key.pub`. But OpenSSH's more common idiom, `IdentityFile id_x` (the private file) with `IdentitiesOnly yes` and the key loaded in the agent, uses the agent's copy without a prompt. gwz instead refuses the encrypted file (D17). For an OpenSSH-format encrypted key, the public blob is stored in clear in the file's header, so `--identity id_x` could select the agent key with no `.pub` at all. The design rejects deriving paths but never weighs this option. It is not a defect, but it belongs in OQ1 or OQ7.

**Proposed help text, read cold:**

- `--identity PATH`: "Use only this SSH key and no other: a private-key file without a passphrase, or a public-key file (for example `~/.ssh/id_work.pub`) to use that key from your SSH agent. gwz's own transport only."
- Long help: whether the file holds a private or a public key is decided by its content, not its name; the agent must already hold the key (`ssh-add`); on Windows the agent is Pageant if a Pageant window is running, else the OpenSSH agent; `--transport native` refuses a public-key file.

**Scripted first-day walkthrough** (macOS):

1. `ssh-add ~/.ssh/id_work`, then `ssh-add -l`.
2. `gwz fetch --identity ~/.ssh/id_work.pub`. Success shows `credential=agent source=invocation_default … fingerprint=SHA256:…`.
3. `gwz auth identity origin --set ~/.ssh/id_work.pub`. By default this covers active members only, not the root (`auth.md`). The root stays on all-keys unless `--all` is given. That behaviour already exists and is out of scope, but it undercuts the hazard this feature closes, so it should go to the operator as a separate finding.
4. `gwz fetch`, which is now exact for each member.
5. Undo with `gwz auth identity origin --all --unset`.

Failure points along the way:

- No agent in the shell: P2-2.
- Windows with Pageant: P2-1.
- A PuTTYgen-saved file: P2-3.
- `GWZ_TRANSPORT=native` set later: K5, which does not say the setting came from step 3 (P3-2).

**Defaults.** The ambient default is unchanged, so no option is left without a default.

**gwz-py.** Same strings as the CLI. The `private_key_path` name is P3-6. The advisory `.pub` suffix check cannot see stored settings. That is acceptable for advice, but it should be documented as such.

## View on OQ1–OQ9 (which answer the evidence favours, not a decision)

- **OQ1:** A. The no-wire-change claim held under attack, and the field naming costs only P3. Also consider the embedded-public-key option above.
- **OQ2:** content only. It matches OpenSSH, and a suffix rule would not fix the RFC 4716 case. Widen K4 instead (P2-3).
- **OQ3:** defer the fingerprint selector. Nothing in D38 or D30 needs it, and the colon in a Windows path makes it ambiguous.
- **OQ4:** keep the default-route meaning and fix the doc comment. Correct the premise: an in-process probe of the transport setting is possible without a protocol change, but it can race the operation (P3-4).
- **OQ5:** in-crate synthetic-Pageant rows (the receiver already exists) plus CLI rows on the pipe agent. D30 asks for one Windows exact row from any agent, and real-Pageant list and sign was already proven ambient on 2026-10-17. An operator session is optional.
- **OQ6:** set the fingerprint at the offer. The blob is byte-equal to the agent's listed key, which is proven public data under `auth.md`'s "known from the actual credential", and it keeps K3 useful.
- **OQ7:** revise, but with source-aware and shell-safe words (P3-3, P2-3).
- **OQ8:** exact blob equality. D30 says "the other is never offered", and offering a matching certificate would offer more than was selected.
- **OQ9:** dropping the git2 custom-credential work is consistent with git2-rs staying reference-only. The new estimate covers Windows, which the old one excluded, so it is not like for like, and its lower bound looks optimistic (P3-5).

## Next action

Remediate P2-1 to P2-3 in the design:
- source-specific K1 remedies;
- a K0 text for an unreachable agent on Unix;
- a wider rule C near-miss set and shell-safe `ssh-keygen -y` words.

Fold in P3-1 to P3-7, then send it to a re-review on this axis. No product code should start until the P2s are closed.
