# Exact agent-key selection design: remediation plan, round 1

Object: `GwzExactAgentIdentityDesign-DRAFT.md` at gwz-core `de8e4788`. Reviews: `-ReviewConsistency.md` (NO-GO, P2 3, P3 7) and `-ReviewSafety.md` (NO-GO, P2 4, P3 9), peer-blind, 2026-10-18. One merged revision of the design text; each reviewer re-verdicts its own findings on the revised commit.

**Blind convergence** (found on both axes independently):
- METHOD_NONE is worded K2 instead of K3: Consistency P3-1 = Safety P2-2.
- K1's remedy is wrong for Pageant: Consistency P2-1 = Safety P3-2.
- PuTTYgen's RFC 4716 `.pub` falls to the generic refusal: Consistency P2-3 = Safety P2-4.

| Finding | Disposition | Closure in the revised text |
|---|---|---|
| C P2-1 / S P3-2: K1 Pageant remedy | Accept | K1 carries one remedy per source (Unix `ssh-add`; Windows OpenSSH pipe naming the service; Pageant "Add Key" or close Pageant). Step 1.3 pins each. |
| C P2-2: Unix no-agent gets the bare `Unavailable` text | Accept | A new text K0 (exact open, agent unreachable: none configured or nothing listening). It names the key file, says nothing was offered, and gives the start-agent / `ssh-add` next action. Selection condition stated; X5 split per platform. |
| C P2-3 / S P2-4: rule C near-misses (RFC 4716, `.ppk`, BOM, UTF-16LE, options-prefixed line) and the PowerShell `>` remedy | Accept | Rule C accepts RFC 4716 or classifies every listed near-miss into K4, with words that name the fix. The remedy is shell-safe (PowerShell `Set-Content -Encoding ascii`; Git Bash `>`). One parser row per case. |
| S P2-1: capability not tied to the compiled route | Accept | The static answer is the same predicate that installs the transport route (`cfg!(gwz_transport_candidate)` until C3b, then the activated build). The doc comment is updated. The qualification test asserts false without the route and true with it, on all three platforms. |
| S P2-2 / C P3-1: METHOD_NONE gives K2 | Accept | The exact step marks METHOD_NONE as rejected (or carries a marker); K3 gets its own selection condition. X8 adds a row where the server accepts none of the RSA algorithms and K3 is expected. |
| S P2-3: oracle cannot tell a certificate from its key; no both-loaded row | Accept | The oracle asserts on (type token, fingerprint), plus the certificate ID, and never on the fingerprint alone. New rows have the agent holding the plain key and its certificate, select each, in both list orders, with an ambient negative control. Intern key named as (Key, kind, blob). |
| C P3-2: K5 native advice not source-aware | Accept | K5's remedy is built from the transport source and the selection source. The native clause is dropped from T6, T6b and the cap text when a public key was selected. |
| C P3-3: K6 names only `--identity` | Accept | K6 says "name `<file>.pub` wherever you named `<file>`", always prints the shell-safe `ssh-keygen -y` line, and fixes the attribution. |
| C P3-4: capability doc comment and the OQ4 premise | Accept | Merged with S P2-1. OQ4 is restated with the in-process option and its race. |
| C P3-5: step graph and estimate basis | Accept | K6 gets a single owner. The missing edges are added (2.3 needs 2.1 and 2.2; 5.1 needs 4.1). An in-crate synthetic-Pageant step owns X9e, B06e and the Pageant exact row. The Windows oracle control is proven in Phase B. The Surface review is placed explicitly. The estimate is restated like-for-like. |
| C P3-6: unlisted user docs and the `private_key_path` name | Accept | Step 3.2 lists `auth.md`, `Releases.md`, `Troubleshooting.md`, `MessageCatalog.md`, `MachineOutput.md` and the gwz-py docstring. |
| C P3-7: drift and citations | Accept | Each item is corrected. Keys in the reuse row are renamed so they don't collide with K1/K2. |
| S P3-1: K3 after a refused URL password | Accept | The "key offered" bit is `key_fingerprint.is_some()`. |
| S P3-3: K5 classifier unspecified | Accept | Classifier, bounded read (`regular_file::open`), and wording that fits every reason native was chosen. |
| S P3-4: CR/LF line rule | Accept | Same CR/LF split as the container; control characters refused in the comment. |
| S P3-5: blob structure not validated | Accept | Per-family field-layout validation in `agent_pubkey.rs`, refused with K4, plus a parser row. |
| S P3-6: "no fallback" row could pass vacuously | Accept | One password-only-server variant and one password+publickey variant, each with an ambient control. |
| S P3-7: step 2.1's intermediate state | Accept | The invariant is stated and tested: an Agent entry in the selected branch is refused, never filtered out. |
| S P3-8: gwz-py's advisory check precedes the flip | Accept | The advisory check moves into step 5.1. |
| S P3-9: a URL password is still offered first | Accept | Stated in the long help and the troubleshooting entry. |

The Surface note on the root not being covered by `gwz auth identity --set` without `--all` is pre-existing behaviour, recorded as a separate operator finding, not part of this revision.
