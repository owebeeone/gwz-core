# GwzRemoteTransportRetryPlan remediation 1

Status: **draft; not implementation authority**. One patch to
[GwzRemoteTransportRetryPlan.md](GwzRemoteTransportRetryPlan.md), from
`a1421c53ae6c49c4940739dca83e4056194c361a55198b22c42febcc5cfc0110` to
`b40b75c024d2f0228a49d149fe95e19212aa1b7558779f4752b72c8f567af7f3`.

Round on `a1421c53…`:

| Review | Verdict |
|---|---|
| [Consistency](GwzRemoteTransportRetryPlan-ReviewConsistency.md) | NO-GO. P2-1 … P2-4, P3-1 |
| [Safety-1](GwzRemoteTransportRetryPlan-ReviewSafety-1.md) | NO-GO. New P2-1, P3-1. Prior Safety IDs closed |
| [Surface-1](GwzRemoteTransportRetryPlan-ReviewSurface-1.md) | NO-GO. New P2-2, P3-5, P3-6. Prior Surface IDs closed |

| ID | Disposition |
|---|---|
| Consistency P2-1 | §3.4–§3.6 name the v1.1.0 §5 bullet, the timeout plan's 3,000 / 10,000 sentences, and Design §10.1's 3,000 ms stall default. S2.3 applies those replacements on acceptance. |
| Consistency P2-2 | §3.7–§3.8 name Design §7.2, the transport plan's "cannot raise" bullets, and "Keep the pool's validation bounds." S1.4 cites them. |
| Consistency P2-3 / Safety P3-1 | `--max-retries` long help states the wait schedule itself. `--ssh-timeout` only sets the stall. |
| Consistency P2-4 | S2.2 pins `--ssh-timeout` help. S3.4 pins `--max-retries` and the fetch, push, and pull sentences. §11 names those steps. |
| Consistency P3-1 | §3.3 quotes the full pool-capacity out-of-scope bullet. |
| Safety P2-1 | Key state **Closed** after budget exhaustion or a non-retriable setup failure. Later members in that operation receive the recorded failure and open no setup. S3.1 covers `--jobs 1` late joiners, including after Healthy. |
| Surface P2-2 | Short and long `--ssh-timeout` help say the stall clock covers setup and a body read. A body stall aborts and is not retried. |
| Surface P3-5 | The non-retry sentence names fetch, push, and pull. |
| Surface P3-6 | SSH and HTTPS share the stall clock and the 30 s setup budget. `0` clears both on both schemes. |
