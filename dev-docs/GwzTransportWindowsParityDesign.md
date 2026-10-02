# Windows transport parity (TR1.8)

Date: 2026-10-03. Status: **DRAFT; not frozen, accepted, implemented or
release-qualified.** Required prefreeze native rows are pending in
[the baseline record](GwzTransportWindowsBaseline.md). No downstream Windows
implementation may consume this as GO. S4.1 native prerequisites and partial
Pageant, SSPI and machine-proxy rows have executed; their limits remain gates. The current design work changes no
product source and preserves `endpoint_environment`'s `compile_error!`.

## 1. Authority and object

The transport release plan, amendment 2 revision 6 §§3.5, 3.18 and 3.19,
and V110 §2 and S4.1 control. OD15 places every parity mechanism inside the
transport; OD16 permits default credentials to any host. TR1.6 revision 4's
accepted helper contract and operator answers, and TR1.5's accepted setting
contract, control their seams. This document cannot reverse those decisions.
Process: root AgentProcessRules, GwzProcessOptimization §§3.1, 4 and 8;
review-loop's canonical prompts. Evidence locations follow root EVIDENCE.md,
with V110 §2's explicit E: override for this campaign only.

Read baseline: core `2e64e88a28c332ed422cc390adc76738dc701bb1`, root
`12f11c7949919834fe8858247dc4c0cd49a8134b`. Full dependency tuple and inherited
noise are in the baseline record. The draft is not an exact committed review
object until the root owner settles it through GWZ.

Owned outputs: this design, its concise baseline record and proposed user
guide, plus the one root-authorized private run and disposable primitive
fixtures. No shared interface implementation, main/sibling edits or Git/GWZ
mutation are owned here. S4.2–S4.5, TR4.6–TR4.10 and TR8.4
remain later work. TR4.10 receives its separate mandatory secret-handling dual
gate; the Windows phase receives its aggregate review.

## 2. Runtime ownership

One runtime captures the environment, machine proxy configuration, local agent
source and caller token identity before its first Open. The CLI command and
the Python per-operation entry each take this capture at their existing
snapshot point. Workers receive immutable owned values; they never reread
process environment, WinHTTP configuration or agent selection during retry.
No process-global mutable cache, thread-local state or credential cache is
introduced. Unique mapping names and credential scopes use the runtime's
IdSource. Platform code lives in enclosing Windows modules or cfg_if blocks.

Capture owns configuration only. Network, known_hosts reads, agent request
exchange and SSPI work execute in supervised endpoint jobs under Control.
The machine proxy's WinHTTP-owned strings are copied into validated owned
values and GlobalFree'd on every exit, partial failure included. Capture
failure refuses the operation before an endpoint opens.

The future 1.2 server compares the client's token logon AuthenticationId at
SessionOpen, per amendment §3.18. 1.1 does not implement server admission.
Local Pageant and SSPI use the current caller's logon session, never an
impersonated remote user. A default credential identity is not a password or
an environment value.

## 3. SSH home and trust

Resolve from the captured Windows environment in libgit2's order: HOME,
HOMEDRIVE concatenated with HOMEPATH, then USERPROFILE. Empty or unavailable
candidates are skipped. Preserve UTF-16 paths and spaces; do not round-trip
through UTF-8. A selected relative or malformed path is refused with a
specific home error before agent/network I/O. The primitive row must compare
libgit2's handling of missing/nonexistent candidates before this clause freezes;
it may require an explicit compatibility disposition, not a silent fallback.

known_hosts and `~/` identities resolve under that captured home. Explicit
absolute identities retain their existing behavior. The same resolver must
cover SshEndpointConfig::from_environment or retire that duplicate factory.
Known-host validation remains mandatory before any authentication; no trust
failure invokes native transport or another agent. The Windows network arm
uses the existing begin_slice/end_slice semantics, including disabled stall
with an aggregate deadline still enforced.

## 4. Agent source and local pipe

Select once: `FindWindowW(L"Pageant", L"Pageant")` in the host's desktop/logon
session, as libssh2's Pageant-first backend; otherwise SSH_AUTH_SOCK from the
snapshot; otherwise `\\.\pipe\openssh-ssh-agent`. "Visible" means the
window discoverable by this protocol, not IsWindowVisible: Pageant's protocol
window can be hidden. Both sources running selects Pageant. A Pageant pipe
named in SSH_AUTH_SOCK is an ordinary selected pipe.

The source is an owned enum, not a PathBuf that sometimes means a window.
Pageant's identity includes the HWND, pinned process handle, process creation
identity, caller SID and logon AuthenticationId. Before each request confirm
the window still belongs to that pinned process. Vanished/replaced Pageant
refuses; it never selects the pipe. A found Pageant owned by another SID or
logon session refuses rather than falling through. This strengthens local
source verification; the baseline must record its effect against 1.0.17.

Admit local native pipe names only; reject UNC remote servers, traversal and
unsupported mingw Unix socket forms before connect, naming SSH_AUTH_SOCK.
Connect with SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, then verify the
pipe server process token has the caller's SID and logon AuthenticationId.
No SMB authentication may occur. The physical spike must prove the native
OpenSSH service's server identity; if it is SYSTEM, this proposed same-SID rule
cannot be frozen unchanged. Record a narrow verified-service alternative or
seek an amendment rather than pretending native OpenSSH satisfies it.

Pipe I/O is overlapped, one request owner per handle, completion polled under
Control. CancelIoEx targets that owner's OVERLAPPED. A cancelled request retains
its buffer/OVERLAPPED/event until completion is observed, then closes handles.
ERROR_NOT_FOUND is not evidence of completed I/O. Cleanup unconfirmed stays
owned in the runtime cleanup aggregate; no detached thread owns borrowed data.
Source disappearance, malformed reply and failed trust poison that channel.

## 5. Pageant exchange

Use WM_COPYDATA, dwData `0x804e50ba`, with a NUL-terminated ASCII mapping name.
Create an 8192-byte pagefile-backed Local mapping, exclusive creation with
collision refusal, owned by the caller. The name is
`Local\PageantRequest-<pid>-<runtime-id>-<request-id>`; the PuTTY spike must
prove that pinned Pageant accepts this name. The actual PuTTY 0.83 spike
now accepts the Local/unique name, caller+SYSTEM ACL and RSA SHA-256 signature;
collision and cross-user counterexamples remain unexecuted. Grant only caller SID and SYSTEM
read/write; no Everyone access. Same-user Pageant can open it; cross-user
Pageant is refused before creation. Mapping ACL/owner and pre-created-name
attack are physical proof rows.

The four-byte big-endian frame plus payload must fit 8192 bytes, so payload is
at most 8188. Reject oversized requests before dispatch. Validate reply length
1..8188, message type, exact payload exhaustion and every signature/key bound
using the existing agent codec; never copy 8192 bytes starting after the header.
Unsupported keys are skipped in order under TR2.8. An empty list or no usable
key says `Pageant has no key this transport can use`; a declined signature
permits the next key, while malformed I/O ends the channel.

Each request has its own mapping and serialized owner. No concurrent caller
writes one mapping. Use SendMessageTimeoutW to the pinned window, never
HWND_BROADCAST, with SMTO_BLOCK | SMTO_ERRORONEXIT and without
SMTO_NOTIMEOUTIFNOTHUNG. Same-process/same-input-queue windows are refused because
the documented timeout is otherwise ignored. There is one send, no resend
after a short timeout: a sign request may still be active in the receiver.

The proposed request bound is the remaining Control allowance, including the
existing setup aggregate and stall bounds. Pageant confirmation does not reset
those clocks. No independent 120-second agent allowance is introduced. Cancel
may return while the supervised request retires; cleanup must be observed
within the existing cleanup bound or reported unconfirmed. The spike must
prove late receiver mapping writes and timeout ownership before freezing this
choice; SendMessageTimeout returning alone does not prove the receiver stopped.
Retain input storage until send completion, never read a mapping after a timeout,
and never reuse its name. Wipe on confirmed quiescence before unmap/close.
A synthetic native receiver retained its mapping reference, then wrote after
the sender timed out at 50ms and closed its own mapping references. This proves
OS reference ownership can survive sender close; it does not prove real Pageant
confirmation cancellation or receiver quiescence. No timed-out response was read.
If quiescence cannot be proved within the bound, an isolated request broker
design is required before GO, not an unbounded in-process orphan.

Mapping contents, agent comments, key bytes and signatures never reach logs,
failure text or evidence. Counts, algorithm names, opaque source IDs and
redacted result classes suffice. A Pageant request timeout names the bound and
the confirmation/key action, without suggesting fallback happened.

## 6. WinHTTP machine proxy

Read WinHttpGetDefaultProxyConfiguration once beside capture, for both runtime
entries. On Windows this machine setting wins: environment HTTPS_PROXY,
https_proxy, ALL_PROXY, all_proxy, NO_PROXY and no_proxy are ignored, matching
1.0.17's WinHTTP path. The user guide states this inverse of Unix behavior.
Do not merge bypass lists from ignored variables.

NO_PROXY access means direct. NAMED_PROXY admits a bare host[:port], or a
semicolon-separated scheme mapping with a single applicable HTTPS entry;
default port is 80. Bracketed IPv6 is parsed explicitly. Reject ambiguous
multiple applicable proxies, malformed ports, userinfo, URL paths, PAC/automatic
forms and unsupported schemes before any Open, naming WinHTTP machine proxy.
Windows default-proxy API does not stand for the user's browser/PAC settings.
The accepted grammar and defaults must be compared with executed baseline
rows, with unsupported working 1.0.17 forms explicitly disposed before freeze.

Bypass is semicolon-delimited. `<local>` matches a hostname without a dot,
not every RFC1918 address. Literal hosts and addresses compare without ASCII
case; `*` is a glob within the complete host pattern, never URL/path matching.
Matching uses the request's canonical destination host after discovery
redirect; CONNECT uses the same destination and chosen proxy. No DNS lookup
expands `<local>`. Reject bypass syntax the implementation cannot interpret;
do not silently send direct on a parser failure. Wildcards, trailing dots,
ports and IPv6 need baseline counterexamples before this grammar freezes.

A loopback 407 Negotiate baseline decides proxy authentication. Until that
row executes, **no proxy credential mechanism is designed as supported**.
If 1.0.17 refuses, the transport refuses 407 before generating any token,
names the machine setting and the off switch, and offers neither a helper
credential nor the login. If WinHTTP authenticates automatically, this document
requires a revised proxy-specific SSPI design, scope and Safety attack before
GO. Origin Authorization never becomes Proxy-Authorization. The administrator
campaign must capture the exact prior configuration and restore/verify it;
`reset proxy` alone is only a restoration for an initially direct machine.

The released native fixture used this machine proxy for a reserved .invalid
origin even with conflicting HTTPS_PROXY. Both tested numeric loopback origins
bypassed it despite an empty configured bypass list. Preserve or explicitly
dispose that implicit native loopback behavior before freeze; these two addresses
do not prove the entire native bypass grammar. Negotiate/NTLM407 rows reached the
proxy once, offered no credentials and failed with the native string-conversion
error. Authentication support is unclaimed; other proxy forms remain proof rows.
All shared proxy state was restored to exact DIRECT with an armed deadline guard.

## 7. Origin authentication choice

Initial discovery is anonymous. Retain its final challenged HTTPS repository
base U and complete challenge set after the existing validated discovery
redirects. Parse each field's first scheme token without case. No origin
credential accompanies CONNECT. Certificate trust is validated before any
origin authentication token is generated.

If NTLM, Basic or Digest is offered and policy AllowConfigured applies, ask the
helper once for U under TR1.6. A usable helper identity selects the highest
offered scheme in this order: Negotiate, NTLM, Digest, Basic. Thus a mixed
Negotiate+Basic offer uses that helper over Negotiate, not the logon session.
A Negotiate-only offer asks no helper even if one is configured. Disabled
disables helpers only; default credentials remain allowed, as 1.0.17's callback.

No credential, invalid helper output, missing git and unstartable git count as
no helper identity on Windows, under TR1.6 §4. If Negotiate or NTLM is then
offered, select the default logon credential over Negotiate first, NTLM second.
A helper timeout/cancel ends the request; it never starts default credentials.
Once a usable helper identity was sent and rejected, never try the logon
identity, another scheme or another helper. No supported challenge yields the
existing unsupported-authentication message, naming offered scheme names as
untrusted quoted text and the native setting hint.

OD16 has no zone test: U under either the Intranet or Internet fixture name
can receive the login's NTLM response. This is an accepted hazard, not a finding
that may reimpose a zone boundary. Discovery redirects change U before lookup;
redirects during the authenticated exchange end it with no credential sent to
the new location. TR1.6 OQ4 (a) controls authenticated Basic redirects; SSPI's
connection-bound exchange does not reuse an old context at a new URL.

## 8. SSPI and Digest

AcquireCredentialsHandleW with SECPKG_CRED_OUTBOUND selects Negotiate, NTLM
or WDigest. Default identity uses NULL auth data, current logon session only;
helper identity uses SEC_WINNT_AUTH_IDENTITY_W with explicit Unicode values.
Handle ownership includes the zeroizing identity buffers for their entire
possible provider use. DOMAIN\user splits once; UPN remains the user with
empty domain. No password, identity, token or OS error string is logged.

For Negotiate/NTLM target is `HTTP/<canonical U host>` without port or path,
brackets removed for IPv6. Preserve U's host, not a reverse-DNS alias.
Request ISC_REQ_CONNECTION | ISC_REQ_ALLOCATE_MEMORY, no delegation and no
required mutual-auth flag that would remove NTLM parity. Check status and
returned attributes instead of assuming requested attributes were granted.
SEC_I_CONTINUE_NEEDED continues on the same lease. SEC_I_COMPLETE_* requires
CompleteAuthToken before using output. Other status values fail explicitly.
Bound every token by QuerySecurityPackageInfo's maximum and the HTTP header
bound; bound the exchange to eight request rounds and the existing setup clocks.
The spike must establish these flags and the finite-round parity envelope.

Pass SECBUFFER_CHANNEL_BINDINGS from the actual verified TLS connection:
SEC_CHANNEL_BINDINGS with application data `tls-server-end-point:` plus the
RFC5929 certificate digest (weak signature hashes upgraded to SHA-256).
All offsets/lengths are checked. Do not use the configured CA certificate,
the original redirect host's certificate or bytes from a prior connection.
The TLS adapter must expose peer DER and its signature algorithm to this
private core boundary; the current native-tls seam does not already prove it.
TLS 1.3/certificate-algorithm cases and an EPA-required server are proof rows.

Digest uses WDigest's HTTP mode ISC_REQ_HTTP | ISC_REQ_ALLOCATE_MEMORY with
helper identity only. Its input includes the entire validated challenge token,
the actual HTTP method and percent-encoded request URI per Microsoft's Digest
input-buffer contract. Digest output is opaque; do not independently implement
hashing, nonce grammar or qop. Basic uses the existing helper path. Digest
per-request tokens use that request's method/URI; no discovery token is reused
for POST. If WDigest cannot reproduce WinHTTP's disposable Digest row, revise
this mechanism before freeze rather than claiming Digest parity from inventory.

Each context has one owner, no concurrent InitializeSecurityContext calls.
FreeContextBuffer after copying/wiping each provider output; DeleteSecurityContext
and FreeCredentialsHandle exactly once on all exits. Network waits follow
Control. Synchronous SSPI may block in OS/provider work: a supervised job must
retain all storage and its capacity until it exits. Cancel/cleanup timeout
reports unconfirmed, cannot free live buffers or admit unbounded replacement
jobs. A native spike must prove a practical deadline/capacity seam; if it cannot,
freeze requires an isolated broker design and its own secret review.

## 9. Connections, retries and POST

The challenged connection is exclusively leased to the operation throughout
Negotiate/NTLM. Consume a bounded 401 body before sending the next request;
connection close, framing error or redirect discards it and its context.
Never continue an exchange on a new connection with an old context/token.
No HTTP multiplexing or another route shares that lease.

A credential-bearing connection receives TR1.6's opaque route scope: same
operation, original route/service and pinned U only. Default credentials also
mint a route scope; matching a logon SID alone cannot allow cross-route reuse.
Authenticated idle connections remain subject to pool caps/eviction and are
discarded at route retirement. Anonymous connections never inherit the scope.
Digest and Basic retain the same credential-route isolation.

Rejected authentication is terminal, as TR1.6 §7. Actual network loss can use
TR2.1's per-key setup retry only before effect and within its attempt bound;
every new attempt creates a new SSPI context and remains on the same captured
identity/source. Never retry a partial/complete POST to acquire credentials.
Windows 1.0.17's POST challenge replay without its already-sent body is recorded
as a parity observation; the proposed transport refuses it rather than replaying
effects. Its explicit compatibility disposition is required before freeze.

## 10. Helper executable and setting seams

Windows git discovery searches captured absolute PATH entries for `git.exe`;
do not implicitly search cwd or append a shell script extension. Paths with
spaces are passed as an executable path, not a shell command. git receives the
captured environment and TR1.6's exact stdin/limits, including accepted GUI
prompt policy. It owns configured helper parsing: do not implement credential
helper shell forms a second time. TR2.2's direct gh executable and `!gh auth
git-credential` forms remain test rows while that old path is retired.

Start git suspended, assign it to a kill-on-close Job Object before resume,
and on timeout/cancel kill/wait for the job, including child helpers. Assignment
failure refuses before resume. Breakaway is not enabled. Pipe buffers stay
owned until asynchronous I/O and process cleanup complete. External/nested-job
and GUI-helper behavior are primitive proof rows, including Python's caller.

TR1.5's three forms work on Windows unchanged: flag, captured GWZ_TRANSPORT,
user-global git gwz.transport; precedence flag > environment > global > default
gwz. Each native form prevents runtime construction. Repository/local values
remain ignored under TR1.5. Removal uses its lifecycle pairs, not a new Windows
settings file. The global home resolution must share §3's Windows home contract.

## 11. Freeze and implementation sequence

Before GO, all baseline rows B01–B18 and primitive rows P01–P08 in the baseline
record need executed results with exact inputs/toolchain/binary hashes. Resolve
each proposed compatibility difference and every conditional choice above,
then replace provisional clauses with one physically proved design. Inventory
or a documented API is not executed evidence.

Settle through the root owner; record committed root/core/dependency tuple.
Required independent Consistency and Safety reviewers receive the same exact
document and controlling graph, canonical review prompts and report paths.
Surface receives only the proposed user guide and existing help/docs, never
this design or implementation. File reports verbatim; no self-GO, no dirty
review. All P0/P1/P2 block. One merged remediation patch, same reviewers'
original counterexamples, at most two architectural remediation rounds.

After design GO: S4.2 network/trust, S4.3 pipe, S4.4 home/assembly, TR4.8
Pageant and TR4.9 proxy, TR4.10 secrets with its dual gate, then S4.5 integrated
candidate and ordinary Windows CI/review/parity batch. Dependencies come from
the amendment, not this list's typography. Each cohesive source file stays
under 500 lines; tests are reported separately. Disabled Windows/Unix branches
must pass fast syntax-aware scope checks. Do not remove the deliberate platform
compile_error until its complete Windows implementation is ready.

## 12. Primary API references

- [SendMessageTimeoutW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw): timeout flags and same-queue exception.
- [WinHttpGetDefaultProxyConfiguration](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpgetdefaultproxyconfiguration): machine configuration and allocation ownership.
- [SEC_CHANNEL_BINDINGS](https://learn.microsoft.com/en-us/windows/win32/api/sspi/ns-sspi-sec_channel_bindings): layout and endpoint-binding form.
- [Digest InitializeSecurityContext](https://learn.microsoft.com/en-us/windows/win32/secauthn/initializesecuritycontext--digest): HTTP mode, URI and single-context concurrency.
- libssh2 1.11.1 agent.c:336–440 (registry libssh2-sys 0.3.2) gives the
  Pageant 8192-byte window protocol and source order. libgit2 at the baseline
  winhttp.c:139–212, 618–640, 830 and 1243–1262 gives scheme/default/proxy
  behavior; sysdir.c:328–330 gives home candidates. These are static references.
