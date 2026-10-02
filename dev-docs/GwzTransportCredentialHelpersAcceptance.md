# Credential helper implementation acceptance

2026-10-03. Status: **accepted at root b35887e579cbf8d42e42ae848038f408f51ddecb,
core ec43f585f7c768afa0fe71e23a7e503cfda9150c, transport
1aab733783e06b25cb5d2321d71ec0b34417a29c, Python
 a0d4350f31069362b3cfbeae668666ab47567264, CLI
f925e1165c2b2d368a00277594450b010a95867a and evidence
662d89828b478a2acce8c0308834db7d17c872f7 after
GwzTransportCredentialHelpers-ReviewCode-2.md and
GwzTransportCredentialHelpers-ReviewState-2.md reported GO;
this accepts credential-helper source and its typed consumers only**.

Surface GO is GwzTransportCredentialHelpers-ReviewSurface-1.md at preceding
root1de9e7f/core64ec039; CLI/Python/user docs are unchanged on round2 and both
source reviewers confirm the Surface corrections remain unaffected. Original
Code/State finding-owner closure reports are preserved with fresh round1
reports. Complete testimony is filed verbatim; no blocker remains.

One implementation object, two remediation rounds. Original settled review
found six distinct non-architectural blockers (Code2/State5, publication race
independently convergent); round1 settled review found one new cleanup-order
P2; round2 closes it. Zero reviewer-classified architectural root causes on
this source object; no escaped accepted-source defect is established here.
Separate earlier shared-clock design rounds remain their own object.

Round2 RED0/3 physically reproduces refused descendants continuing independent
writes. GREEN17 focused/458 affected with4 inherited ignores; unchanged
transport185/0/2 including doctest, source guards and exact core commit gate
pass. CoreClippy exits0 with49 retained warnings, not strict acceptance.
Earlier full ordinary/candidate suites predate correction and remain historical.
Final receipts are /Volumes/projects/limbo/gwz-tr222-round2-final-source-receipt-20261003.json
(SHA3d8d7ef0b898d5139a9529cf4a60410070423288c69741362b6cc5b7883084a1)
and the referenced round1 receipt. Raw validation requires access to coordinator
external logs; public builds and CI do not depend on private evidence.

Next: already-authorized GWZ family member merge, then fresh ordinary/candidate
CLI/core/Python integration against actual combined MAIN. Preserve unrelated
drafts and prior Python WIP stash until representation checks permit retirement.
This does not accept Windows/provider/trust, performance, selected-source,
packaging, supplied-carrier/iroh, sessions or release. No push, tag, publication,
activation or alpha installation is authorized by this acceptance record.
