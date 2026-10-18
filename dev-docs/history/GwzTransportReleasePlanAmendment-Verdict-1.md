# GWZ transport release plan amendment — revision 1 verdict

Date: 2026-09-27. Status: **accepted at SHA-256 `213a164b79b7e46ce3855c699b491dcbd059f479e28d1a393b18e8a1ccb6514c` after [Consistency-1](GwzTransportReleasePlanAmendment-ReviewConsistency-1.md) and [Safety-1](GwzTransportReleasePlanAmendment-ReviewSafety-1.md) reported GO; this accepts the amendment text only**.
- It authorizes no implementation, commit, tag, push or publish.
- OD11 is recorded as the operator adopted it. OD12 stays open until TR1.3's GO.

The same two reviewers re-verdicted revision 1 with their context intact.
- Each verified the object, the plan and the five HEADs at the start and the end, and checked the supplied diff against a fresh `diff -u` of revision 0's copy.
- Neither read the other's round-2 report.
- Both ran only inspection commands and `diff`. Safety disclosed that it wrote one temporary diff file in the session scratchpad, outside the workspace, and removed it; nothing in the workspace was written.

| Axis | Verdict | Round-1 findings | New |
| --- | --- | --- | --- |
| Consistency | GO | All 11 closed (P2-1, P2-2, P3-1 to P3-9) | P3-1 to P3-4 |
| Safety | GO | All 7 closed (P2-1 to P2-3, P3-1 to P3-4) | P3-5 |

Both reviewers confirmed that every hunk of revision 1 maps to a disposition or a residual note in the [remediation plan](GwzTransportReleasePlanAmendment-RemPlan.md). Safety accepted S-P3-4's changed closure test as pinning the same property, and gave a Linux form of it. Neither classified anything as architectural, so the two-round cap is not reached. Both cleared their new items to land without a further round.

## Corrections applied after the GO

Each was applied as its reviewer specified. The amendment then carries the accepted status and hashes `bc1a28fb5f43ebbb2578247f7116c0640969b080b4bd385479bc054834658631`.

1. **Safety P3-5: the automount walk's probe.** `statfs` follows links and triggers mounts, so the probe itself could make the contact. Each probe now neither follows a symbolic link nor triggers a mount:
   - on Linux, `statx` with `AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT`, and `fstatfs` on an `O_PATH | O_NOFOLLOW` descriptor;
   - on macOS, `open` with `O_SYMLINK` or `O_NOFOLLOW`, and `fstatfs` on the descriptor.

   A link is read and its target walked under the same rule, never followed by the probe. Phase 7 gains two rows: a macOS link that points under `/net`, and a Linux autofs direct-map trigger that stays unmounted.
2. **Consistency P3-1.** OD12's yes and no lists also turn "OD12 is open." in the replacements of lines 7 and 508 into the decision.
3. **Consistency P3-2.** §3.12's risk states the route per SSH remote: security-key users lose pooling and reuse for SSH only.
4. **Consistency P3-3.** Under OD12 yes, TR1.3's contract amendments also narrow the contract's §1 exclusion "remote deployment". Under no, it stands.
5. **Consistency P3-4.** The amendment gains its changelog, where OD12's answer and the list applied will be recorded.
6. **Safety residuals:**
   - the host applies the automount walk before it creates the per-user directory;
   - the IPv6 literal must parse as an IPv6 address with no zone identifier;
   - relative `PATH` elements are skipped on every platform, Windows included;
   - TR1.3's enumeration includes the crypto backends and OpenSSL's start-up reads (`OPENSSL_CONF`, `OPENSSL_MODULES`, `OPENSSL_ENGINES`);
   - TR2.7's roots test is behavioural on Linux, with OpenSSL's default paths pointed at a temporary store; TR2.6's review covers macOS.
7. **Consistency residual.** The security-key record is a named manual row that S7.2 runs before its notes are written, so it has an owning step.

## Recorded

- **Not taken, as below the bar or outside the amendment:**
  - the handshake bound for a silent listener, which is server design content;
  - the native branch's disregard of `GIT_SSL_CAINFO` and proxies, which predates this amendment;
  - the tension between Phase 7's debt gate and native routes inside a server, which the accepted plan carries;
  - a note on the key-listing race's choice of key.
- **Deferred to TR1.3:** the `auto` key for routed operations with the switch off. The server design's §4 ("in any build") and the reuse design's §11 disagree on it today.
- **Residual risk:** TR1.3's enumeration is the whole guard for native routes through a server. Phase 7's source-level test pins only that the must-match list and the enumeration agree, not that the enumeration is complete.

## Application

The amendment's §5 lists the edits made on this GO, each with a changelog entry:
- **`GwzTransportReleasePlan.md`:** its status gains the amended-status sentence in AgentProcessRules §7.2's pattern, and its changelog records OD11 as adopted and OD12 as open.
- **`GwzRemoteTransportSshAgentDesign.md`:** its status gains the amended-status sentence for §5's algorithm sentence.
- **`GwzRemoteTransportSshAgentA2.md`:** a changelog note that TR2.8 admits ECDSA after A2.
- **`dev-docs/GwzCoreServerDesign.md`:** its status sentence adds the TR1.3 changes the amendment requires.

Still to come:
- **`dev-docs/GwzConnectionReuseDesign.md`** gains the key-types rule before its review starts (the amendment's §3.3).
- **The program checkpoint** entry waits until another lane's uncommitted edits to `dev-docs/CurrentProgramCheckpoint.md` land.

## Next action

TR1.2's review of the reuse design can start, with the key-types rule added first. OD12 is put to the operator at TR1.3's GO.
