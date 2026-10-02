# Windows native proof dispositions — proposed, not accepted

2026-10-03. This is a coordinator working note for TR1.8, not a new authority
or a completed parity/design gate. The controlling
[Windows design](GwzTransportWindowsParityDesign.md) remains DRAFT and NO-GO.
Each proposal below needs its stated proof and incorporation into the settled
design before independent Consistency/Safety review. No product code changes.

## 1. HOME behavior (B04)

The released binary authenticated with missing/nonexistent HOME, a relative
HOME and a path containing spaces. Empty HOME, Unicode paths and an existing
first HOME with absent or wrong known_hosts refused authentication. These are
authentication observations, not successful workspace clones. Exact receipts
are linked from [the baseline](GwzTransportWindowsBaseline.md); raw evidence
requires private-member access.

The local libgit2 source explains why a directory's existence matters:
`src/libgit2/sysdir.c`, `find_win32_dirs`, expands HOME, HOMEDRIVE+HOMEPATH and
USERPROFILE in that order, retains existing paths, and its home lookup selects
the first existing directory. It does not search subsequent homes because the
selected home's known_hosts is missing or rejects the host. This inspection is
of member `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`; the executed released
binary's separate provenance remains in the baseline.
The inspected sysdir.c is byte-identical to v1.9.7 (SHA256
`bc7368efe6db818f64a8d8b4178455da7b8f8e28b157dc99ed95b094d3e0170f`).

Proposed dispositions:

| Case | Proposal | Closure required |
| --- | --- | --- |
| Missing/nonexistent candidate | Preserve ordered selection of an existing directory. Capture environment and current directory; perform existence/trust I/O in the supervised job. | Missing/nonexistent cases select the expected fallback from an immutable snapshot. |
| Relative HOME | Resolve against the captured current directory before handing it to a worker. Do not silently turn the successful released case into a refusal. | Change process cwd after capture; the job still reads the original fixture's known_hosts. |
| Spaces | Preserve native path bytes/code units. | Authenticated owned fixture, no shell splitting. |
| Unicode | Fix the path handling; do not preserve a conversion-related refusal as intended policy. The observed failure does not establish its precise cause. | Native Unicode known_hosts fixture passes; negative host-key control still refuses. |
| Existing first HOME without usable known_hosts | Preserve refusal without trying another home's trust file. | Missing and wrong first-file controls produce zero key offers even with a valid fallback file. |
| Empty HOME | Refuse an explicitly empty HOME with a specific home diagnostic. Do not silently use another trust directory. | Empty case fails before a key offer; missing-variable case remains distinct and succeeds. |

Empty-HOME refusal preserves the measured refusal, while making its cause
explicit; it does not claim to reproduce every legacy expansion of an empty
string. Captured cwd/path resolution must serve SSH trust, `~/` identities and
the accepted transport-setting global-config lookup consistently. This is a
proposed change to §3's current skip-empty/refuse-relative wording, not an
implemented exception or an approved compatibility disposition.

## 2. Pageant timeout ownership (P02)

Two separate owners must be described. GWZ owns its send scope, mapping handle
and view. Pageant is an external application and can independently own a
mapping reference and a deferred signing prompt. The actual Pageant fixture
kept the mapping alive after sender timeout/close until the owned fixture
Pageant was reaped. The synthetic receiver wrote after sender timeout/close.

Consequently, a send timeout cannot prove that Pageant finished, cancelled its
prompt or stopped writing. [SendMessageTimeoutW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw)
defines a caller wait bound and a same-queue exception; it provides no Pageant
request-cancellation protocol. [WM_COPYDATA](https://learn.microsoft.com/en-us/windows/win32/dataxchg/wm-copydata)
also distinguishes message storage from data copied/retained by a receiver.
The separate mapped payload has its own reference lifetime, as the native
fixtures demonstrate.

An isolated request broker can bound GWZ's worker lifetime. Killing it cannot
retire an external Pageant prompt or guarantee wiping a receiver-held mapping.
The draft must choose a truthful receiver boundary before promising cleanup:
retire our ownership, never reuse the name, never consume a late response,
never resend the timed-out signing request, and report the external prompt's
possible continued existence. Never terminate an operator's Pageant to satisfy
cleanup. Broker reaping and receiver quiescence must have separate assertions.

This is an unresolved design requirement, not approval to discard the wipe
obligation. Before freeze, inventory exactly what enters the mapping, prove no
private key/helper password/default credential is transmitted through it, and
state the limits on wiping data already handed to the external agent. If a
stronger receiver guarantee is required, name a supported primitive that can
provide it; a broker alone is not that primitive.

## 3. Window identity (P02)

No retired numeric HWND was reused in the fixed 100,000-attempt/15-second
budget. Keep that result as UNEXECUTED for the reuse assertion. Do not spin
until success or label the absent counterexample a pass.

A held process handle/creation identity distinguishes owners, but a check
before SendMessageTimeout is not an atomic check-and-send. The design review
must address destruction/reassignment between the check and dispatch, including
what request data another receiver could obtain and whether post-send owner
verification merely rejects its result. Either provide a bounded reproducible
native fixture, or explicitly revise the promised invariant and its test seam;
an injected identity mismatch is not evidence of actual numeric HWND reuse.

## 4. Weak certificate hashes (P06)

Both native MD5-signed peer handshakes failed. The ECDSA, RSA-PSS and other RSA
hash results remain passes within their recorded process-pin fixture scope;
none completes the actual transport TLS adapter or HTTP EPA proof.

Keep TLS acceptance separate from channel-binding hash selection. [RFC5929
§4.1](https://www.rfc-editor.org/rfc/rfc5929.html#section-4.1) selects SHA256
when the certificate signature uses MD5 or SHA1. A pure DER/OID test can prove
that selection without weakening Schannel to negotiate the failed peer. Such
a test is not a native successful MD5 handshake or universal provider-support
claim. Proposed disposition: preserve native TLS refusal; implement/test the
RFC hash-selection rule independently; still require binding to the actual
verified peer and EPA positive/missing/wrong-binding controls before GO.

Source inspection identifies a smaller preferred seam: the pinned
native-tls 0.2.18 already exposes `tls_server_end_point()` through the
tokio-native-tls 0.3.1 stream's `get_ref()`. Its Schannel 0.1.29 implementation
queries the client context's actual peer certificate and returns the digest.
Extract from the final origin handshake in `https_connection::connect`, before
the stream is wrapped for Hyper; extracting from a preceding proxy TLS
handshake would select the wrong certificate. Keep the result owned by that
physical connection and operation's auth scope, not cached by host or treated
as a unique socket identifier. SSPI adds the channel-binding prefix once.

Native `None`/error must have an explicit disposition before an SSPI offer;
neither silently permits authentication without binding. They need not prevent
unrelated anonymous/Basic use. The dependency maps MD5/SHA1/SHA256 to SHA256,
SHA384 and SHA512 to themselves, and unknown suffixes to `None`. No bespoke
certificate parser or new dependency is indicated by initial API extraction.
The separate bounded native Rust probe now passes using only connector-local
fixture roots and normal hostname validation: two distinct verified origin
bindings match independent expected digests, untrusted/wrong-name peers refuse
before binding query/application write, and nested proxy TLS/CONNECT/origin TLS
produces distinct correct proxy and origin bindings. Its retained package remains
separate from released native HTTPS/EPA proof. Renegotiation/peer
change is a lifetime consideration to resolve against §8's existing invariant;
an initial digest does not establish universal immutability. The draft's §8
now uses this demonstrated digest API; that DRAFT edit is not design acceptance.

## 5. Immediate bounded work and remaining prerequisites

The fourth isolated native batch investigated Digest's provider/API contract and
SSPI blocking-call ownership using disposable process/local fixtures. It made
no trust, service, account, hosts, zone, policy or shared proxy changes, and
retained failed attempts and fixed investigation limits.

Four explicit Digest acquire variants returned `SEC_E_UNKNOWN_CREDENTIALS`;
method/URI authentication remains unexecuted. The capacity/lifetime fixture
retained two live contexts through cancellation, refused excess admissions and
released storage only after thread exit, but its delay was a controlled native
wait. Actual observed SSPI calls returned quickly; in-flight blocked-provider
cancellation remains unexecuted. The fourth batch's retained package is linked
from the baseline; these are not new pass claims for the missing P05/P07 rows.

With these five batches retained, revise the Windows draft once around demonstrated
primitives and the explicit dispositions, rather than beginning implementation
against its conditional clauses. Native service identity/both-agent and
cross-SID assertions still need their coordinated prerequisites. Released
HTTPS/helper/EPA/POST rows still need a concrete approved trust fixture and
independently proved cleanup owner; old unanswered/expired proposals grant no
permission. Proxy grammar/capture rows require root-serialized save/restore.

Before settling the Windows design, refresh its controlling graph from accepted
MAIN core `a92a7990081475e349b182a332c7553731f4fb5b`: the helper timing/context
amendment and SSH helper clock mechanism landed after this lane's base. The
standalone native proofs do not implement or validate that later composition.
Carry the accepted timing, configuration and cleanup seams into the Windows
revision explicitly; do not review the older lane as if it contained them.

No missing row is waived here. Freeze and downstream implementation remain
blocked until the baseline's required assertions/dispositions are complete and
canonical independent reviews pass on the root-settled tuple.
