# Credential implementation remediation round 1 — STATE-AXIS REVIEW

**Review object:** Bounded original-reporter closure of State findings P2-1 through P2-5 on core `64ec039089b6e217625d7784b106d917452963cb` and transport `1aab733783e06b25cb5d2321d71ec0b34417a29c`. Controlling remediation document: `gwz-core/dev-docs/GwzTransportCredentialHelpers-RemPlan.md`. Source acceptance remains pending.

**Baseline:**

| Member | Revised settled SHA |
|---|---|
| root | `1de9e7fcbd17e5159ebb71ef3b683c2656a1cf8a` |
| gwz-core | `64ec039089b6e217625d7784b106d917452963cb` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Sources were read through committed range diffs, `git show HEAD:` and numbered source inspections. All six heads matched this tuple at the beginning and end; status showed no tracked modifications.

**Date:** 2026-10-03

**Axis:** State; bounded retracing of this reporter’s five original counterexamples and the changed surroundings necessary to assess their correction. Independent, adversarial, read-only. Current peer reviews run in parallel; nothing here relies on them. Filed verbatim by the lane owner.

**Verdict: GO** — all five original State P2 findings close. No original finding remains open or awaiting evidence. This GO closes those counterexamples only. It is not aggregate source acceptance, a fresh review of the whole package, or release approval.

---

## 0. Evidence base

I read the canonical `PromptStateClosure-1.txt` in full, the newest credential checkpoint section, the committed remediation plan and remediation record, and the original State report. The standing workspace and process instructions remain applicable; the inspected root range contained no changes to `AGENTS_GWZ.md`, `AGENTS.md` or `GwzProcessOptimization.md`.

The original accepted design and Timing, ConfigurationView and SshHelperClock requirements remain the contracts behind these findings. The remediation record was used as evidence, not as authority that could close its own findings.

The bounded source examination covered:

| Original finding | Corrected source and necessary surroundings |
|---|---|
| P2-1 | Core `ssh_setup_context.rs:53–71,138–142`; complete `ssh_setup_context/publication.rs:1–59`; publication tests through line 136; immediate observer test at `tests.rs:44–66`; OpenRequest capture/completion; NativeResource failure projection; transport `setup_clock.rs:208–223` and atomic-admission regression |
| P2-2 | Core `https_auth/secret.rs:40–68,106–115`; username allocation regression at `runner_tests.rs:18–29` |
| P2-3 | Core helper runner’s checks, `run_secret`, completion/admission path at `runner.rs:138–184`; lookup at `lookup.rs:105–126`; complete new runner tests through line 143; retained HelperJob cleanup at `owner.rs:200–251` |
| P2-4 | Backend SSH policy propagation; private RequestContext opening; session opening/handoff; worker Opening construction and pool partition at `ssh_worker/runner.rs:63–73,128–145`; SSH selection/gate at `ssh_local.rs:49–67,110–132`; complete enablement regressions through line 149 |
| P2-5 | Transport `stream/incoming.rs:6–27,115–160`; complete `tests/closed_failure.rs`; core retained-facts forwarding and RPC projection at `https_remote.rs:22–38,65–78`; new RPC regressions |

No tests, builds, probes, mutations or file writes were performed. Original counterexamples were independently retraced in source. Test outcomes below are recorded execution evidence.

The external exact source receipt:

`/Volumes/projects/limbo/gwz-tr222-remediation-final-source-receipt-20261003.json`

matched its required SHA-256:

`ceed1a3740aa76ab3609c1758392bea9d3c9abe6d04200e462147addd2ed523d`

I compared every owned entry with committed `git show HEAD:` bytes: 31 core, five transport, one CLI and two Py paths, 39 total, with zero mismatches. The receipt’s precommit member heads were not substituted for the settled tuple; the committed byte comparisons establish the source binding. All eleven final gate logs matched their receipt hashes.

The relevant raw final logs contain passing results for:

- Immediate zero-allocation observer; independent pool observer; pre/post-commit unwind; competing publishers; earlier terminal rejection.
- Username spare-capacity, stable pointer/capacity, repeated SSH conversion and unchanged Basic header.
- Final completion/parsing admission, ready timer/cancellation with a real completed child, and cleanup.
- Disabled backend against a helper-enabled password-only endpoint; selected-key preservation and separate pool reuse.
- Valid detailed `Closed.failure`, invalid close detail, RPC timing/retry projection and existing Failure-facts precedence.
- Atomic admission distinguishing an equal prior resource terminal and exact-boundary expiry.

The final affected core log records **455 passed, zero failed, four ignored**. The transport log records **185 passed, zero failed, two ignored**, including doctest. Strict transport library Clippy completes without warnings; core Clippy completes with **49 retained warnings**, including the disclosed private opening arity expansion. This is not strict-core Clippy green.

I also inspected the recorded RED assertions corresponding to the original defects and atomic-admission correction. Earlier RED commands have the receipt’s disclosed retrospective-envelope limitation. The retained default-concurrency affected result was **454 passed, one failed, four ignored**, in the ordinary address-fallback SSH fixture; its isolated row passed and the final bounded-concurrency union passed. That failure is not concealed or counted as a passing run.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1 | Associate detailed Failure with the exact admitted terminal before observers cache fallback; retain arbitration and unwind safety | Immediate callbacks now occur after association. An independent pool observer waits for the logical association. Earlier cancellation/network/equal-resource winners cannot receive unadmitted detail. Recorded regressions pass. | **Closed** |
| P2-2 | Allocate SSH terminator capacity before copying sensitive username bytes | Parser reserves `len + 1`; first and repeated SSH conversion do not grow the populated vector. HTTPS header construction excludes the terminator. Recorded pointer/capacity regression passes. | **Closed** |
| P2-3 | Recheck unchanged deadline/cancellation after child completion and parsing, before success admission | Ready child success is checked regardless of select winner. Parsing is inside supervised ownership and checked afterward. Refusal reaches cleanup and cannot return a credential to the answer consumer. Recorded regressions pass. | **Closed** |
| P2-4 | Preserve backend helper enablement through private local SSH opening and isolate pooled authentication | Disabled bit reaches the helper gate; disabled and enabled pool identities differ while original key selection is retained. Password-only and selected-key/reuse regressions pass. | **Closed** |
| P2-5 | Retain admitted `Closed.failure`, preserve separate facts and immutable first ownership | Valid close moves the full Failure into retained ownership after admission. Late terminals cannot replace it. RPC derives missing facts without mutating the retained Failure. Recorded transport and consumer regressions pass. | **Closed** |

## 2. Invariant analysis

**P2-1 — first detailed failure publication.** The original sequence exposed the clock terminal before storing detail, allowing `failure(record)` to cache an incomplete fallback permanently. The revised `Publication` reserves the association before calling the clock. `SetupContext::failure` waits while that logical publication is pending. Guard drop associates the supplied Failure only when atomic admission returns `Ok(record)`, clears the pending state, releases the context lock, notifies readers, and finally delivers clock notifications.

Thus an immediate custom-waker observer sees the completed association. A separate pool observer that sees the committed terminal before association cannot cache the fallback. A prior terminal returns `Err(existing)`, including an equal scalar resource cause, so candidate detail cannot be attached to a different winner. Precommit and postcommit unwind release the logical reservation through guard drop.

The examined production capture paths release the setup-slot lock before projection; NativeResource obtains the Job result before projecting its error. The corrected association does not await child disposal. The original zero-allocation sequence now retains `helper_budget_ms: 0` and reaches the typed M10/code75 renderer.

**P2-2 — username allocation lifetime.** The parser allocates space for the SSH terminator before copying the username. Appending the terminator therefore fits in the existing allocation. Repeated conversion sees the terminator already present. The original growth/deallocation step is absent. Header construction strips that terminator, preserving the original HTTPS username bytes. Drop continues to wipe the owned username and password allocations. The regression’s pointer/capacity checks directly cover the allocation transition without inspecting freed memory.

**P2-3 — late helper success.** The select remains capable of choosing completed work when refusal is also ready, but completed work is no longer sufficient for success. `admit_child_output` checks the same cancellation tokens and absolute deadline. Final fill uses `run_secret`, whose parser checks before and after parsing; another admission check follows before Job success completion and return.

Retracing the original withheld-poll sequence now yields Timeout or Cancelled whichever ready select arm wins. Parsing that crosses the deadline also yields refusal. On these refusal paths, the Job still owns its process group and permits and reaches termination/reap or retained cleanup. Output and parsed secrets leave through their wiping owners. The HTTPS credential consumer consequently receives an error rather than an answer to cache or offer.

**P2-4 — disabled SSH helpers.** Backend configuration now captures the existing helper policy in `HostRoute` and passes it through `open_with_helpers`. Disabled opening creates private extras even when there is no URL password or host-spelling difference. The endpoint receives the bit through handoff, and `ssh_local` checks it before entering helper method discovery/lookup.

The worker partitions disabled operations under separate opaque pool identities. Opening separately retains the original authentication selection, allowing selected-key lookup to use its original registry identity and project successful authentication under the partitioned pool identity. Enabled and disabled requests therefore cannot borrow each other’s helper-authenticated connection. The recorded real password-only test covers both operation orders, no disabled lookup/password offer, and enabled success; the selected-key test covers preserved authentication and reuse within separate partitions.

**P2-5 — detailed close failure.** Admission and close-state checks still precede retention. The corrected `Closed` branch moves its Failure into `retained_failure` after recording the scalar error. The original valid close handshake now retains timing and retry detail exactly, including allocation ownership. The existing terminal guard ignores subsequent envelopes before they can replace that value. Invalid detail fails admission and retains nothing.

`Closed.facts` remains separately available. The RPC adapter supplies those facts only to its derived callback/error clone when the retained Failure has no facts. Existing Failure facts take precedence. The original typed projection gap is therefore closed without rewriting the first retained Failure.

## Changed-range analysis

The necessary corrections comprise:

- A cohesive publication guard and pending logical association in core, plus the additive transport-neutral atomic admission result.
- Preallocated SSH username terminator capacity and header compatibility handling.
- Successful helper-result admission checks and parsing within supervised Job ownership.
- Private helper-policy carriage, preserved authentication selection and opaque SSH pool partitioning.
- Full `Closed.failure` retention and separate-facts projection into the RPC consumer.
- Source-specific regressions and recorded gates.

The shared `terminate_if_alive` API is a material interface change. I inspected its admitted-versus-existing behavior only as necessary to retrace P2-1, including equal scalar winners and expiry. This bounded closure does not replace the separately required fresh Code/State review of that shared interface and its changed proof surface.

No new concrete blocker was established in the necessary surrounding paths. The five original root causes remain **non-architectural** and are closed; this review introduces no new architectural root cause. The separate Negotiate and help corrections belong to other original findings and are not independently accepted by this report.

## 3. Risks and next action

The verdict is limited to the five original State counterexamples. Recorded predecessor full suites remain pre-correction evidence. Retained warnings, ignored campaigns, the disclosed concurrency-sensitive SSH gate failure, and native Git/OS ownership limitations are not relabeled as comprehensive green proof.

Windows/provider/trust/parity, platform/performance/package/release outcomes, supplied-carrier expansion and 1.2 sessions remain deferred. This report supplies no approval for those outcomes or for activation.

The next action is to file this bounded closure and combine it with the required fresh Code/State and continued Surface verdicts on the same settled tuple. Aggregate acceptance and integration remain the lane owner’s decision after those reviews.
