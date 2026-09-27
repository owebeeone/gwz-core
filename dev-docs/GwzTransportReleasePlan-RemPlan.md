# GWZ transport release plan — first remediation plan

Date: 2026-09-27. Status: **remediation plan for [the first verdict](GwzTransportReleasePlan-Verdict.md); applied as one revision of `GwzTransportReleasePlan.md`.**

Every finding of the two reports gets exactly one disposition. The findings are [Consistency](GwzTransportReleasePlan-ReviewConsistency.md) C-P2-1 to C-P3-16 and [Safety](GwzTransportReleasePlan-ReviewSafety.md) S-P1-1 to S-P3-6.
- All findings are accepted. None is disputed.
- Every correction is to the plan's text. No correction changes the release's scope or the operator's recorded decisions.
- The revision also takes the reviewers' residual notes where they are cheap (§3).

## 1. Blocking findings

| ID | Disposition | Where | Closure test |
| --- | --- | --- | --- |
| C-P2-1 | Remedy (a): only `local` placement is in this release. §2 moves client placement, a `cli`-placed endpoint in any process, to the unsupported column; frame tags 16–31 stay reserved. §2 also restates adopted S5.1's "both placements" and the Transport Plan's Phase 6 ledger sentence as `local` only, with this plan as the amendment that sentence requires. `transport_capabilities` advertises only `local`. | §2 | Every left-cell capability of §2 has an owning step. No `cli` row can be marked a transport route at S7.2. |
| C-P2-2 | Phase 10 adopts by quotation the Phase 8 preamble (the stop rule, and "the operator runs the commands") and the product-repository sentence (order, visibility and registry pins). §4 and §10 name the same scope. | §4, Phase 10, §10 | From the plan alone, a reader can answer both "step 5's push succeeded and step 6's publish job failed; what now?" and "what does gwz-py's release branch pin gwz-core to?". |
| S-P1-1 | TR1.3's list gains client-side listener verification. The client checks the listener before sending any byte. On Linux and macOS: peer credentials, socket and directory ownership and mode, and no symlink. On Windows: the pipe server's user SID, with an identification-level impersonation level. The `/tmp` fallback is refused unless the directory already exists with the right owner and mode. A failed check sends no frame and refuses with a named code. Phase 4's note and Phase 7's exit gain the squatted-listener refusal, and §8 records the hazard. | TR1.3, Phase 4, Phase 7, §8 | Phase 7 exit row: a listener owned by another user, at the `auto` path and at an explicit address, on all three platforms, receives zero bytes, and the client refuses. |
| S-P2-1 | §6's merge rule is replaced by an enforceable one: (a) to (d) below. | §6 | The rule names its switch and its lineage, and S7.1's check covers the switch. |
| S-P2-2 | Phase 2's rule for main now ends at S7.1 plus TR2.6's GO. Before then, a release from main makes the retry help and `--max-retries` conditional on the transport build, and its own review covers the ordinary-path delta. OD4's alternative gets the same end condition. | Phase 2, OD4 | A help-render test in a build without the transport finds no retry sentence and no `--max-retries`, or the option is documented as having no effect there. |
| S-P2-3 | TR1.5 fixes the switch's scope. It is set per process: a flag, an environment variable or user-level configuration. A workspace or repository value is ignored, with a message naming the scope. Precedence is flag, then environment, then user configuration. A session with the switch on prints one notice, outside `--verbose`, at its first network operation. | TR1.5 | A workspace manifest and a member's repository config that both set the switch still take the transport route. |
| S-P2-4 | TR1.5 and TR1.3 both state the server rule. With the switch on, every process-wide read the native path makes joins the session's must-match set: at least `SSH_AUTH_SOCK`, and each contract §5.8 read. A mismatch refuses with `server_environment_mismatch`. The `auto` key covers those values when the switch is on. | TR1.3, TR1.5, Phase 7 | Phase 7 exit row: a server started with one `SSH_AUTH_SOCK` and a client with the switch on and another `SSH_AUTH_SOCK` gives a refusal, and the server's agent records zero signature requests. |
| S-P2-5 | TR1.2 gains a "Configuration ownership" question before its eligibility question. Each operation's endpoint configuration derives from its own session's snapshot, and a shared runtime never serves a session with configuration derived from another. The question supersedes the HTTPS design's §4 construction-time snapshot and `SshEndpointConfig::from_environment`. The eligibility question compares per-request inputs. The observations question adds that no request observes another session's configuration. | TR1.2, Phase 6 | Phase 6 exit row: two sessions on one host context, with different `GH_TOKEN` and `SSH_AUTH_SOCK`. Each `gh` invocation sees its own session's token, each new SSH connection asks its own session's agent, and no connection is shared. |

**S-P2-1's rule, as applied in §6:**
- (a) **One named switch.** The candidate cfg `gwz_transport_candidate` gates, until Phase 9:
  - the transport;
  - the `server` command and its options;
  - `SocketCoreBridge`;
  - `--max-retries`;
  - the off switch.

  S7.1's completeness check covers all of them.
- (b) **No 1.0.x releases from main.** Once a step that changes ordinary-build behaviour merges to a product repository's main, no 1.0.x release is cut from that main. The first is CS3.4 at the latest. A 1.0.x patch is cut from the last 1.0.x tag on a release lineage (AgentProcessRules L1-27), and recorded in `dev-docs/GwzMergeCheckpoint-v1.0.x.md` (L3-13).
- (c) **Documented contracts.** A defect fix that changes a documented contract, such as TR2.3's `errors` array, is released only with Surface review.
- (d) **Deltas on main.** At each patch decision, the program checkpoint lists the unreviewed and unactivated changes on main.

## 2. Non-blocking findings

| ID | Disposition |
| --- | --- |
| C-P3-1 | TR1.2's list of contract sections it amends gains §2's worker row, §4.2 (which states whether `TransportCapacityConflict` re-enters), §4.3, §5.5, §5.7, §8, §13 and §14. The NO-GO closing condition uses §4's wording. TR1.7 cites the contract's §9, §10 and §14 as TR1.2 and TR1.4 leave them. TR1.2 gets a closure sentence. |
| C-P3-2 | §4 restates the four dropped clauses: the S6.2 publish workflow and its pin sentence; S6.3's platforms and route-assertion preamble; S6.3's cost-and-count row; and S6.1's unconfirmed-cleanup report and its two cancel tests. |
| C-P3-3 | §4 re-homes Verdict-2's five carried items. The first three go to TR1.4a. The "paths" reading goes to S7.2 and the Linux fixtures to S7.3. |
| C-P3-4 | §1 adds a phase rule. In adopted text, "Phase 7" means this plan's Phase 9, "Phase 8" means Phase 10, and "S7" means S7.1–S7.5. Cited 1.1.0 and retry-plan steps are always prefixed. |
| C-P3-5 | §4 adopts the amendment's S7.2 addition explicitly, restating its S6.3 evidence as Phase 5's route tests and S7.3. The exit row replaces S6.3 rather than sitting beside it. S7.1's site list gains the session entry's arms. Phase 10 step 7 points at the moved pin obligation. "first S7.3 addition" becomes "S7.3 addition". |
| C-P3-6 | Phase 3 records that the bootstrap publishes satisfy S2.3's first-publish token clause. S2.3's exit adds a configured trusted publisher per name. Phase 10 step 1's token sentence is replaced. |
| C-P3-7 | §10's status texts use AgentProcessRules §7.2's exact supersession pattern, with scopes equal to §4 and a changelog entry. A GO-time status edit re-points `GwzPyTransportDesign.md` at §4. |
| C-P3-8 | TR1.2's idle question states that changing the idle default also supersedes S5.4's sentence, the Transport Plan's, the transport design's and the release-readiness guide's, by quotation. |
| C-P3-9 | §1 keeps the retry plan in force "as TR1.2 amends its §3 item 7, §6 and S1.4". TR1.2's capacity question names those clauses by quotation and says what replaces the refusal. |
| C-P3-10 | TR1.4 splits into TR1.4a and TR1.4b (with S-P3-1). TR1.4a lists every session-plan section to revise, and G2 becomes the build prerequisites. |
| C-P3-11 | "What can start now" orders TR3.1 first. TR2.1, TR2.2's reproduction and S4.2–S4.4 follow it. TR2.3 and TR2.4 need no transport build. |
| C-P3-12 | TR2.4 names `binding.rs`'s two version-3 branches, which go inside the same `cfg_if` boundary. Version 3 is refused before any Git, credential or pool effect when the feature is off. |
| C-P3-13 | TR1.6 adds Surface. TR2.3's contract change takes Surface under §6(c). S7.5's list adds the `--verbose` row fields. TR1.7 cites the session plan's R8 and CS4.5. §2's API row names `NativeCoreBridge`'s optional host context. |
| C-P3-14 | Folded into S-P2-1(a). |
| C-P3-15 | TR1.2's test question and TR1.3's list each add the Design §11 cells (S5.6 rows) they introduce, with their evidence kind. |
| C-P3-16 | TR1.6's amendment scope adds Design §11's HTTPS cell ("other helpers rejected") and 1.1.0 S7.1's "gh-only authenticated HTTPS stay as designed". §2 records the non-gh route under both options. |
| S-P3-1 | TR1.4a follows TR1.1 alone and revises the session plan's Phases 1 and 2 and its Phase 3 steps that do not depend on the runtime model. TR1.4b follows TR1.2 and TR1.3. §8 records the closed recovery: if TR1.2 stops under the cap, an amendment approved by the operator ships the contract's per-operation session host and the server without reuse. |
| S-P3-2 | TR8.1's miss rule is to retune within the shared ceilings and measure again, and only then to present OD8 with the gap and the list of exits to re-run. OD8 becomes an operator decision made on that evidence. |
| S-P3-3 | Phase 4's note drops Pageant. TR1.3 reconciles the server design's §11 agent row with S4.3: one agent source per session, chosen explicitly, and Pageant only by explicit selection after a fixture proves it. TR1.2's eligibility question gains the agent's kind and address on Windows. |
| S-P3-4 | TR2.4 names the feature `unstable-sequenced`, and the crate docs mark it unreviewed with no compatibility promise. OD5 keeps the alternative of excluding the module from the package. |
| S-P3-5 | A new TR3.2 retires git2-rs's upstream `publish.yml` and `ci/publish.sh` and every bootstrap workflow. A workflow-text test goes with it. The operator revokes the bootstrap token secret. |
| S-P3-6 | TR1.2's eligibility question identifies the agent by its own identity, not its path. Its trust-inputs question adds a different agent behind the same path, and keys that need confirmation or presence or have expired. Phase 6 exit gains a row for each. |

## 3. Residual notes taken

- "S3.3" is prefixed everywhere. 1.1.0 S3.3's live runs take their two-clock evidence from the production-graph regression, or run at `--max-retries 0` once TR2.1 lands.
- OD3's pin sentence: the pin moves only in a gwz-core commit that changes the allowlist.
- §1: the timeout plan is closed by 1.1.0 S3.1 and S3.3. S3.2 is the Q6 review.
- §2's reuse row names the in-process CLI's reuse within one command.
- §4 names what replaces the amendment's §6.
- §3 says "at the committed HEADs", and that gwz-core's committed manifest has no gwz-transport dependency.
- §2 extends redaction to private members' repository names in public reports, and adds that every live-account run needs the operator's go.
- OD4's alternative notes that reverting the help needs a retry-plan amendment, since S2.2 and S3.4 pin it by test.
- TR1.4a says CS3.7's "as today's `Command`… does" must not stand as the panic model: `Command`'s `Drop` finishes without a guard for unwinding.
- TR1.3 states what a sandboxed caller with `GWZ_SERVER` set sees.
- Phase 10's post-release check stops the servers it starts.

## 4. Re-verdict

The same two reviewers continue with their context intact. They receive this plan, the verdict and the revision's diff, and each re-verdicts its own findings on the new SHA-256. Reports go to `-ReviewConsistency-1.md` and `-ReviewSafety-1.md`, merged into `-Verdict-1.md`. An architectural root cause in that pass is theirs to classify.
