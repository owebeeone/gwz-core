# User-facing surface proposed by the GWZ adaptive concurrency design (revision 8), extracted verbatim

Source: gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md, sha256 0d8d9be981807be51d55e2be205e9dc97e6f95a0db86293aa8450c0f529e0872. Extracted by the lane owner so the Surface reviewer can read only what a user would meet. Nothing here is implemented yet.

## A. The note (design §9), verbatim
## 9. What the user sees

- **A note**, human mode only, on stderr, **only on an Overload** (a conclusive Throttle, or a Suspect confirmed by its test): `gwz: github.com: limited to 8 concurrent connections (32 requested)`. The host is the member's configured host, never a redirect target (§5.3). The value is `N` once the wave that caused it has resolved, not the first value set (§4.4 R3). An inconclusive refusal, a refuted Suspect, a hold, a settle and a test print nothing, so a one-off failure never produces a note. It is printed once per host, not per member. Later changes of `N` are coalesced, at most one line per host per 2 s carrying the latest value (`gwz: github.com: limit now 12`), and a final `gwz: github.com: limit lifted` when the machine reaches SATURATED. A Queue wait prints nothing. Absent in `--json`, `--jsonl` and `--quiet`, and no JSONL event, the same rules as the transport setting's note (its §4 and §5, `merge_render.rs:245-251`).
- **JSON.** A default `--json` payload is unchanged byte for byte. A failed member carries the throttled message in its error row, and the optional `throttle` detail on the failure for consumers that read it (§5.3).
- **TR2.24.** The connection statistics proposal's `meta.transport_diagnostics`, under `--verbose`, is the natural home for per-host `limit_configured`, `limit_observed`, `overloads`, `inconclusive`, `tests` and `held_ms`. This design only names them; it does not design TR2.24's fields or payload.
- **`--verbose` human lines** may show the limit's history. Not designed here.

## B. The final error (design §5.3), verbatim
### 5.3 The member's budget, and what it finally reports

A member's attempts, retries, throttle requeues and **the tests it carries** together are bounded by `--max-retries + 1`, 4 by default (the operator's rule: requeues count against the existing retry budget). An attempt is a connection the member started (or an exchange on a leased connection that was refused); one that was only held back, and never started, is not an attempt. An inconclusive refusal and an unfair test are attempts. A probe test is never given a member's final attempt; a confirming test is, only when no other member can carry it (§4.7, §4.5 rule 3).

When the budget runs out, a `Retry-After` is longer than 30 s (the members parked on the key cannot wait it out, and nothing is sent to the host before it has passed), or a local permit wait expires, the member fails with:

- code **`Capacity`** (unchanged, so an older reader still renders something true), and
- an optional `FailureDetail` field, `throttle`, added beside `retry_attempt` (`gwz-transport/src/protocol.rs:851-857`): `{scheme, host, port, limit_observed, limit_configured, attempts, attempts_max, elapsed_ms, last_signal}`, where `last_signal` is one of `http_429`, `http_503`, `refused`, `reset`, `setup_timeout`, `local`, and an optional `retry_after_ms`. `host` and `port` are the **member's configured** host and port; when the pool key differs (a cross-host redirect, `https_destination.rs:120-153`, `prepare.rs:192`), they are omitted, since HTTPS design §7 never shows a redirect target. It carries counts and milliseconds; never a URL, a response body, or a credential. An optional field is additive, where a new `ErrorCode` or `SetupFailureCause` value is not: both decode an unknown value as `DecodeError::UnknownEnum` (`protocol.rs:240-336`), which would break an older peer. This follows the stats design's D5 and TR1.6's detail (the retry plan's own `retry_attempt` is the precedent).
- The message the user sees: `github.com:443: throttled: the server limited this command to 8 concurrent connections (32 requested); gave up after 4 attempts over 21 s`. For a local expiry: `local setup capacity exhausted for 30 s (64 supervised jobs in use)`. For a `Retry-After` beyond the cap: `… asked to wait 120 s (longer than the 30 s this command holds a host)`.
- **`--max-retries 0`:** a refusal fails its member at once with this error, consistent with "zero means the first failure is the member result" (retry plan §4). No probe test can ever be carried (every attempt is final), so **no refusal lowers `N` and no pre-authentication drop lowers `Ns`**, and no confirmation opens: a decrease could never be checked, and the operator's direction forbids an untested one. A `Retry-After` still sets the hold. A Queue wait still waits.

The worst-case bound for a member is its `R + 1` attempts, each bounded by its own setup clocks (retry plan §5's `(R + 1) x aggregate`), plus holds of at most 30 s each, plus quiet waits (§5.2), plus settle waits of at most `Ts` each, plus the jitter, plus waits behind occupants whose own clocks bound them. No wait is unbounded.

## C. The attempt suffix after a Down key's retest (design §14 item 8), verbatim
8. **§5, lines 280–281:** "`N` is the attempt that just failed." Compatible for the first `R + 1` attempts: the counting is still per key, and a Down key's finished members report the recorded failure's `attempt N of M`. *Addition:* a failed retest reports `retest after attempt M of M`, not an attempt number above `M`. Its wording is for the Surface review the status line requires.

## D. Behaviour a user can observe, summarised by the lane owner (not design text)
- `--max-per-host` (default 32) is a ceiling; the transport may use fewer connections per host when a server refuses more, and keeps testing to climb back.
- `--max-retries` (default 3) bounds each member's attempts (R + 1); refusals caused by a server throttle are retried within that budget and count against it.
- When a host is unreachable after its retries, members that select it wait up to 30 s for a retest; after two failed retests in a row, later members fail at once with the recorded failure until a retest succeeds.
- No new flags, no new configuration keys, no new JSON/JSONL fields in default output; one optional field (`throttle`) in a failure's detail on the wire.
