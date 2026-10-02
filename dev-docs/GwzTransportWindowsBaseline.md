# Windows S4.1 and TR1.8 baseline

Date: 2026-10-03. Status: **S4.1 native prerequisite proof executed; partial released-1.0.17
authentication/primitive baseline retained. TR1.8 is not ready to freeze.**

## 1. Exact local source and preservation

| Repository | HEAD |
|---|---|
| root | `12f11c7949919834fe8858247dc4c0cd49a8134b` |
| gwz-core | `2e64e88a28c332ed422cc390adc76738dc701bb1` |
| gwz-cli | `0164e66376dac204910552148c62cb6c5c55ed03` |
| gwz-py | `b2369f1d0bf72f7fbbd5949d92c4c75a6fbc24b5` |
| gwz-transport | `35475977530171ab77ee2fbb1e8128f938acb5ae` |
| git2-rs | `d13951f7e0bfb6e0efcee1207ac5b140adefa455` |
| libgit2 | `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` |
| gwz-git | `a9d7ee09cce6dd1407be99d8ec3676a1d288b4bf` |
| gwz-core-evidence | `90740fe07b12a5300967e66b9efd7521960aa573` |

Observed via read-only `git rev-parse HEAD` in the lane. Initial root/core
tracked status was clean. Inherited root SSH N2b prompt drafts and
GwzWorkspaceRouteMappingDesign, plus core GwzRemoteTransportBugReport, are
out of scope and preserved. The latter has private host/account material and
must not be committed publicly as inherited. Current design files add new
untracked docs only; no production file or platform guard changed.

The root owner's prewarm receipt records native COW cloning of all 13 repos
and four representative artifact hashes in
`/Volumes/projects/limbo/gwz-lanes-prewarm-20261003/lane-build-receipts.json`.
This is a local cache receipt, not Windows execution. Preserve the caches.
The examples in that directory point to main and must not test this lane.
This lane regenerated candidate preparation through
`python3 gwz-core/tests/transport_backend/prepare.py
/Volumes/projects/limbo/gwz-tr1-8-win-candidate-20261003`.
Its manifest anchors dependencies and source links to this lane. Generated
candidate protocol SHA-256 is
`c9b99d2003c6b28f07a429968a8026d078dbbfb164e13f21a21645920a4e2ca5`.
Future local checks use Rust 1.95.0, normal profiles and the retained lane
cache; preparation alone is not a candidate build or test result.

## 2. Remote observations and authorization boundary

The new private run is
[2026-10-03-tr1-8-windows](../../gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-03-tr1-8-windows/README.md)
(private access required). Root authorized one unique scratch directory under
E:, pinned toolchain checks, disposable loopback identities/fixtures and isolated
primitive spikes. Runtime: `E:/gwz-tests/tr1-8-20261003-61-a50c2d`.
Compiled outputs, downloaded tools, keys/certificates and Python dependencies
remain outside the archive. No live-account, hosts/zone, existing-agent-key or
account-creation rows are authorized. The Windows product guard is unchanged.

`s41-provision` exited 0: actual create/write/read/remove passed, ancestors had
neither Cargo config name, Rust/Cargo 1.95 were explicitly resolved, native host
was x86_64-pc-windows-msvc. `pinned-native-build` compiled and executed the owned
Rust probe from the external E: target; runtime ancestors were rechecked. E: is
Healthy ReFS. This closes the physical S4.1 prerequisite; root settlement and
independent design reviews remain separate. Read-only `test -w` and inherited
inventory are not substituted for this executed proof.

Released Windows 1.0.17 archive SHA-256:
`bb9e37202832cdc909dcc29014c198632801bc77b9d1f93db8c4628834e10b2e`;
executable `cbab5e9b0aa964d59891c3b8d02bb4042218715cda3902f6de52589718ca12f8`.
PuTTY 0.83 Pageant was verified against its official SHA-256 manifest and started
only after finding no existing Pageant in the caller's window station. Each
window PID was checked against the owned process, and each owned Pageant was
reaped. Caller session 0 differs from console session 1; no claim that another
session's agent inventory was examined or changed. Native OpenSSH pipe was
absent (error 2); a provider inventory is not its authentication proof.

Read-only SSH uses `ClearAllForwardings=yes`; no forwarded operator agent
serves a disposable authentication fixture. Never retain raw keys, certificates,
Authorization/Proxy-Authorization, SSPI tokens, passwords or operator identity.
Only disposable fixture certificate hashes identify the proposed trust entry.

Root separately serialized bounded machine-proxy transactions. The exact prior
state was access type 1, empty proxy and bypass (DIRECT). An independent deadline
restore guard was armed before each set. Only an owned loopback CONNECT proxy
was selected; every attempt restored and verified exact DIRECT in parent finally
and guard, with owned guard exit observed. First attempt failed on a PowerShell
reserved HOME variable; its failure and successful restoration are retained.
Native user trust imports/search-list changes remain **unapproved and unexecuted**.
Process CA alternatives failed. Mac-only concrete proposal is private
`MACOS_TRUST_APPROVAL_v1.md`; root requested operator approval, with no answer at
this checkpoint. Exact prepared owner v6 SHA-256
`4823a0d0331e6cc68b5b074d4b8a55f4b68dad3968cad1b8cfa6018e61b7b919`;
process ownership helper v3
`e977fb42dc7015d62ab1725a0a52528bb860d3ab0107be5a43faa62920604492`;
bounded TLS row v1
`17368be04f2ad9093733cc55170c68cb5030d76a84af07b8027a06a9f801d982`.
One generated fixture DER/PEM is pinned; native calls use the same DER, validity
checked immediately before arming. Native DER cleanup permits expiry. Drift and
expired-input refusal passed using scratch copies, with no trust calls. Approval
would cover one new user keychain/owned trust entry/search-list interval, maximum
45-second mutation permission plus bounded confirmed cleanup. Required future
receipt must record negative preflight, exact source/certificate hashes, HTTP/TLS
outcome, owned trust/keychain absence, exact prior search-list equality, revoked
mutation rights, pending owned process retirement and observed guard exit. No
trust-pass claim exists before execution and restoration are observed.

Windows trust is **held separately**. Actual caller/guard, explicit breakaway and
one local CIM launch all report membership in some job; nested ancestor policy is
unknown. CIM exact process creation identity and same caller SID were verified,
owned process reaped. Guard v7 refuses any-job membership before arming; the
expected refusal executed. No job/service/credential policy was changed. One
scratch-only original launch-shape test killed only its dedicated local SSH
connection, reconnected and verified lease expiry, pinned owner/worker/guard
retirement, exact mock cleanup and no late write. That is observed survival for
this exact launch shape, **not universal job independence**, and does not override
v7 or authorize Windows trust. Root must review any equivalent supervisor proof
before a Windows transaction can proceed.

V110 §2 explicitly controls `/e/gwz-tests/<unique-name>` and external E: target
directories, superseding EVIDENCE/REPLAY's stale D: fixture location for this
transport campaign. Historical D:/E: evidence is untouched. Ancestor Cargo
configuration or unreadability refuses a run; check both config and config.toml
and runtime-specific ancestors immediately before build. Pinned toolchain
checks use explicit `+1.95.0-x86_64-pc-windows-msvc`, not the default compiler.

## 3. Released baseline — executed facts and remaining rows

All rows below use **released 1.0.17**, exact downloaded archive/binary hash,
source tags and the pinned Rust/toolchain provenance where built. A current
1.0.17 version string in a main candidate binary is not released 1.0.17 proof.

| ID | Status | Row | Observed outcome or required assertion |
|---|---|---|---|
| B01 | EXECUTED | S4.1 Bash/toolchain | PASS: explicit Rust/Cargo 1.95 and native pinned probe compiled/executed |
| B02 | EXECUTED | S4.1 storage/config | PASS: E: write/read/remove, both Cargo config names absent through runtime ancestors; external native output |
| B03 | EXECUTED (authentication) | HOME unset | PASS authentication: HOMEDRIVE+HOMEPATH and separate USERPROFILE-only known_hosts rows; not a complete GWZ clone |
| B04 | UNEXECUTED | Home edge cases | Empty, missing, nonexistent, relative, space and UTF-16 candidates compared with sysdir behavior |
| B05 | EXECUTED (authentication) | Pageant alone | PASS authentication: pinned PuTTY 0.83, verified RSA SHA-256 agent signature, released binary accepted owned key; final WorkspaceNotFound is intentional |
| B06 | UNEXECUTED | Pageant and OpenSSH | Both alive, different disposable keys; Pageant wins exactly |
| B07 | EXECUTED (refusal) | Neither agent | PASS refusal: no owned key offer, authentication false, RemoteRejected no-usable-agent-identity error |
| B08 | PARTIAL (inventory) | Selected pipe | Native pipe absent/error2; actual service identity/authentication and selected pipe rows unexecuted |
| B09 | PARTIAL | Machine proxy | PARTIAL: owned CONNECT route used for reserved .invalid origin despite conflicting HTTPS_PROXY; numeric 127.0.0.1/127.0.0.2 origins bypassed it; grammar/bypass rows remain |
| B10 | EXECUTED (refusal) | Proxy 407 Negotiate | EXECUTED refusal: Negotiate and NTLM407, zero credential offers/context completion, native string-conversion error; no complete proxy authentication support claimed |
| B11 | UNEXECUTED | Negotiate-only | Configured fake gh credential present, helper not called, logon session authenticates |
| B12 | UNEXECUTED | NTLM-only | Helper identity authenticates, then no-helper default identity row |
| B13 | UNEXECUTED | Mixed Negotiate+Basic | Fake gh then non-gh helper authenticate over Negotiate, no default offer |
| B14 | UNEXECUTED | Digest | Helper identity authenticates, method/URI and POST behavior characterized |
| B15 | UNEXECUTED | Zone/redirect | Disposable Intranet and Internet names, discovery first-to-second, auth at second under OD16 |
| B16 | UNEXECUTED | EPA | Server requires channel binding, successful auth; missing/wrong binding negative controls |
| B17 | PARTIAL | macOS/Linux | Mac plain-HTTP Negotiate unsupported characterization; HTTPS blocked by native trust; Linux unexecuted |
| B18 | UNEXECUTED | Windows POST challenge | Anonymous discovery then 401 POST: actual replay/body behavior and effect recorded |

Fixture names proposed: `gwz-tr18` (single-label/Intranet) and
`gwz-tr18.invalid` (dotted/Internet), both loopback. Their actual URL-zone
classification must be measured before use, never inferred from spelling.
Do not edit hosts or zone configuration without coordinated authorization.
Do not use a live account to prove a disposable identity row. An explicit
credential selected from a configured test helper must be distinguishable from
the operator's logon identity by server-side identity class assertions, without
retaining user name/SID/token. If a temporary local account is necessary,
coordinate its creation/removal rather than silently inventing it.

## 4. Mandatory physical primitives — partial proof, no freeze

| ID | Status | Primitive | Observed proof / remaining required counterexamples |
|---|---|---|---|
| P01 | PARTIAL | Pageant mapping | Pinned actual Pageant accepts Local/unique names and explicit caller+SYSTEM ACL, empty list and signature verified; collision/cross-user negatives remain |
| P02 | PARTIAL | Pageant bounded send | Synthetic receiver: sender50ms timeout/close, receiver retains OS mapping ref and writes late; no timed-out response read. Actual confirmation/hung/vanished/reused HWND remain |
| P03 | UNEXECUTED | Local pipe | No UNC/SMB open, correct native service identity, overlapped partial reply/cancel with outstanding I/O |
| P04 | PARTIAL | WinHTTP capture | Native capture/GlobalFree and exact DIRECT restore executed; grammar, implicit loopback bypass and immutable candidate snapshot still need proof/disposition |
| P05 | PARTIAL | SSPI | Native default NTLM+Negotiate direct handshake complete; synthetic matching CBT succeeds and wrong CBT SEC_E_BAD_BINDINGS. Not HTTP/EPA or explicit/helper isolation proof |
| P06 | UNEXECUTED | TLS/EPA | Actual peer DER/signature algorithm available; RFC5929 digest correct across cert algorithms/TLS versions |
| P07 | UNEXECUTED | Digest | WDigest reproduces WinHTTP HTTP helper row with actual method/URI, complete-token statuses |
| P08 | UNEXECUTED | Helper Job Object | Paths with spaces, direct exe + shell helper, GUI helper, nested job, child kill/reap, partial pipe I/O |

These spikes run outside repositories and build outputs stay outside the
private evidence archive. A spike can establish a primitive before contract
freeze; it cannot remove Windows compile guards or claim product parity.

### Remaining-row authorization and dependencies

Every unresolved row remains a freeze gate; the following classification is a
handoff, not permission to drop a required assertion. Root requested this bounded
partial package now while Mac approval is unanswered. No further experiments are
part of this handoff. Existing-scope rows are unexecuted work, not missing authority.

| Rows | What can run in existing scope | Additional prerequisite / boundary |
|---|---|---|
| B04 | Disposable HOME edge cases with owned Pageant/known_hosts | No OS trust/account changes needed |
| B06/B08 | Own selected-pipe protocol fixtures can characterize selection | Actual native OpenSSH service/both-agent rows require coordinated service/agent setup and disposable keys; absent pipe inventory is not completion |
| B09/P04 | Owned machine-proxy grammar/bypass and capture/snapshot fixtures | Root serializes each proxy interval; exact prior DIRECT/guard/restore required; no generic reset/PAC/user-proxy change |
| B11 | Fake configured-helper invocation counter plus default-logon server fixture | Full released HTTPS row needs approved Windows native trust/supervisor precondition; helper is fake, no live gh |
| B12/B13 | Generated explicit fixture credentials and controlled verifier can be prepared | Full HTTPS needs Windows trust. If OS-backed distinct user is needed, account/token provisioning needs separate root approval; no operator password use |
| B14/P07 | Direct WDigest status/method/URI probe and controlled Digest verifier | Actual HTTPS helper/POST needs Windows trust; package inventory is not availability proof. Provider refusal needs exact evidence/disposition, never enabling weak policy |
| B15 | Read-only actual URL-zone classification and fixture routing can be prepared | Full HTTPS requires trust and named certificate/routing. Try owned proxy routing without hosts edits; hosts/zone edits require root approval if unavoidable |
| B16 | Existing synthetic CBT proof remains partial | Actual TLS/EPA-required positive/negative server rows need Windows trust and verified peer binding; no distinct account necessarily required |
| B17 | Mac row awaits requested approval; Linux fixture source can be prepared | Mac native trust unapproved; Linux target assignment/prerequisites needed, no Linux row attempted here |
| B18 | Owned discovery/POST challenge/replay fixture can be prepared | Full released HTTPS row needs Windows trust; document actual body/effect, not inferred replay |
| P01 | Name collision/ACL probes on owned mappings/Pageant | Real cross-SID actor requires separately coordinated distinct token/account; no real agent changes |
| P02 | Owned Pageant confirmation, synthetic hung/late/vanished HWND and cancellation fixtures | Existing isolated fixture scope; actual receiver ownership/quiescence still must be measured before broker choice freezes |
| P03 | Owned local named-pipe partial/cancel I/O and no-UNC primitives | Native service identity/auth row additionally needs coordinated provider setup; do not substitute a fake server for native service proof |
| P05 | Default/explicit SSPI status, isolation and bounded owned-provider-process fixtures | Current/native provider policy may refuse; record exact result. Distinct OS identity only if needed requires approval; no policy change |
| P06 | Standalone process-scoped TLS peer-DER/signature/hash primitive | Existing unique-CA fixture scope can avoid OS trust; released-native TLS/EPA row still needs approved native trust; unavailable build/provider tooling is reported, not silently installed |
| P08 | Owned helper Job Object, spaces/GUI/nested-job/child/partial-I/O fixtures | Existing isolated fixture scope; changing inherited host job policy/service/elevation remains prohibited |

B01/B02/B03/B05/B07 have executed outcomes described above; B10's measured
407 refusal completes that baseline observation, not proxy-auth support. Remaining
rows with PARTIAL status require the named missing assertions. Passing all rows
and explicit compatibility dispositions plus filed independent reports precede GO.

## 5. Retention and restore contract

The root owner reserved the new private run named above; its archive/member
AGENTS, README and REPLAY were read before writes. Prior runs are untouched.
Campaign runners and raw redacted transcripts belong there; runtime checkouts,
compiled executables, certificates/private keys and target caches do not.
Private access is required for raw evidence. A public design/gate must not
depend on archive access to build or test.

Each executed row records exact command and exit, SHA-256 of runner/inputs/
binary, source tuple, OS/toolchain, fixture address class, result assertions,
cleanup confirmation and limits. Record failures as failures and skipped rows
as unexecuted. Do not add pass-count pins or test-log census release gates.

Machine proxy work is serialized with the root owner. Save the exact prior
proxy and bypass configuration before mutation, arm restoration before setting,
restore on all exits, verify equality after each row. Reset proxy is sufficient
only when prior state was direct. Interrupted restoration blocks further
shared-host mutation and is reported explicitly. Do not change real Pageant's
key inventory or stop an existing agent; fixtures own only their disposable
processes and identities. Never terminate a process by name alone.

## 6. Review readiness

Current outcome: S4.1 physical prerequisite complete; partial native baseline
and primitive proof, provisional design, **no freeze or GO**. Real HTTPS trust
rows, explicit/helper identity isolation, selected native agent, EPA, Digest/POST,
zone/redirect and bounded helper/SSPI cancellation remain unresolved. The parent owns settlement and
review capacity. To become READY TO SETTLE: execute B/P rows in authorized
isolation, resolve provisional clauses in the design, fill exact private
evidence links and concise public results, then send owned paths and hashes
to the root owner. Only its committed tuple is eligible for canonical
Consistency/Safety+Surface review; passing filed reports are required before
downstream implementation.

## 7. Canonical review input for the root owner

Generate prompts from review-loop's `references/review-prompt-template.md`;
these fields are inputs, not permission to review the present dirty draft.

- Object: committed `GwzTransportWindowsParityDesign.md` plus this baseline.
- Controlling graph: ReleasePlan; Amendment-2 revision 6 §§3.5, 3.18, 3.19;
  V110 §2/S4; accepted CredentialHelpersDesign revision 4 and its recorded
  operator answers; OffSwitchDesign; server design's amended §5; retry plan.
- Tuple: root/core/dependency heads supplied by the root owner after settlement,
  and exact committed docs SHA. Never fill this with a document's pre-edit HEAD.
- Out of scope: inherited SSH N2b prompts, route draft, private BugReport,
  other lanes, product Windows implementation and release activation.
- Consistency attack: source-selection precedence, helper scheme precedence,
  HOME semantics, proxy inverse precedence, authenticated redirect policy,
  every compatibility disposition and whether the executed matrix satisfies
  the amendment's evidence-first gate.
- Safety attack: Pageant shared-memory late writes and source spoofing;
  pipe server identity/SMB refusal/overlapped cancellation; machine-proxy restore,
  bypass and 407; OD16 forced authentication without reimposing its lifted
  bound; SSPI secret ownership, channel binding, connection isolation and retry;
  cancellation without freeing live OS storage or admitting unlimited jobs.
- Surface object: committed `GwzTransportWindowsUserGuide-DRAFT.md` only,
  existing auth docs and TR1.5's accepted help text. No code or design/plan.
  Walk through selecting/removing all three settings, unavailable agent,
  proxy/authentication refusal and helper sign-in using those texts alone.
- Reports: `GwzTransportWindowsParity-ReviewConsistency.md`,
  `GwzTransportWindowsParity-ReviewSafety.md`,
  `GwzTransportWindowsParity-ReviewSurface.md`, filed verbatim beside this file.
- Allowed review commands: read-only `git rev-parse HEAD`, `git status --short`,
  `git show <settled-sha>:<named-file>`, targeted `rg` and `sed` inside the
  lane. No remote mutation, builds, secret/token dumping or peer report reads.
- Review starts/ends with tuple verification; any movement stops the verdict.
  Parent owns the committed package, independent reviewer capacity and merge.
