# SSH production setup — admission and implementation sequence

Date: 2026-09-21. Status: design accepted after retained Consistency/Safety GO.
Authority: GwzRemoteTransportDesign.md §§6–7, accepted SSH worker and agent
A1/A2/A3. This refines the later production-setup boundary in
GwzRemoteTransportSshAgentDesign.md §2. It does not activate production routing,
change CLI/core messages, or waive the operator-deferred platform/source batch.

## Ownership and operating-system admission

All connection creation remains under A3's pool entry and A1's setup Job.
Connector::start only captures owned bounded inputs and starts the Job. The
helper performs endpoint-local known-host loading/admission, name resolution, TCP connect,
SSH handshake, trust validation, and authentication. Successful handoff still
requires join, matching authority, and native authentication; no Git command is
opened during setup. One original absolute deadline and cancellation Control
covers the whole attempt. Expiry is checked before and after every OS/native
call and before authentication/publication. No stage restarts the deadline.

OS name resolution and regular-file access are admitted as potentially
non-preemptible calls. A caller may time out while one remains in the kernel.
The worker requests cancellation, refuses further effects if it returns late,
and retains the helper, physical pool charge and endpoint cleanup slot until
joined destruction. A3's sticky cleanup failure stops endpoint admission on an
overrun. Its process-wide bounds remain 64 helpers and 64 active-or-retained
endpoint cleanup owners; no detached resolver thread or replacement for a
still-live helper. A permanently stuck OS call can permanently consume a slot;
we explicitly accept that fail-closed availability cost. No hard physical
termination deadline for OS DNS/filesystem calls is claimed.

TCP and native SSH waits use nonblocking sockets and cancellable waits of at most
20 ms. Try resolved addresses sequentially under the same deadline; no connection
racing, background DNS cache or whole-connection authentication replay. Retain at most 32 resolved
addresses. Reject an empty result; a different address is eligible only before
SSH negotiation begins. Once handshake starts, a failure ends this setup attempt.
This prohibition concerns moving to another resolved address or restarting a
connection after SSH negotiation, not A2's within-session identity progression.
A2 still enumerates once and attempts each key once in order; only explicit
AUTHENTICATION_FAILED advances, while ambiguous/terminal native failure ends
setup. This changes neither pool keys nor DNS-equivalence rules.

## Host trust

The endpoint supplies its known_hosts path, normally the local account's
.ssh/known_hosts. Preserve current libgit2 host/key/port matching and credential-access policy: no SSH config,
ProxyCommand, interactive trust prompt, automatic insertion or fallback to an
untrusted host. Missing/empty/unknown/mismatching trust refuses before opening an
agent or offering a file key. The logical hostname and effective port, not the
resolved IP alias, select the trust entry. Native known-host parsing/checking
handles hashed names and nondefault-port entries.

Read only a regular file, with O_NONBLOCK on admitted Unix paths so a FIFO cannot
trap opening. Follow ordinary endpoint-local symlink semantics, then verify the
opened descriptor is regular. Limit known-host input to 4 MiB and each line to
16 KiB excluding the CR/LF terminator. Require UTF-8, NUL-free chunks before
native parsing; malformed/unsupported input fails closed. Parse complete physical
lines through the native line parser. These are intentional G1 compatibility
exceptions for both local-core and driver-hosted endpoints: larger stores can
now refuse; valid 4,092–16,384-byte physical lines can now succeed where the pinned
4,091-byte chunk reader refused. This broadens accepted representation only, never
which host key is trusted. Size/encoding admission refuses with InvalidInput
(mapped by the setup adapter to InvalidRequest), before DNS/TCP/credential access.
Native malformed content remains a refusal, without an alternative parser or
credential fallback. These are internal limits, not new command-line settings. Cancellation checks also bound
work between lines. Preserve native known-host-aware host-key preference so a
server offering several host-key algorithms can choose a trusted one; do not
accept a different untrusted key merely because its algorithm is preferred.

Read trust for each newly created connection. Already authenticated pooled
sessions retain their trust until disposal, as the accepted design requires.
The independently approved host-key bytes are rechecked by A2 before agent I/O.
Trust objects and native session references end before connection transfer.

## Explicit identity admission

The current invocation/per-remote/configuration precedence and endpoint-local
path resolution remain authoritative. Before every pool allocation, including
reuse, load and validate the selected regular file under supervised admission.
That admission carries the request's deadline and cancellation and happens
before pool lookup; it must not block the shared endpoint worker. Missing,
unreadable, changed or invalid selected files cannot reuse an old session.

Use an owned bounded immutable key snapshot for both eligibility and the ensuing
native authentication. Do not re-open the pathname after computing compatibility.
A private endpoint authority table may assign opaque, nonsecret tokens to exact
validated snapshots; paths/private bytes never enter pool proofs, protocol Facts
or logs. Equality of a token must imply the same validated key material. Token
lifetime is endpoint-local, never persistent; retirement cannot make an old token
refer to another key. Bound both table entries and retained secret bytes before
admission. A future implementation must specify those bounds and cleanup in its
N2 checkpoint before code is accepted.

Unencrypted file authentication uses the native in-memory private-key API under
the shared deadline. Explicit selection never enters ambient-agent enumeration.
Encrypted key prompts/exact-agent matching remain unsupported and fail closed,
as today. Native parsing/authentication must prove the snapshot is usable; a
pathname, metadata tuple or caller-supplied token alone is not proof. An optional
public-key fingerprint in Facts must be derived from proven public key material;
a private snapshot token must not be presented as a public-key fingerprint.

## Sequence and gates

1. **N1 network and trust:** implement the supervised resolver/file admission,
   cancellable nonblocking connect/handshake, and native known-host verification.
   Compose it with A2/A3 ambient-agent setup in controlled native tests. This
   removes fixture-preconnected sessions from that path. At most 350 added
   production lines across two cohesive modules and 770 focused test/support
   lines. Endpoint discovery supplies owned paths; no user environment or
   credentials are consulted by tests. Exact explicit-key admission remains N2.
2. **N2 selected authority:** specify bounded per-request admission, snapshot/token
   ownership and cleanup, native file authentication, and original deadline
   propagation through admission and pool lookup. Test unchanged/changed/deleted
   files against cached sessions, alternate explicit keys at capacity, unusable
   and encrypted files, no agent fallback, cancellation and resource retention.
   Refine concrete interfaces and limits before implementation; reuse A1/A3
   supervision. Do not insert synchronous file I/O into Route::resolve as a shortcut.
3. **N3 backend attachment:** preserve the shared endpoint through backend clones
   and nested with_transport scopes; attach operation-specific observations and
   exercise every network driver in the accepted SSH worker call-site map.
   Capability activation still requires the deferred platform/source batch.

N1 differential gates characterize native acceptance of a valid 5 MiB store and
endpoint refusal; below/at/above the 4 MiB and 16 KiB bounds; and padded comments
and host lists below/at/above the pinned 4,091-byte chunk boundary through both
readers. Differences permitted by G1 must be labelled as intentional. Every
unapproved delta refuses before TCP or credentials. N1 must also prove an agent's
first key explicitly rejected and second accepted, while handshake/terminal
authentication failure never tries another resolved address.

N1 tests must cover fresh native connection/authentication/Git exchange and reuse,
unknown/mismatched/missing trust before authentication, nondefault port, hashed
host entry, multiple server host-key algorithms with only one trusted, malformed/
oversized/nonregular trust, unavailable TCP peer, stalled handshake cancellation
and timeout, sequential address handling, and late resolver/file completion after
cancellation. Injected OS stalls prove ownership and suppression of subsequent
effects, not kernel cancellation. Existing native A1–A3 tests remain the focused
regression gate. Use fixture keys and controlled loopback peers only.

This draft receives retained Consistency/Safety review. N1 receives retained
Code/State review on its exact committed implementation/evidence tuple. P0–P2
block; at most two merged remediation rounds per object. Public commands and wire
shapes remain unchanged. Raw runs belong in the private ssh-integration evidence
campaign, with failed attempts, commands and exact source fingerprints retained.

## Design remediation 1

Consistency P2-1 and Safety P2-1 independently found trust-input compatibility
changes hidden by a preservation claim. G1 now explicitly authorizes both bounded
refusal and complete-line admission, with exact limits, encoding, error class,
placement coverage and native differential gates. No change to cryptographic
trust or credential fallback. Consistency P2-2 separates whole-connection replay
from accepted A2 ordered key progression. Retained Consistency/Safety GO close all P2 findings. Consistency P3-1 is
corrected here by explicitly ordering trust admission before name resolution.

## Design acceptance

Accepted at root `eadf8dc25f93b3f8d9d9c4f3660732367861559f`, core
`9acf508aefe4ef974e52f19016f33ecf4bf56b34`, transport
`28f5afb3938a2aa8af0e1e8d5b07779add6ab776` after retained
[Consistency GO](../../dev-docs/GwzRemoteTransportSshProductionSetup-ReviewConsistency-1.md)
and [Safety GO](../../dev-docs/GwzRemoteTransportSshProductionSetup-ReviewSafety-1.md).
Two Consistency P2s and one Safety P2 close in one merged remediation; the trust
compatibility boundary was independently identified by both axes. Nonblocking
Consistency P3-1 operation-order wording is corrected in this annotation. N1
must assert zero resolver calls for rejected size/encoding. Design acceptance
authorizes N1 implementation, not capability or production activation.

N1 test ceiling refinement: N2 authority and N3 backend work remain excluded.
The original650-line test allowance moves to660 for native whitespace and
key-type-prefix parity regressions; final tests652 lines. Production remains
348/350 lines in one module. This adds no behavior beyond the stated native
parity boundary. Implementation acceptance is recorded in GwzRemoteTransportSshN1.md.

N1 remediation test allowance: N2/N3 remain excluded. The reviewed connect-error
and CR-token counterexamples require deterministic connector and terminal-Control
regressions, a separate native ending/comment matrix and CRLF cap checks. The
660-line allowance is refined to770; final tests764. Production remains within
350 lines/one module. The added seam does not change the setup or wire API.

## N2 concrete refinement (accepted design)

GwzRemoteTransportSshSelectedIdentityDesign.md specifies bounds, ownership and
native in-memory authentication. Its retained Consistency/Safety acceptance
refines the earlier
“validated snapshot before allocation” wording: representation/current file
admission precedes lookup; exact same-Key snapshots share a canonical token even
while unproven, and reuse additionally requires a matching authenticated reusable
physical resource. A new token for different bytes cannot match an old connection;
a candidate becomes proven only after joined live native authentication. It also extends the retained
endpoint owner to cover admission Jobs before pool entries exist. The new file
and aggregate bounds are explicitly reflected in G1; no public/wire API changes.
