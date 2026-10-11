# TR1.8 freeze review (Safety), round 2

**Tuple.** gwz-core `17ec75f1`, gwz-core-evidence `928f6d4`, gwz-transport `6146bd58`, gwz-cli `21d62310`, gwz-py `5950ba38`, gwz-sspi `73de8c8e`. All match at start and end.

- **Root does not match.** The root is at `f16d9fb0`, not `b3e6de03`. `b3e6de03` is its parent; `f16d9fb0` is a gwz-merge commit that touches only `gwz.conf/` (the lock and markers). No document under review changed. The root owner should restate the tuple.
- **Review reports are untracked.** The round-1 review reports, and Surface's round-2 report, are untracked in gwz-core.
- **Read-only.** I modified nothing.

## Closure of round-1 findings

| ID | Disposition claimed | Status | Evidence |
|---|---|---|---|
| **S1** (P0) | D2: HTTPS only, HTTPS→HTTP refused, `http://` takes the native route with a warning | **closed** | Design `:168` states the boundary and cites HTTPS design §5 (`GwzRemoteTransportHttpsDesign.md:194-200`, verified). Also register items 4 and 10 (`:292`, `:298`), L18, §14 item 7 (`:342`), guide `:13-18` and `:311-313`, Annex S `:220-222` and `:259`, amendment `:786`. Re-ran the counterexample: the text no longer permits Basic after a downgrade. MAIN refuses the downgrade (`https_destination.rs:209`). |
| S1 / S8, redirect bound | five hops, `prepare.rs:501`, product tests named | **partially closed** (P3) | The bound is stated and matches code: `budget.redirect_hops == 5` refuses the sixth hop. The non-forwarding tests are named. **However:** the native redirect-after-exchange test is assigned to step 4.9 in §14 item 13, but plan step 4.9 (`Plan:520-526`) does not list it. Also, `:501` guards only a 3xx on a natively authenticated **POST** route. A 3xx during the advertisement's SSPI exchange is refused earlier, at `native/exchange.rs:338` (`status != 401 \|\| history.complete`), which the design does not cite. The owed test must cover both points. |
| S2 | D4: kept and documented | **closed** | Design `:172`, register item 11, guide `:323-328`. |
| S3 | D1: measured (RX), then admitted; `http=`-only refused | **closed for safety; one stale sentence** (P3) | §6 HTTPS-origins bullet; register item 9; RX has 40 rows, all `restored=True`; h03 and h06 go direct on 1.0.17. Two residues remain. §14 "Claims kept" (`:352`) still says the https-origin selection and the CONNECT leg are "refused before any open", which contradicts §6. §6's last bullet says the forms RX did not run are "refused or taken by WinHTTP's rule as stated there", but that item states neither. Every reading fails closed or sends traffic to the proxy, so nothing silently goes direct. |
| **S4** | LocalInteraction classification, with L20 and an extension owed to step 3.6 | **not closed (P2, architectural)** | See R2-1. |
| S5 | L17, exact `\\.\pipe\<name>`, guide sentence | **closed** (P3 trace) | Design `:86-88`, L17, guide `:85-88` and `:124-132`. Plan step 3.3 (`Plan:398`) still says "reject UNC, traversal" rather than the exact grammar. The design controls, but 3.3 should quote it. |
| S6 | DACL barrier stated, owner set to the caller's user SID, residual in L9 | **closed** | Design `:108`, L9, guide `:124-129`, plan step 3.5 ("owner and DACL for the caller and SYSTEM only"). |
| S7 | One exchange's boundary defined | **closed** (P3 wording) | Design `:198`; plan 4.8 ("eight-round bound counts per operation"). §8 still reads "bound the exchange to eight request rounds"; align it with "per operation". |
| S8 | as S1's redirect row | **partially closed** (P3) | as above |
| S9 | Own-address rule named | **closed** | Design §6 implicit bypass, guide `:239-241`, plan 4.6 tests. |
| S10 | L19 | **closed** | Design `:198`, L19. |
| OD16 wording | NetNTLMv2, crack and relay, NTLM policy | **closed** | Design `:202`, guide `:311-319`. |

## New or residual findings

**R2-1: the Pageant wait relies on a change to an accepted contract that nothing carries (P2, architectural).** This is S4 not closed.

- **Where:**
  - Design `:114` (Bound), L20, and §14 item 13 ("owed to 3.6").
  - Guide `:119-123`, which states the behaviour as fact: "the connection's stall and setup-budget clocks stop … you cannot lengthen it".
  - The accepted clock forbids it: "Only the accepted ambient password-only route may enter local helper phases" (`GwzTransportSshHelperClockAmendment.md:248-249`).
  - Plan step 3.6 (`Plan:421-427`) has no goal, file or test for extending the clock, and its review is phase-only. The clock it would change was accepted after a dual Consistency and Safety review that needed two architectural rounds.
- **Violated clause:** the GO rule (§11, "every provisional clause is replaced by one physically proved design"). The design itself calls L20 "not accepted text".
- **State sequence (round-1 counterexample, re-run):**
  1. Step 3.6 is built as the plan states.
  2. The clock admits no agent route, so the Pageant wait runs in the Network phase with the stall live.
  3. A Pageant that asks for confirmation on each use, or a passphrase prompt on first use, outlasts the stall.
  4. There is one send and no resend, so the request expires and every retry repeats the failure. 1.0.17 waits.
  5. The guide has promised the opposite.
  - The other branch: 3.6 extends the clock ad hoc. That changes an accepted timing mechanism with no review at its tier.
- **Impact:**
  - A stuck state for confirm-per-use agents (recoverable only with `--transport native`).
  - A false user promise.
  - A cross-contract change with no owner.
  - Nothing is disclosed and there is no hang, because the receiver boundary holds: no read after timeout.
- **Correction (operator or lane owner).** Either:
  - (a) a short amendment to the SSH clock, with its own dual review, landed before 3.6, admitting the Pageant confirmation route into `LocalInteraction`; plan 3.6 then gains the goal, the files (the clock and its admission) and the tests. Or:
  - (b) a decision that 1.1.0 keeps the Pageant wait in the Network phase, stating the cost in §5, L20 and the guide (a confirm-per-use agent fails; a passphrase entered on first use succeeds on retry; native waits).
- **Architectural:** yes. The root cause is that the accepted clock admits exactly one local route. This is round 2 of 2, so it goes to the operator.

**R2-2: the D2 warning's timing is not pinned (P3).**
- **Where:** design §14 item 13 ("warning reaches the CLI and Python users", step 4.4); plan 4.4 test ("prints the warning").
- **Gap:** neither says the warning is emitted *before* any connection, unconditionally (not gated by `--verbose` or quiet), and visible by default in Python.
- **Impact:** cleartext is unchanged from 1.0.17, so there is no new exposure. But a late or gated warning would let the cleartext path proceed silently, against D2.
- **Correction:** state "at route selection, before the first request, every operation, not suppressible".

**R2-3: owed items are missing from their owning steps (P3).**
- The `prepare.rs:501` and `exchange.rs:338` test is not in plan 4.9 (see S8).
- The exact pipe grammar is not in 3.3 (see S5).
- Plan 4.7's goal still says the 407 message names "the off switch" (`Plan:506`), which contradicts its own tests and design §6.

**D1 refusal, checked.** It is stated so it cannot silently go direct. The `http=`-only and `socks=`-only cases are refused before any Open (§6, register item 9, plan 4.6 tests), unresolvable proxies end the operation, and parse failures never go direct. The undecided forms fail toward the proxy or toward refusal.

**D2 warning, checked.** The route is pinned: HTTPS only and downgrade refused, already enforced in MAIN. The timing is not pinned (R2-3 above).

**Removed claims.** No new removal drops a safety property. D3 keeps gwz-sspi `Identity::Explicit` and Digest uncalled, and plan 4.4 and 4.8 forbid building or offering them in 1.1.0.

Verdict: NO-GO. R2-1, a P2, stays open and is architectural; it needs the operator's choice between (a) and (b) above. Everything else is closed or P3.