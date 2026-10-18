# User-facing surface proposed by the GWZ adaptive concurrency design (revision 9), extracted verbatim

Source: gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign.md, sha256 8354166a9a355cf694b35bbeb853978950cb7cdadc47cb16ea6c5b719ff8f3b7. Extracted by the lane owner for the Surface re-check. Nothing here is implemented yet.

## A. What the user sees (design §9), verbatim
## 9. What the user sees

- **Notes**, human mode only, on stderr, prefixed `note: ` (the `gwz: ` prefix is reserved for failures: `gwz: <Code>: <message>`). They print whether or not stderr is a terminal, between progress lines when progress is shown; only `--json` and `--jsonl` suppress them (no network command has `--quiet`). One line per event, never per member:
  - **On an Overload** (a conclusive Throttle, or a Suspect confirmed by its test), once per host, with `N` once the wave that caused it has resolved (§4.4 R3): `note: github.com refused extra connections; using 8 at a time (ceiling 32, --max-per-host); continuing, no action needed`.
  - **When the limit changes later**, coalesced to at most one line per host per 2 s: `note: github.com: now using 12 connections`.
  - **When the machine reaches SATURATED again**: `note: github.com: back at the ceiling of 32 connections`.
  - **When a host goes Down and members park** (§5.5), once per Down episode: `note: github.com:22 is unreachable; waiting up to 30 s to retry`.
  - An inconclusive refusal, a refuted Suspect, a hold, a settle and a test print nothing, so a one-off failure never produces a note. A Queue wait prints nothing.
- **The final error** of a member: §5.3's table, one row per cause, each with one remedy.
- **JSON.** A default `--json` payload is unchanged byte for byte. A failed member carries its message in its error row, and the optional `capacity` detail on the failure for consumers that read it (§5.3).
- **Docs that ship with it** (the same change as the code):
  - `--max-per-host` help gains: "gwz may use fewer connections when the host refuses more, and raises the number again when it recovers; a note says so."
  - `--max-retries` gets its own option entry: its default (3 extra attempts, so 4 in all), that it also bounds the retries of throttled connections, and that `--max-retries 0` turns off the adaptive limit (no refusal lowers it).
  - `docs/Troubleshooting.md` gains a section, "Fewer connections than --max-per-host, or a Capacity error", showing the notes and the §5.3 messages with their remedies.
  - `docs/MachineOutput.md` documents the `Capacity` code, the `capacity` detail with each `cause`, and the `retry_attempt` "attempt N of M" suffix.
- **TR2.24.** The connection statistics proposal's `meta.transport_diagnostics`, under `--verbose`, is the natural home for per-host `max_per_host`, `limit_applied`, `overloads`, `inconclusive`, `tests` and `held_ms`. This design only names them; it does not design TR2.24's fields or payload.
- **`--verbose` human lines** may show the limit's history. Not designed here.

## B. The final error (design §5.3), verbatim
### 5.3 The member's budget, and what it finally reports

A member's attempts, retries, throttle requeues and **the tests it carries** together are bounded by `--max-retries + 1`, 4 by default (the operator's rule: requeues count against the existing retry budget). An attempt is a connection the member started (or an exchange on a leased connection that was refused); one that was only held back, and never started, is not an attempt. An inconclusive refusal and an unfair test are attempts. A probe test is never given a member's final attempt; a confirming test is, only when no other member can carry it (§4.7, §4.5 rule 3).

What a member finally reports depends on why it failed. **One table, every case** (Surface review P2-1):

| Case | Code | Message (human row, after the existing `member '<id>' at '<path>': ` prefix) | Detail |
|---|---|---|---|
| **Server throttle:** the member's budget ran out while the key had a Throttle or a confirmed overload during its attempts | `Capacity` | `github.com:443 kept refusing connections (8 at a time accepted, ceiling 32); gave up after 4 of 4 attempts over 21 s; retry later, or lower --max-per-host` (the parenthesis is omitted when the limit was never lowered) | `capacity` with `cause: "server_throttle"`, and `retry_attempt` |
| **Long `Retry-After`:** longer than the 30 s a command holds a host | `Capacity` | `github.com:443 asked to wait 120 s, longer than the 30 s one command holds a host; retry in 2 min` | `capacity` with `cause: "server_retry_after"` and `retry_after_ms` |
| **Local capacity:** a local permit wait expired (the 64-job budget) | `Capacity` | `local connection slots stayed full for 30 s (64 jobs in use); lower --jobs` | `capacity` with `cause: "local"`; no host |
| **A refusal with no throttle evidence,** at any `--max-retries` (a reset, a refused connect, a stall, with no 429 or 503 and no confirmed overload on the key) | unchanged: the code and message it reports today | unchanged | unchanged (`retry_attempt` as today) |
| **A Down key's member that made no attempt** (§5.5, after two failed retests) | the recorded failure's code | `not attempted: github.com:22 is unreachable (4 of 4 attempts failed earlier in this run)` | no `retry_attempt`: this member made none |
| **A failed retest** of a Down key | the retest's own failure code | its own message, with the suffix `still failing after a retry 30 s later (4 of 4 attempts failed earlier in this run)` | `retry_attempt` omitted |

- **`Capacity`** is unchanged as a code, so an older reader still renders something true, and it never stands for a plain refusal. The `cause` field says which of the three it is, so a script never has to read the message.
- **The optional detail field is named `capacity`**, after the code it explains, and added to `FailureDetail` beside `retry_attempt` (`gwz-transport/src/protocol.rs:851-857`): `{cause, scheme, host, port, limit_applied, max_per_host, elapsed_ms, last_signal, retry_after_ms}`.
  - `cause`: `server_throttle`, `server_retry_after` or `local`, always first.
  - `limit_applied`: the per-host connection limit gwz was using when the member gave up (gwz's adaptive limit, not a number the server stated).
  - `max_per_host`: the ceiling, the resolved `--max-per-host` (default 32), named after the flag.
  - `last_signal`: `http_429`, `http_503`, `refused`, `reset`, `setup_timeout` or `local`; `retry_after_ms` only with a `Retry-After`.
  - Attempt counts are **not** repeated here: `retry_attempt` (`{attempt, attempts}`, "attempt N of M", `M` = `--max-retries + 1`) carries them, once.
  - `host` and `port` are the **member's configured** host and port; when the pool key differs (a cross-host redirect, `https_destination.rs:120-153`, `prepare.rs:192`), they are omitted, since HTTPS design §7 never shows a redirect target. Counts and milliseconds only; never a URL, a response body, or a credential. An optional field is additive, where a new `ErrorCode` or `SetupFailureCause` value is not: both decode an unknown value as `DecodeError::UnknownEnum` (`protocol.rs:240-336`), which would break an older peer.
- **`--max-retries 0`:** a refusal fails its member at once, consistent with "zero means the first failure is the member result" (retry plan §4), with the table's code for its case: a 429 or a 503 with `Retry-After` is a server throttle (`Capacity`), anything else keeps its code. No probe test can ever be carried (every attempt is final), so **no refusal lowers `N` and no pre-authentication drop lowers `Ns`**, and no confirmation opens: a decrease could never be checked, and the operator's direction forbids an untested one. A `Retry-After` still sets the hold. A Queue wait still waits.

The worst-case bound for a member is its `R + 1` attempts, each bounded by its own setup clocks (retry plan §5's `(R + 1) x aggregate`), plus holds of at most 30 s each, plus quiet waits (§5.2), plus settle waits of at most `Ts` each, plus the jitter, plus waits behind occupants whose own clocks bound them. No wait is unbounded.

## C. The attempt suffix after a Down key's retest (design §14 item 8), verbatim
8. **§5, lines 280–281:** "`N` is the attempt that just failed." Compatible for the first `R + 1` attempts: the counting is still per key, and a Down key's finished members report the recorded failure's `attempt N of M`. *Addition:* a failed retest reports no attempt number above `M`; its message carries `still failing after a retry 30 s later (M of M attempts failed earlier in this run)`, and a member that made no attempt reports `not attempted: …` with no `retry_attempt` (§5.3's table).

## D. Behaviour a user can observe, summarised by the lane owner (not design text), unchanged from the revision-8 extract
- `--max-per-host` (default 32) is a ceiling; the transport may use fewer connections per host when a server refuses more, and keeps testing to climb back.
- `--max-retries` (default 3) bounds each member's attempts (R + 1); refusals caused by a server throttle are retried within that budget and count against it.
- When a host is unreachable after its retries, members that select it wait up to 30 s for a retest; after two failed retests in a row, later members fail at once with the recorded failure until a retest succeeds.
- No new flags, no new configuration keys, no new JSON/JSONL fields in default output; one optional field (`throttle`) in a failure's detail on the wire.
