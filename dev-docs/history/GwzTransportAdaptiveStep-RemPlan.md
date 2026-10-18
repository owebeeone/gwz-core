# Adaptive step: remediation plan (round 1)

Object: gwz-core `e043d952`. Review: `GwzTransportAdaptiveStep-ReviewCodeState.md` (NO-GO: eight P2s, three of them architectural, and two P3s). One merged patch; the same reviewer re-verdicts against its eight probes (`scratchpad/review-adaptive/probes.log`, sources in that directory).

| Finding | Disposition | Closure test |
| --- | --- | --- |
| P2-1 | One connection table per site at governor level, kept whether or not any operation is live. Per-operation machines read it; their windows open on it. | PROBE1, plus the idle-lease variant |
| P2-2 | `Seen::Started` carries the pool request's operation (owner). Only the arming operation's connection takes its test. If the pool cannot carry the owner, add it to gwz-transport's request and connection events: an additive API, no wire change. | PROBE2: the gate reopens within the test's setup clock |
| P2-3 | A fresh HTTPS answer applies §4.2's rule in every scope, as SSH does. | PROBE7, over HTTPS and over SSH |
| P2-4 | A connect failure carries its pool `ConnectionId` (`ConnectFailed`, `ConnectTimeout`, `SetupEnded`). The result is judged on that connection's own window; a cancelled connect's window freezes at the cancel. The FIFO `ended` queue goes. gwz-transport change if needed: additive, no wire change. | PROBE3, plus the SSH report-before-Retired variant |
| P2-5 | An RAII carrier token: dropping it without a connection gives the test back, on every exit. | Endpoint tests for each exit the review names |
| P2-6 | HTTPS Retry applies SSH's budget rule. | PROBE4 |
| P2-7 | No implicit scope creation. `set_demand` and `test_unused` do nothing for an ended operation, and `report_demand` skips ended requests' opens. | PROBE6, plus a mid-open cancel endpoint test |
| P2-8 | A throttle at `hi = 0` gets the retry machine's wait, or a T0 hold, before the requeue. The changelog's reading (2) is corrected. | PROBE5: at least 1 s between attempts |
| P3-1 | Wire `closes_suppressed`, `is_test_carrier` (no deferral of a carrier in the SSH background close) and `connect_time` (Ts). | The identity-churn probe test, and a carrier-never-deferred test |
| P3-2 | With `adaptive = false`, Suspects go to the retry machine as before. | A dead-host test at `--max-retries 0` with more members than C |

Cap: this is round 1. A reviewer who finds a third new architectural root cause in a later round stops the lane for redesign.
