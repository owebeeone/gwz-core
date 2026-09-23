# GwzRemoteTransportRetryPlan remediation

Status: **draft; not implementation authority**. One patch to
[GwzRemoteTransportRetryPlan.md](GwzRemoteTransportRetryPlan.md). No product
code, commit, tag, or push.

Round 1 reviewed plan SHA-256
`623511dd9508bc0a8c44ca14e576eec51923e041ed7ad6bea46a0f2db862948e`.

| Review | Verdict |
|---|---|
| [Safety](GwzRemoteTransportRetryPlan-ReviewSafety.md) | NO-GO. P2-1 … P2-5, P3-1 |
| [Surface](GwzRemoteTransportRetryPlan-ReviewSurface.md) | NO-GO. P2-1, P3-1 … P3-4 |
| Consistency | No report. The reviewer aborted. This is not a GO. |

Operator addition in this revision: a `--max-retries` option. The round-1
plan said there would be no new flag. That sentence is withdrawn.

Re-review is a new round. The patch adds a flag and changes the scheduler,
the pool capacity rule, and the retry machine. Cursor reviews stay on Grok.

## Dispositions

| ID | Disposition | Where it closes |
|---|---|---|
| Safety P2-1 | Logical `--max-per-host` stays the requested value. Live worker threads are bounded by resolved `--jobs`. Spawn failure is a typed operation error, not a panic and not a clamp. | Plan §1, §6, S1.5 |
| Safety P2-2 | `max_requests` becomes `max(1024, resolved --jobs)`. Its 16384 upper bound is removed. A checkout inside that count is not failed as `Capacity` for `max_requests`. | Plan §6, S1.3, S1.4 |
| Safety P2-3 | Caps are installed at operation start. Sequential operations each install their own resolved caps. A second operation that arrives while a lease is non-idle is refused. It does not inherit the first operation's caps. | Plan §6, S1.4 |
| Safety P2-4 | A cold or degraded key has one in-flight setup. Healthy keys use the normal cap. Sibling failures in one generation count as one attempt. A stale success does not mark the key healthy or reset the counter. | Plan §5, S3.1, S3.3 |
| Safety P2-5 | Only stall and aggregate timeouts are retriable. Interaction timeout and allocation timeout are not. The classifier takes the timeout origin. | Plan §4, S3.1, S3.2 |
| Safety P3-1 | Network-only bound and full wall-clock bound, both in terms of `--max-retries`, plus cleanup. | Plan §5 |
| Surface P2-1 | `--ssh-timeout` short help names a per-attempt stall and retries. Long help gives the no-progress bound and the aggregate bound at the defaults. | Plan §8 |
| Surface P3-1 | Retry scope, transports, `--ssh-timeout 0`, and `--max-retries` are in the long help. Fetch, push, and pull long help point at those flags. A later success counts as answered. | Plan §8 |
| Surface P3-2 | "100 is not a maximum" and "32 is not a maximum" are replaced by "Values above 100 are accepted" and "Values above 32 are accepted". | Plan §8 |
| Surface P3-3 | Both `--max-per-host` lines say member operations. The host is the hostname `git_host` parses from the remote URL. | Plan §6, §8 |
| Surface P3-4 | `--jobs` and `--max-per-host` say the smallest value is 1 and 0 is rejected. | Plan §8 |

The revised plan SHA-256 is `a1421c53ae6c49c4940739dca83e4056194c361a55198b22c42febcc5cfc0110`. Closure is a GO on that hash, not this table.
