# Windows S4.1 and TR1.8 baseline

2026-10-03 MAIN import: original experimental records and provisional clauses
remain historical/DRAFT. Full Windows NO-GO persists. The separately accepted
[SSPI-only design](../../dev-docs/GwzSspiDesign.md) has its own mechanism review;
it does not accept these whole-platform dispositions, guide or proof rows.
Read SSPI lifetime claims through Windows parity's explicit SSPI-only amendment.
Current MAIN accepted helper timing/configuration/SSH clock govern their domains.


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

This table is the initial baseline tuple, observed via read-only `git rev-parse HEAD` in the lane. Initial root/core
tracked status was clean. Inherited root SSH N2b prompt drafts and
GwzWorkspaceRouteMappingDesign, plus core GwzRemoteTransportBugReport, are
out of scope and preserved. The latter has private host/account material and
must not be committed publicly as inherited. Current design files add new
docs only; no production file or platform guard changed. Root preserved that
package at root `fdfa9e2c6502c82d2e3894d0b2716ad7dbfed342`, core
`497149940f2ea570e8be0943af31e7de7127fde7`, evidence
`70f481147830707d375f4da7f5c70e2aff3f7460`. Those heads were rechecked before
the second batch; its changes are baseline/checkpoint prose and a new private
run only. Neither package accepts the provisional design.

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

### Additional isolated primitives, 2026-10-03

Root resumed the available subset in
[2026-10-03-tr1-8-primitives](../../gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-03-tr1-8-primitives/README.md)
(private access required), with a new unique external runtime
`E:/gwz-tests/tr1-8-primitives-20261003-61-9c481a`. The preceding frozen run
is unchanged. Pageant and released GWZ binary hashes were rechecked after copy.
No OS trust, proxy, service, account, zone or inherited job policy changed.

- Released HOME/known_hosts characterization: missing and nonexistent HOME,
  relative HOME and spaces-only HOME authenticate the owned key. Empty HOME,
  Unicode-only and Unicode+spaces paths, an existing first HOME without
  known_hosts, and an existing first HOME with a wrong host entry refuse before
  key offer (`GitCommandFailed`, invalid/unknown hostkey). All servers and owned
  Pageants were reaped. The first runner's prediction that empty HOME would
  authenticate failed and is retained; later versions collect observations.
  Authentication ends at intentional WorkspaceNotFound, not a full GWZ clone.
  These results require explicit compatibility dispositions against design §3's
  empty-path skipping, relative-path refusal and UTF-16 preservation clauses;
  they do not silently change that provisional contract.
- Actual pinned Pageant holding an encrypted disposable PPK displayed one
  owned deferred-decryption dialog. SendMessageTimeout returned after 2037ms;
  no timeout response was read. The named mapping still existed after sender
  references closed and disappeared after the owned Pageant was killed/reaped.
  Synthetic late-write timeout (64ms), collision error183 without writing,
  vanished selected window and replacement-owner refusal also executed. Numeric
  HWND reuse was not observed; cross-SID and concurrency remain unexecuted.
- Native owned local pipe: replies arrived separately as 1 then 2 bytes; an
  outstanding overlapped read completed with error995 after CancelIoEx, before
  OVERLAPPED/storage release. Owned server PID matched, handles/buffers/thread
  retired. Fixture allowlist rejects UNC before open and the pipe uses native
  REJECT_REMOTE_CLIENTS. Native OpenSSH service identity is still unproved.
- Python native Job Object supervisor assigned suspended direct/spaces,
  shell/spaces, GUI and nested-job helpers before resume. Actual stdout arrived
  as 1 then 2 bytes. The complete private-job process lists (4/5/2/4 members)
  were pinned by handle/membership and all reaped after kill-on-close, including
  shell descendants. Earlier fixed family-size assumptions failed and are
  retained. This proves the primitive, not configured-helper parser integration.
- NTLM and Negotiate acquired default and explicit synthetic credentials and
  created first-leg contexts; synchronous DeleteSecurityContext and credential
  release returned0, native buffers were zeroed. Owned worker cancellation
  between real context legs reaped the worker. In-flight native-call cancellation
  and complete helper/default identity isolation are still unexecuted.
- WDigest AcquireCredentialsHandle refused synthetic Unicode credentials with
  realm and empty domains, plus an ANSI control, with SEC_E_UNKNOWN_CREDENTIALS
  (`-2146893043`) before initialization. Separate per-handle DIRECT WinHTTP
  Digest GET and POST fixtures stayed at401 with no authorization header; POST
  sent the owned ten-byte body on both requests. Servers/handles retired.
  Neither observation proves global provider unavailability or Digest parity;
  no policy weakening, real account or fallback scheme was attempted.
- Native Schannel SslStream exposed actual peer DER/signature OIDs in TLS1.2
  and TLS1.3 with four owned RSA cert signatures (SHA1/256/384/512). RFC5929
  SHA1→SHA256 and other digests matched independent Python calculations.
  A wrong unique process certificate pin refused application data. No OS trust
  import occurred. Non-RSA/PSS/MD5 and the eventual transport TLS adapter/EPA
  composition remain unproved; this is not released native HTTPS evidence.

### Bounded residual primitives, 2026-10-03

The third isolated run
[2026-10-03-tr1-8-residual-primitives](../../gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-03-tr1-8-residual-primitives/README.md)
(private access required) uses external E:/gwz-tests/tr1-8-residual-20261003-61-5dd1f1.
Both preceding runs remain byte-for-byte unchanged. No production/design, native
trust, proxy, hosts, zone, service, account or job policy changes occurred.

- P02: four simultaneous sender threads each completed eight actual owned empty
  Pageant requests (32 total, 4ms), and eight nonce-tagged synthetic requests
  (32 total, 13ms), with distinct mappings and no crossed replies. Threads,
  receiver and actual Pageant were reaped. A versioned measurement repeat
  observed maximum four in-flight send scopes for actual and synthetic receivers
  (32 each; 4ms/918ms); no extra handle churn. Retired Pageant HWND 33226944 was not
  reused in 100000 native creates/destroys over 2223ms, within a fixed 100000/15s
  cap. Numeric reuse assertion is UNEXECUTED; no further churn was attempted.
- P06: process-pinned Schannel peer DER/OIDs and digests matched independent
  DER calculations for ECDSA P-256/SHA256, P-384/SHA384 and RSA-PSS SHA256/SHA384
  in TLS1.2 and TLS1.3. PSS parameters identify matching message/MGF1 hashes;
  the fixture refuses differing hashes. MD5 peers failed both native handshakes
  with AuthenticationException (exit 2); no native MD5 peer/hash pass is claimed.
  All clients, listener sockets and server threads retired. These are hash
  primitives, not a TLS1.3 channel-binding policy, transport adapter or EPA proof.
- B15: native URLMON MapUrlToZone(flags0) returned HRESULT0, zone1/Intranet for
  http/https://gwz-tr18/ and zone3/Internet for http/https://gwz-tr18.invalid/.
  Owned COM reference released. No routing, TLS, redirect or authentication row
  executed; names have no installed hosts/zone override in this batch.

### WDigest and SSPI worker ownership, bounded fourth batch

[2026-10-03-tr1-8-digest-workers](../../gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-03-tr1-8-digest-workers/README.md)
(private access required), external E:/gwz-tests/tr1-8-digest-workers-20261003-61-c879e2.
Prior three runs unchanged; no accounts, policy, trust, service, agent, proxy,
hosts/zone, product or design changes. Investigation stopped at its fixed budget.

- P07/B14: WDigest query succeeds, max token 4096. Four explicit synthetic
  acquire variants (basic Unicode realm; EX Unicode realm; EX ANSI realm; EX
  Unicode empty domain) all refuse SEC_E_UNKNOWN_CREDENTIALS in 1.94–2.13ms.
  Native identity sizes 48/72 and EX version 512 recorded. All owned diagnostic
  processes reaped; native identity buffers zeroed. HTTP method/URI exchange
  remains UNEXECUTED, zero initialization calls/listeners after failed acquire.
  Microsoft documents supplied credentials and distinguishes Credential Guard's
  blocked SSO from explicit credentials; this result does not establish a
  supported general Digest prohibition or prove the refusal's precise cause.
  A provider-accepted disposable identity/fixture remains a prerequisite; no
  account/policy weakening was tried or implied.
- P05: two actual explicit NTLM first-leg contexts and output/identity storage
  remained live through a 100ms cancellation deadline while workers waited in
  an owned native WaitForSingleObject fixture. Capacity 2 remained charged;
  20 third-slot admission attempts refused. CancelSynchronousIo returned false,
  error 1168 for both; storage was not freed. After native waits timed out at
  about 2000ms, held kernel thread handles confirmed exit before output/identity
  zeroing, FreeContextBuffer/DeleteSecurityContext/FreeCredentialsHandle (all 0)
  and capacity release. Canceled results were suppressed. Earlier missing
  thread-query rights caused a retained v1 failure; versioned v2 uses documented
  own-thread query rights and passes. Outer owned fixture process reaped.
  Real SSPI calls took 0.057–0.104ms and had already returned: blocked-provider
  cancellation remains UNEXECUTED. Native wait is a controlled seam, not a
  simulated success for an actually blocked SSPI provider or LSASS quiescence.

A production deadline needs a proved provider-interruption seam or an isolated
worker/broker contract with bounded admission, retained ownership and its own
secret review. Broker exit cannot establish external Pageant prompt/mapping
quiescence, as the earlier receiver-retention evidence already demonstrates.
No new Pageant or guardian test ran. Root owns design revisions and dispositions.

## 3. Released baseline — executed facts and remaining rows

### Native Rust TLS binding primitive, fifth bounded batch

[2026-10-03-tr1-8-tls-adapter](../../gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-03-tr1-8-tls-adapter/README.md)
(private access required), external E:/gwz-tests/tr1-8-tls-adapter-021ace.
Native Win11 build 26200, Rust 1.95 x86_64-msvc; native-tls 0.2.18,
tokio-native-tls 0.3.1 and Schannel 0.1.29. Standalone fixture dependencies have
their own locked resolution; this is not a full MAIN candidate build.

Five rows pass: each of two distinct verified origin leaves returns the expected
independently calculated SHA256 binding through the actual Rust API; untrusted
and wrong-name TLS peers refuse before binding query/application write; nested
TLS proxy, CONNECT and origin TLS produce distinct correct proxy/origin bindings.
Only connector-local fixture roots are used, with hostname checks enabled.
Owned Python/OpenSSL servers read external PEM keys; the unexecuted native PFX
server shape was discarded because its import could persist keys. No OS trust,
proxy, hosts, account, policy or service changes. Native client/server exit 0 and
owned server threads join/reap. Collector bookkeeping failures are preserved
separately from the successful native execution.

This closes the initial native TLS binding API primitive, not EPA, HTTP auth,
Hyper/pool integration, full algorithm/TLS-version coverage or renegotiation.
§8 of the DRAFT design now selects this API; no review GO or product code change.
The fourth batch's public-document hashes describe its handoff snapshot; this
coordinator addition changes public documentation only and preserves its raw run.

Released behavior assertions below use **released 1.0.17**, exact archive/binary hash,
source tags and pinned Rust/toolchain provenance where built. Native prerequisite
observations are explicitly labeled and do not complete released assertions. A current
1.0.17 version string in a main candidate binary is not released 1.0.17 proof.

| ID | Status | Row | Observed outcome or required assertion |
|---|---|---|---|
| B01 | EXECUTED | S4.1 Bash/toolchain | PASS: explicit Rust/Cargo 1.95 and native pinned probe compiled/executed |
| B02 | EXECUTED | S4.1 storage/config | PASS: E: write/read/remove, both Cargo config names absent through runtime ancestors; external native output |
| B03 | EXECUTED (authentication) | HOME unset | PASS authentication: HOMEDRIVE+HOMEPATH and separate USERPROFILE-only known_hosts rows; not a complete GWZ clone |
| B04 | EXECUTED (characterization) | Home edge cases | Missing/nonexistent/relative/spaces authenticate; empty/Unicode/existing first HOME without correct known_hosts refuse. Compatibility dispositions unresolved |
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
| B15 | PARTIAL (native prerequisite) | Zone/redirect | URLMON flags0 classifies gwz-tr18 as Intranet/1 and gwz-tr18.invalid as Internet/3 for HTTP/HTTPS; released redirect/auth assertions unexecuted |
| B16 | UNEXECUTED | EPA | Server requires channel binding, successful auth; missing/wrong binding negative controls |
| B17 | PARTIAL | macOS/Linux | Mac plain-HTTP Negotiate unsupported characterization; HTTPS blocked by native trust; Linux unexecuted |
| B18 | UNEXECUTED | Windows POST challenge | Anonymous discovery then 401 POST: actual replay/body behavior and effect recorded |

Fixture names proposed: `gwz-tr18` and `gwz-tr18.invalid`, with proposed owned
loopback routing. URLMON measured Intranet/1 and Internet/3 respectively for
HTTP/HTTPS in the third run; routing itself remains uninstalled/unexecuted.
Do not edit hosts or zone configuration without coordinated authorization.
Do not use a live account to prove a disposable identity row. An explicit
credential selected from a configured test helper must be distinguishable from
the operator's logon identity by server-side identity class assertions, without
retaining user name/SID/token. If a temporary local account is necessary,
coordinate its creation/removal rather than silently inventing it.

## 4. Mandatory physical primitives — partial proof, no freeze

| ID | Status | Primitive | Observed proof / remaining required counterexamples |
|---|---|---|---|
| P01 | PARTIAL | Pageant mapping | Actual Pageant ACL/list/signature and owned collision183/no-write executed; cross-user negative remains |
| P02 | PARTIAL | Pageant bounded send | Actual encrypted-key dialog timeout and live mapping until owned Pageant reap; synthetic late write, vanished/replacement-owner refusal executed. Concurrency 4×8 actual and synthetic requests passed; numeric reuse remains unexecuted after fixed cap |
| P03 | PARTIAL | Local pipe | Owned local pipe partial replies/cancel completion995 before storage release, fixture no-UNC guard and server PID executed; native service identity remains |
| P04 | PARTIAL | WinHTTP capture | Native capture/GlobalFree and exact DIRECT restore executed; grammar, implicit loopback bypass and immutable candidate snapshot still need proof/disposition |
| P05 | PARTIAL | SSPI | Default handshake/synthetic CBT plus explicit/default first legs and between-leg worker retirement executed; bounded canceled-worker storage/capacity retention also passes; controlled wait is not in-flight provider cancellation. Full helper isolation/HTTP EPA/provider deadline remain |
| P06 | PARTIAL | TLS/EPA | Earlier Schannel hash rows pass within their scope; MD5 native handshakes fail. Native Rust binding API now passes two origin digests, untrusted/wrong-name refusal and distinct nested proxy/origin bindings. EPA, integrated adapter lifetime and MD5 disposition remain |
| P07 | PARTIAL (refusal) | Digest | Basic/EX Unicode/ANSI WDigest acquire refused at fixed diagnostic cap; direct WinHTTP HTTP GET/POST remain401/no offers. Native method/URI exchange unexecuted; accepted fixture/provider cause unresolved |
| P08 | EXECUTED (primitive) | Helper Job Object | Spaces/direct/shell/GUI/nested job, assignment before resume, partial stdout and every enumerated owned descendant kill/reap executed; helper parser integration separate |

These spikes run outside repositories and build outputs stay outside the
private evidence archive. A spike can establish a primitive before contract
freeze; it cannot remove Windows compile guards or claim product parity.

### Remaining-row authorization and dependencies

Every unresolved row remains a freeze gate; the following classification is a
handoff, not permission to drop a required assertion. Root resumed the isolated
subset above while Mac approval is unanswered; this batch is now bounded and
complete. The rows below identify residual work, not an execution approval.

| Rows | What can run in existing scope | Additional prerequisite / boundary |
|---|---|---|
| B04 | Executed HOME edge characterization | Root must settle explicit empty/relative/Unicode/first-existing-home compatibility dispositions |
| B06/B08 | Own selected-pipe protocol fixtures can characterize selection | Actual native OpenSSH service/both-agent rows require coordinated service/agent setup and disposable keys; absent pipe inventory is not completion |
| B09/P04 | Owned machine-proxy grammar/bypass and capture/snapshot fixtures | Root serializes each proxy interval; exact prior DIRECT/guard/restore required; no generic reset/PAC/user-proxy change |
| B11 | Fake configured-helper invocation counter plus default-logon server fixture | Full released HTTPS row needs approved Windows native trust/supervisor precondition; helper is fake, no live gh |
| B12/B13 | Generated explicit fixture credentials and controlled verifier can be prepared | Full HTTPS needs Windows trust. If OS-backed distinct user is needed, account/token provisioning needs separate root approval; no operator password use |
| B14/P07 | WDigest acquire and direct WinHTTP Digest GET/POST refusals executed | Current synthetic-credential fixture does not complete Digest. Four basic/EX Unicode/ANSI variants exhausted the bounded diagnostic and refuse acquire; obtain a provider-accepted disposable identity/fixture or root-owned mechanism redesign. No general unsupported-provider claim; full released row needs trust |
| B15 | Native URL-zone classification executed; fixture routing can be prepared | Full HTTPS requires trust and named certificate/routing. Try owned proxy routing without hosts edits; hosts/zone edits require root approval if unavoidable |
| B16 | Existing synthetic CBT proof remains partial | Actual TLS/EPA-required positive/negative server rows need Windows trust and verified peer binding; no distinct account necessarily required |
| B17 | Mac row awaits requested approval; Linux fixture source can be prepared | Mac native trust unapproved; Linux target assignment/prerequisites needed, no Linux row attempted here |
| B18 | Owned discovery/POST challenge/replay fixture can be prepared | Full released HTTPS row needs Windows trust; document actual body/effect, not inferred replay |
| P01 | Collision/ACL/list/signature executed | Real cross-SID actor requires separately coordinated distinct token/account; no real agent changes |
| P02 | Actual prompt/mapping retirement and synthetic stale/replacement cases executed | Concurrency executed; numeric reuse not observed within fixed 100000/15s cap. Mandatory reused-window assertion remains unexecuted, requiring root disposition or a separately bounded reproducible fixture |
| P03 | Owned overlapped partial/cancel/no-UNC primitive executed | Native service identity/auth still needs coordinated provider setup; fake server is not native service proof |
| P05 | Default/explicit first legs and between-leg owned worker cancellation executed | Controlled-wait canceled capacity/storage retention executed; actual provider blocking/interruption unproved. A practical isolated ownership/deadline seam and full identity proof remain; no account/policy change authorized |
| P06 | Native peer/hash primitives and initial Rust binding API executed | MD5 native refusal needs explicit disposition. Hyper/pool/lifetime integration and EPA remain; released HTTPS needs approved native trust |
| P08 | Complete owned job primitive executed | Parser/configured-helper composition remains later implementation; inherited host job policy/service/elevation changes prohibited |

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
