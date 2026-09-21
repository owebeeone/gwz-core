# SSH N1 — native connection and host trust

Date:2026-09-21. Status: implemented locally; retained Code/State review pending.
Authority: accepted GwzRemoteTransportSshProductionSetup.md N1, amended G1,
accepted A1/A2/A3. No production routing/dependency or public wire activation.

## Implementation

ssh_network.rs runs entirely within an existing supervised Job. It loads a
regular endpoint-local trust file using O_NONBLOCK and opened-descriptor checks,
admits bounded UTF8/NUL-free complete lines before DNS, retains32 resolved
addresses, connects sequentially with nonblocking socket2, and handshakes using
native libssh2 under the shared cancellation/deadline Control. Poll waits are at
most20ms. No address replay occurs after SSH negotiation begins. Blocking OS
resolution/file calls remain subject to the accepted retained-owner admission;
no kernel cancellation or hard physical termination bound is claimed.

Native known-host parsing/check_port retains logical-host/effective-port/hash
matching. Separate native sets select trusted host-key algorithms in the pinned
libgit2 preference order. Filter tokenization/prefix recognition matches pinned
hostline; no handwritten hostname matching or new C binding. The approved host
key and exclusive connection transfer together to A2, which rechecks trust
before opening the bounded agent. All native known-host owners end before handoff.
N1 does not authenticate directly or open a Git command itself.

Errors are sanitized io kinds. Size/encoding admission uses InvalidInput, which
A3 maps to InvalidRequest. Missing/malformed/unknown/mismatched trust refuses.
The new whole-line and bounded-file behavior is an explicit G1 exception, not a
claim of byte-for-byte native input parity. Native whitespace and key-type
classification quirks remain preserved outside those enumerated exceptions.

## Evidence and limits

[Private raw evidence](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-21-network-n1/README.md)
retains failed attempts, exact commands and final source fingerprints. Full
isolated Rust1.95 locked/offline suite passes (88 executed tests). Fifteen tests
in the network binary include14 new cases and the retained worker queue test.

Coverage: fresh native setup→agent authentication→pooled Git stream reuse;
rejected first/accepted second agent key; hashed and nondefault-port trust;
unknown/mismatched/missing trust; malformed/oversized/nonregular files; actual
loader UTF8/NUL/size refusals before resolver; total/line boundaries and native
4091-byte differentials; multi-key peer selection; cancelled/timed-out handshake
with socket closure; sequential address refusal without post-negotiation replay;
terminal auth refusal without another address; late loader/resolver cancellation
with retained disposal. Injected stalls are not claims of actual kernel stalls.

Owner TDD found two semantic parity defects before review: broad Rust whitespace
trimming admitted a native-rejected bare vertical-tab line, and exact token
classification omitted native-recognized key-type prefixes. Both have red/green
native regressions. Earlier enum/IP compilation errors and the initial incorrect
pseudo-comment test hypothesis remain recorded. No known escaped defect.

348 production lines/one file,652 test lines/one file. Test ceiling refined to660
for the parity regressions; production remains within350. N2 explicit snapshot
admission and file-key authentication, N3 backend observation/routing attachment,
and the deferred platform/source batch remain required. No general production
credential setup or advertised endpoint capability is accepted by this slice.

Aggregate tier: retained Code/State on exact committed tuple; P0–P2 block;
at most two merged remediation rounds. Review and acceptance results follow.
