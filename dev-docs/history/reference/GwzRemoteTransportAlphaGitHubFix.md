# Alpha GitHub SSH command correction

2026-09-22. User-discovered alpha defect; accepted at core `0904568d7e64c323f4353171ea203506360c8984`
after retained Code GO (root dev-docs/GwzRemoteTransportAlphaGitHubFix-ReviewCode.md).
Corrected alpha installed; SHA256 `5419fd0218ab06084bee9152b09a79864a2c165bce62d9d7924a70d4bb6dce8c`.
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

Live identity/access checks describe the agent tool environment, which may differ
from the user terminal environment. No user credential/configuration was changed.
