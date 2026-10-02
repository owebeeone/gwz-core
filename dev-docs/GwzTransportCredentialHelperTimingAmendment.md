# TR2.22 helper context amendment

Date: 2026-10-03. Status: **DRAFT; not accepted or implemented**.
The coordinator owns settlement and independent amendment review.

## Problem and controlling text

TR1.6 revision 4 §11 requires M4 to name the interaction allowance actually
applied to a lookup and M10 to name the retained allocation allowance at helper
admission. OQ5(a), OQ6(a), OQ7(1) and the 120-second bound are already accepted.
This amendment supplies numeric provenance needed to render those messages
and an explicit HTTPS username carrier for already accepted OQ1(b). It changes
neither authentication policy nor retry clocks/outcomes.

The accepted wire checkpoint is core `26922a1cfd823a4894e09be9aeb05f7f73d21414`
and transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Its closed
`FailureDetail` has only M8 cause/pipe kind, M6 scheme tokens and retry counts.
No field identifies a helper timeout or its allowance. A bare
`Timeout`/`Allocation` also names unrelated pool/stream admission timeouts.

The driver knows the Open it sent, including its interaction bound. Its
allocation observer in `session/driver/opening.rs` reports the Open's initial
allocation value, not what anonymous discovery or endpoint admission consumed.
The retained budget belongs to `https_worker::Budget` and the endpoint's
carried `Retry`; it is not available to the renderer. Reconstructing it from
wall-clock elapsed time would include network, helper and retry waits that the
accepted clock excludes. Therefore the existing seam cannot truthfully select
M10 and report its retained allowance, including with an endpoint in another
process. M4 also needs helper provenance so it cannot be confused with another
interaction timeout.

## Exact bounded timing addition

Add **one** optional missing-or-null integer to the authored taut
`FailureDetail`: `helper_budget_ms`, tag **5**. Its presence identifies a
configured-helper timeout. The existing `Failure.setup_cause` identifies its
phase; no additional enum, string, URL or credential field is introduced.

Validator rules, on every Failure carrier through both admission and decode:

- The enclosing failure has code `Timeout` and effect `None`.
- With setup cause `Interaction`, `1 <= helper_budget_ms <= 120_000`.
- With setup cause `Allocation`, `0 <= helper_budget_ms <= 86_400_000`.
- Any other cause, absent cause, negative/out-of-range value, or non-timeout
  code refuses the message.
- This field cannot coexist with `helper_cause`, `pipe_kind` or `schemes`.
  The existing valid endpoint retry count may coexist.
- Absence preserves the accepted wire shape and generic timeout rendering.
  An older writer's timeout is never relabeled as a helper timeout by inference.

The endpoints record the exact integer duration they apply to the timer:

- **M4:** the fixed 120,000 ms interaction allowance, shortened by a positive
  smaller Open interaction allowance, captured after helper admission and
  immediately before spawning git. Slot waits do not consume it.
- **M10:** the retained allocation allowance at the beginning of the two
  consecutive helper-admission waits (endpoint permit, then host slot). Both
  waits use one allocation deadline. Capture it before either wait; do not
  report the zero remainder at timeout as the allowance that was available.
  A budget already exhausted is represented by zero and starts no helper.

The interaction limit comes from TR1.6 §3.3's fixed bound, not from a claim
that every admitted Open has that limit. In the accepted source,
transport `codec/validate.rs` admits any positive i64 allocation/interaction
deadline, and `pool::Config::validate` permits resource durations up to
86,400,000 ms. `Client::budget_for_config` constructs allocation from that
validated configuration; `budget_for_open`, retained anonymous work and queue
charging only shorten it. Thus **86,400,000**, not the 30,000 ms default, is
the current producer's proven allocation ceiling. No new allocation cap is
introduced. The production driver's Open uses 30,000/120,000 ms and the
endpoint shortens them. The helper's interaction allowance is
`min(configured_interaction_ms, positive_Open_interaction_ms, 120_000)`.
Omitted caller shortening leaves the configured/fixed bound. That interaction
cap applies to helper work only; it does not change a pool's or network's own
deadline. Helper admission receives the actual retained allocation allowance.
The producer uses those same captured millisecond values for its timers and
its detail, so rounding does not invent a duration. The renderer converts
milliseconds to seconds exactly: an integer when divisible by 1,000, otherwise
a decimal with up to three fractional digits and trailing zeroes removed.
For example 120,000 becomes `120`, 1,250 becomes `1.25`, and 1 becomes `0.001`.
This fills existing `<n>`/`<m>` slots. M4's wording remains unchanged. To avoid
calling a configurable allowance a fixed 30-second bound, M10's first clause
is precisely amended to: "No credential helper could start in the <m> seconds
available to wait for resources:". The remainder of M10 stands. Its
driver default remains 30 seconds; the amendment introduces no new policy or
wait duration. TR1.6 §11's statement that `<m>` is at most 30 is qualified as
the production driver's default, not a universal typed-Open bound.

## One missing fixed parser cause

TR1.6 §9.1's replacement of HTTPS lines 148–152 explicitly rejects duplicate
or malformed recognized fields. Its seven M8 cause strings cannot truthfully
describe that refusal when both fields exist. Add the single enum alternative
`HelperFailureCause.malformed_output = 8`, displayed as **"its answer has a
malformed or duplicate username/password field"** in M8's existing cause slot.
It carries no helper text and does not change parser acceptance. Unknown lines
remain tolerated; recognized fields remain strict. Existing missing-field,
control, colon, encoding, newline and pipe/limit alternatives keep their values.

## Exact bounded HTTPS username addition

Add `https_username` to authored taut `Destination`, tag **6**, optional
missing-or-null **STR**. `ssh_username` at tag 5 keeps its SSH-only meaning.
This chooses an explicit separate field instead of OQ1(b)'s suggested widening
or renaming of the existing SSH field; the accepted username behavior stands.

The value is the nonempty percent-encoded username component returned by the
validated parsed URL's `username()`, copied unchanged. It is never a decoded
username, a password, a whole URL, or an authentication header. Absence means
no URL username; an explicitly empty field is invalid. Its byte length is at
most **18,000**, the existing maximum complete HTTPS input URL. The producer
and endpoint also require the complete reconstructed base URL, including its
authority and path, to stay within that existing 18,000-byte bound.

Validation at transport admission and encoded decode:

- The field is absent for SSH and permitted only for HTTPS.
- A present value is nonempty ASCII serialized userinfo, at most 18,000 bytes;
  no raw controls, whitespace, `@`, `:`, `/`, `?`, `#` or backslash is allowed.
  A percent sign followed by two hex digits decodes only for the refusal check;
  any other percent sign remains literal, as Git's existing URL decoding does.
  No decoder normalization or decode/re-encode changes the stored text.
- Both the encoded username and repository path refuse percent-decoded control
  characters. Decode valid UTF-8 to inspect Unicode control characters; for
  invalid UTF-8, reject every decoded ASCII control byte. Never print the
  decoded data. Global codec metadata/allocation limits remain additional
  bounds, not superseded limits.
- Driver-side HTTPS shape validation rejects these controls before submitting
  Open. Endpoint revalidation rejects malicious or invalid carried Opens
  before helper/network effects. Redirects reach the same shape check before
  following their Location.

`Session::open_https` produces the field from its validated URL. The endpoint
forwards it unchanged into the reconstructed helper base URL; a `%3A` username
remains `%3A` in the one `url=` input line. No separate `username=` input line
is emitted. Git alone decodes it and selects configured helpers. Request URI,
Host, Authorization, facts, Failure detail and diagnostic text never derive a
username from this field. Any request URL serialization first removes userinfo.
The route's private URL selector may retain this component to keep account
selection isolated, but is not logged or reported.

Generic raw-envelope Debug/pretty-printing is not an authorized diagnostic
path. The transport's generated Debug representation, where applicable, must
redact the new field; regression checks cover local/generated wrapper debug
representations and reject any copy into error formatting. Public application
GWZ request/response schemas remain unchanged.

## Precisely superseded clauses

This DRAFT supersedes only these representation restrictions if accepted:

1. TR1.6 revision 4 OQ7(1)'s exclusive Failure-detail vocabulary: add the single
   bounded helper budget integer and fixed malformed-output cause above; keep
   all prohibitions on helper output,
   stderr, URL, username and header content. Its §9.4 C10 and the accepted wire
   checkpoint's four-field representation are extended by this same field.
2. TR1.6 revision 4 OQ1(b)'s suggested carriage in the SSH-only destination
   field: use `Destination.https_username`, tag 6, instead. Its encoded URL-only
   input, control refusal, password refusal and no-request-userinfo rules stand.
3. The accepted authored Destination declaration and HTTPS codec shape: allow
   only the separate bounded optional HTTPS username; `ssh_username` remains
   forbidden on HTTPS. No other scheme/policy combination changes.
4. TR1.6 §11 M8's seven-cause list: add exactly the fixed malformed-output
   phrase above, preserving §9.1's parser refusals. TR1.6 §11 M10's first clause
   and its universal 30-second ceiling: use the precise configurable-budget
   qualification above. All other wording and code/clone outcomes stand.

The older HTTPS design's §5 no-userinfo sentence is already superseded for
usernames by TR1.6 §9.1's OQ1(b) contingency. This amendment supplies the
carrier; it does not supersede that policy again. No public application schema,
error-code number, clone suppression rule or secret policy is changed here.
Only the exact M8 cause and M10 clause named above change message wording.

## Affected owners and checks

Transport: `protocol/transport.taut.py`, regenerated Rust/IR/admission artifacts,
`src/codec/failure_detail.rs`, destination shape validation, and focused
detail/carrier tests. The exact boxed Rust projection remains unchanged.
Generated username Debug redaction is a reproducible fail-closed projection,
not a handwritten generated edit. Normal frame and allocation bounds remain.
No retained fixture is edited.

Core: helper admission and lookup outcomes; `https_worker` budget/Failure
producer; `transport_host::HttpsOpenFailure`'s shared typed renderer, used by
both local and carried endpoint outcomes; `Session::open_https`'s Destination
producer, HTTPS endpoint reconstruction, redirect validation and account route
isolation. Current same-build Destination literals in core, transport and the
archive consumer gain explicit `https_username: None`; immutable retained
readers stay unchanged and old missing fields still decode as absence.
The public error-code allocation
already accepted by OQ5(a) remains separate coordinated generation work; this
amendment allocates no error code.

Regression rows:

1. Old absent detail retains generic timeout text; no inferred helper message.
2. Round-trip both valid phases and exact 1,250 ms decimal rendering.
3. Reject wrong code/effect/cause, forbidden mixed detail, zero interaction,
   negative values, 120,001 interaction and 86,400,001 allocation, through local
   admission and encoded decode on every Failure carrier.
4. Anonymous discovery consumes part of 30,000 ms; queued helper timeout names
   that retained allowance, starts no helper and sets no latch.
5. Endpoint-permit wait followed by host-slot wait uses the same deadline and
   reports their initial allowance; a started helper receives its full bound.
6. A general connection/pool allocation timeout remains generic.
7. A URL username such as `a%3Ab` reaches a recording Git helper as Git's one
   username selector, with no password selector; the wire preserves the encoded
   component. URI/Host, Failure text/detail and Debug diagnostics contain neither
   the username nor credentials.
8. Reject password userinfo, SSH carrying the new field, empty/oversize/forbidden
   encoded username, decoded C0/DEL/Unicode controls and total URL overflow.
   Assert driver refusal before Open and endpoint refusal before effects,
   including a redirect's Location and carried encoded Opens.
9. Duplicate username/password, and malformed recognized fields, use the one
   fixed malformed-output cause; unknown lines remain ignored. A larger valid
   configured allocation allowance round-trips and is rendered truthfully,
   without changing the default driver's 30-second wait or adding a new cap.

No new free-form diagnostics, clocks, retry transition, authentication policy,
Windows activation or timing campaign is authorized. Root must settle this
DRAFT and obtain independent Consistency/Safety review, with Surface for the
typed API/message shape, before implementing either new wire field. The runner
may proceed using existing accepted contracts. This one bounded amendment owns
both context gaps; no second serial interface freeze is proposed.
