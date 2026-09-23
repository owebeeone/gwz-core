# Typed connection setup failure causes

Status: **draft; not implementation authority**. This bounded amendment needs
review before code changes. It corrects the failure-cause carrier required by
`GwzRemoteTransportRetryPlan.md` at SHA-256
`08e198e00c5f6ff697dca6b71f8117ce8963afb91126a30af2b5ea94a6ac6619`.

## Problem and boundary

The retry classifier must distinguish a setup stall from an interaction or
allocation timeout, and `ConnectionRefused` from `NotFound` and
`AddrNotAvailable`. A code alone cannot do that: SSH setup maps all three I/O
kinds to `Unavailable`, and the pool currently reduces a connection `Failure`
to only `(code, effect)` in `Error::ConnectFailed`. The current timeout path
temporarily writes `stall` or `aggregate` into `Facts.key_fingerprint`. A
fingerprint field must contain only a real host-key fingerprint; removing the
value before a transport observation does not make that overloading safe.

## Correction

Add an optional typed `setup_cause` field (key 4) to the transport taut
`Failure` message in `gwz-transport/protocol/transport.taut.py`. Define a
`SetupFailureCause` enum with `stall`, `aggregate`, `interaction`,
`allocation`, `connection_refused`, `not_found`, and
`address_not_available`. The field is `optional=True, missing_ok=True`.
It is set only for a failed connection setup, never for a successful `Opened`
or a fetch/push body failure. It contains no address, path, fingerprint,
credential, or native error string. `Facts.key_fingerprint` retains its
existing sole meaning.

The endpoint stamps the cause at the earliest typed decision. The SSH
`agent_job` and `ssh_setup` paths stamp `stall` or `aggregate` from
`TimeoutReason`; the pool stamps `aggregate`, `interaction`, or `allocation`
where its respective clock expires. SSH and HTTPS connector I/O mappings preserve the original
`io::ErrorKind` for `connection_refused`, `not_found`, and
`address_not_available` before mapping to a public `ErrorCode`. HTTPS
connect errors preserve the final attempted TCP error kind instead of
collapsing it to `Io`; DNS and setup errors retain their own kind. Other
setup errors leave `setup_cause` absent. `ErrorCode`, `Effect`, and their
existing meanings do not change.

Carry the optional cause through `gwz-transport::pool::Error::ConnectFailed`
and both SSH/HTTPS adapters, then through `OpenFailed(Failure)` to the core
request. The transport pool still decides only allocation and cleanup; it
does not decide retry eligibility. The one classifier in `gwz-core` accepts
`(code, setup_cause, before_reusable)` and retries only:

- `Timeout` with `stall` or `aggregate`;
- `Unavailable` with `connection_refused`;
- `Io` or `CarrierLost` while setup is still in progress.

An absent cause cannot make `Timeout` or `Unavailable` retriable. An
`interaction` or `allocation` cause is never retriable. The operation's
per-key machine supplies the before-reusable fact; no operation body is
replayed. A key closed by a non-retriable failure stays closed for that
operation. Display renders `ssh setup timeout: stall` or `ssh setup timeout:
aggregate` from the typed cause, never from text parsing.

This is endpoint-local cause classification carried over the existing
logical failure message. It adds no physical carrier, process global,
secret-bearing field, error code, or transport profile version. It does not
change the independent stall and aggregate clocks.

## Compatibility and authority

The existing transport CBOR decoder reads fields by integer key and ignores
unrecognized map keys. An older reader accepts a new writer's key 4 and
retains the old code/effect/facts behavior; if it re-encodes the failure, it
may drop the unknown cause. A new reader accepts an older writer's missing
key 4 as `None`. The classifier then fails closed for ambiguous `Timeout`
and `Unavailable`. A malformed or unknown enum value remains a decode
error. The profile remains v2, and the GWZ request schema remains v0.

This amendment supersedes only the `GwzRemoteTransportAlphaTimeoutPlan.md`
§5 bullet excluding a new schema field or message: one optional `Failure`
field is now permitted for typed setup causes. No new CLI flag or message is
permitted. It also specifies the cause carrier left implicit in
`GwzRemoteTransportRetryPlan.md` §§4–5; all retry sets, budgets, states,
defaults, and release exclusions there remain unchanged. The separate
optional `max_retries` field on GWZ `OperationPolicy` remains as planned.

## Implementation and verification gate

Regenerate `gwz-transport/src/protocol.rs` from its taut source using the
pinned generator; do not hand-edit generated output. Update all affected
`Failure` literals and pattern matches, including the pool's
`ConnectFailed`, without inventing fallback causes. The touched source
areas are `gwz-core/src/git/endpoint/{agent_job,ssh_setup,ssh_worker,
https_connection,https_pool}.rs` and
`gwz-core/src/transport_host/session/driver.rs`, plus the core retry
classifier. The transport pool's `allocation.rs` and `mod.rs` preserve the
cause through failed checkout.

Tests pin: each timeout origin, all three distinguished `Unavailable`
causes, absence for generic `Io`, old/new CBOR reader compatibility, and
that `Facts.key_fingerprint` is unchanged by timeout handling. An older
generation golden `Failure` still decodes with no cause. SSH and HTTPS
integration tests assert the same classifier result before `Opened` and no
retry after `Opened`. The transport and core standalone test gates and
source-pinned generation checks remain required by their own repositories.
