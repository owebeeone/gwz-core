# Alpha GitHub SSH command correction

2026-09-22. User-discovered alpha defect; correction tested, focused review pending.
The adapter inserted `--` before the quoted repository operand. A real GitHub
SSH comparison proves that spelling is rejected; canonical Git/libgit2 spelling
without the separator succeeds. The local general-shell SSH fixture accepted
both forms, so earlier tests failed to cover the hosted service's command grammar.

Correction: retain quote escaping, omit the separator, and explicitly reject
leading-dash repository operands, matching native option-injection protection.
Applies to both upload-pack and receive-pack. No protocol, pool, authentication
or timeout changes. Contract regression failed before the correction; all three
native channel tests now pass, including injection and session reuse.

Corrected real workspace fetch removes malformed-name failures and does not
reproduce the timeout. Private evidence still refuses access under the current
SSH account; stable gwz now fails that repository too. No credential changes.
This is separate from the adapter bug and is not represented as fixed.

Evidence (private access): gwz-core-evidence/campaigns/transport-qualification/runs/2026-09-22-alpha-github.
Retained Code review is the focused interior gate for this contract-preserving
bug fix. Alpha acceptance/release limitations remain unchanged.
