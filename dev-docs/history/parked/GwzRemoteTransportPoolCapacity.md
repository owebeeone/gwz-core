# Remote transport pool capacity: align with operation concurrency, then multiplex channels

Status: BRIEF for an implementing agent. Nothing here is done. Written 2026-09-23 from measurements in
`garnets-wz/dev-docs/gwz-go-to-market/research/gwz-fetch-scale.md` (private; the numbers are copied below).

## Why

No-op `gwz fetch` over SSH, 32 small github.com repos, medians of three runs:

| setting | gwz 1.0.17 (new connection per exchange) | gwz-alpha 0.2.0-alpha.transport (pool) |
|---|---|---|
| defaults (`--jobs 50 --max-per-host 8`) | 11.3 s | 7.5 s |
| `--max-per-host 32` | **3.1 s** | 6.9 s |
| `--max-per-host 4` | 19.3 s | 8.5 s |
| `--jobs 1` | 77.1 s | 31.5 s |

GitHub accepted 32 concurrent SSH connections from one account with no refusal. The pool wins whenever
concurrency is constrained and loses to plain parallel connections when it is not. Reliability went the
other way: 1.0.17 at 16 members exited 1 in 2 of 5 runs (`failed to start SSH session: Timed out waiting on
socket`, default `--ssh-timeout 3`, handshake measured at 2.0-3.5 s); the alpha produced no partial in 38 runs.

## Root cause (verified in source, 2026-09-23)

1. **The pool has its own per-host cap that the CLI knob does not reach.** `gwz-transport/src/pool/mod.rs`
   `Config { per_user_host: 8, per_host: 8, total: 256, .. }`; `pool/allocation.rs:53-55` and `:106-107` refuse
   a new connection when `counts_for_host(..).total() >= per_host`. The operation's concurrency comes from
   `gwz-core/src/operation/resolve_per_host.rs` and `par_map_per_host.rs` (`--max-per-host`, default 8). With
   `--max-per-host 32` the operation runs 32 members at once but the pool serves them through 8 connections.
2. **One exchange per connection.** `GwzRemoteTransportRequirements.md` D2: "one active exchange per physical
   connection initially"; concurrent channels sharing one SSH connection are listed as out of scope for the first
   implementation (line ~315). So pool capacity == connection count; reuse only helps when N exceeds the cap or
   when an operation makes several exchanges per repo (push = advertisement + push).
3. The same requirements doc measured what the multiplexed design can expect: 24 advertisements as channels on
   one connection at 0.44-0.63 s each; 4 concurrent channels on one connection in 0.62 s total (lines ~67-68).

## Target

Both true at 32 repos over SSH, measured with the procedure in the research file:
- defaults are no slower than 1.0.17 with `--max-per-host 32` (about 3 s), and
- the pool opens far fewer connections than repos (target: at most 4 per host for 32 members), and
- no partial results across 3 rounds at 16 and 32 members.

## Phase 1 - capacity follows the operation (milestone: parity at 32 repos)

Goal: raising `--max-per-host` raises pool capacity; defaults stay as they are.

1. Failing test in `gwz-transport`: a pool configured with `per_host = N` grants N concurrent leases to one host
   and the (N+1)th waits; then the same with the config derived from an operation-level concurrency value.
   (< 150 LOC)
2. Plumb the operation's effective per-host concurrency into the pool `Config` (`per_user_host` and `per_host`
   >= the operation's per-host limit; `total` >= `--jobs`). Find where gwz-core constructs the transport/pool for
   an operation (grep `pool::Config` / `Pool::` in gwz-core and gwz-cli) and thread the value through the
   existing message contract without adding a new global. Keep the pool's validation bounds. (< 250 LOC)
3. Decide and document the default: the pool default `per_host` stays 8 only if `--max-per-host` also stays 8;
   the two must not silently disagree. Add one line to `gwz fetch --help` / docs if a new flag or env is
   introduced (prefer none). (< 100 LOC)
4. Re-measure the 32-repo table (defaults, `--max-per-host 32`, `--max-per-host 4`, `--jobs 1`) and record it in a
   "Results" section at the bottom of this file. Expected: the `--max-per-host 32` row drops to about 3 s.

## Phase 2 - concurrent channels per connection (milestone: fewer handshakes AND full parallelism)

Goal: K connections per host, each carrying up to M concurrent channels, so 32 members run on 4 connections.

1. Read `GwzRemoteTransportSshChannel.md`, `GwzRemoteTransportSshWorker.md`, `GwzRemoteTransportDesign.md` and
   the mux (`gwz-transport/src/mux/mod.rs`, `max_streams: 64`) before touching anything: the framing may
   already allow several streams per connection at the protocol level while the SSH worker serialises them.
   Record in this file which layer enforces "one exchange per connection" today. (no code)
2. Failing tests: a connection with `channels_per_connection = M` grants M leases and the (M+1)th waits; a host
   with K connections and M channels each reaches K*M concurrency; lease release returns a channel, not a
   connection; a channel failure does not take the connection down unless the session is dead. (< 200 LOC)
3. Implement channel-level leases on the SSH path: an ssh2 session is not thread-safe, so the worker that owns the
   session must drive M channels with one event loop (or one thread per session servicing channels), never a
   mutex around a shared session across threads. Respect the measured safe point (4 concurrent channels worked
   on GitHub); make M configurable with default 4 and hard-cap 8. (< 500 LOC, split further if it grows)
4. Allocation policy: fill channels on existing connections before opening a new one; open a new connection only
   when every open connection's channels are busy and the per-host cap allows; idle connections close on the
   existing idle timeout. (< 200 LOC)
5. Re-measure the 32-repo table with the pool capped at 4 connections per host. Expected: close to the 3 s
   parity row with 4 handshakes instead of 32. Record physical connection count and channel-open count per run
   (requirement C7 says tests can count both; expose the counts in `--verbose` transport rows if they are not
   already there).

## Out of scope here (separate briefs)
- https credential offering in the alpha (`offered=false`, private member fails) - `GwzRemoteTransportHttps*.md`.
- Empty top-level `errors` on a Partial result - machine-output contract.
- `--ssh-timeout` default of 3 s in 1.0.17 vs a 2.0-3.5 s handshake.

## Verification
- `cargo fmt --all -- --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings` in each
  touched crate; gwz-core's gate runs via `python3.13 run_tests.py` (PATH python3 is too old for tomllib).
- The measurement procedure is in `garnets-wz/.../research/gwz-fetch-scale.md`: a scratch workspace of 32 small
  public repos over SSH, three rounds, medians, exit codes, verbatim errors. Rebuild it under the scratch dir;
  never run measurements in the real gwz-dev workspace.

## Rules for the implementing agent
- Work in a gwz lane (`gwz local clone <name>` from gwz-dev), not in the main checkout; commit on the lane;
  do not merge, push, tag or release. No `Co-Authored-By` trailer. gwz-dev, gwz-core and gwz-transport
  AGENTS.md apply; every control-flow body braced; explicit `cfg` boundaries; TDD-first.
- Change one layer per commit: plumbing (Phase 1), then channel leases, then allocation policy.
- Record every measurement in this file's Results section with the command, the build, and the exit codes.
