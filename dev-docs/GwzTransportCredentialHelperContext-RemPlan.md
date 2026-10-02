# Helper context amendment — combined remediation round 1

Date: 2026-10-03. Owner: root. Status: **NO-GO pending corrections and original-reviewer closure**.

Reviewed draft tuple: root `bb44a7214eac779095352224e8940df268fd3f96`,
core `0adb093a99f4eb8e9b8818b767514a842533a193`, transport
`9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. Raw Consistency, Safety and
Surface reports are filed verbatim beside this plan. This is one correction
of the same bounded draft, not a new interface or implementation review.

## Dispositions

1. **Consistency P2-1 / Safety P2-1: same causal-diagnosis root cause.**
   Keep valid zero retained allocation and no helper spawn/latch. Amend the
   complete M10 wording, caller heading and explanation so obtaining helper
   resources failed within the stated allowance; busy/waiting helpers are a
   possible cause, never an established fact. Explicitly supersede the old
   unchanged-remainder restriction and the helper-admission assignment for
   an already-exhausted allowance. No extra wire field, policy or clock.
   Regression: zero allowance with both semaphores free emits truthful zero
   seconds, starts no helper, sets no latch and never asserts saturation.
   Retain positive-allowance permit/host-slot saturation and one deadline.
2. **Surface P2-1: native Git recovery uses a broader configuration scope.**
   Correct every helper-identification and sign-in claim, including M8's
   wording if needed, so a successful ordinary Git command is not presented
   as proof that GWZ's actual helper was repaired. Explain repository and
   conditional-include differences; provide a concrete secret-safe route to
   identifying and repairing the unconditional global/system/XDG/environment
   helper sources GWZ uses. Preserve legitimate unconditional includes and
   accepted credential policy; do not substitute --no-includes as purported
   equivalence. Explicitly supersede affected recovery/message clauses.
   Walk through global helper A needing repair with working helper B from
   both a matching includeIf and a repository-local setting; instructions
   must reach A or explain the mismatch and name the next action to A.
   Also keep the ordinary A-only recovery understandable. No new command/API.
3. **Consistency P3-1: exact decimal versus unsuperseded T15(b).**
   Fold this correction into this patch. Precisely supersede TR1.6 §10
   T15(b)'s rounded-down seconds assertion; captured integer milliseconds
   drive both timer and detail, and seconds render exactly. Specify how a
   sub-millisecond retained remainder is captured/expired without extending
   the budget. Require 1,250 ms to produce 1.25 in both old/new regression rows.

## Closure

The drafter changes only the amendment and caller Surface note. Existing-
contract runner working edits remain excluded, and no new tags/cause are
implemented until amendment GO. Root settles one corrected document tuple,
checks its exact boundary gate and returns it to the SAME three reviewers.
Each reviewer checks its own counterexample and the revised whole document;
Surface continues reading only the caller note. Maximum two remediation
rounds; a further architectural cause follows the controlling stop rule.
This plan does not accept product code, Windows support or release readiness.
