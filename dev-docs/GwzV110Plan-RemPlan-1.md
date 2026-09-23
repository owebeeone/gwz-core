# GwzV110Plan remediation 2

Round-2 reviews, both NO-GO, on plan SHA-256
`6ec8f7e715e082f144b21b4b1862a9f4125650b5e62db143485729f1e13e1c4c`.
Round-1 findings stayed closed. This patch addresses only the new findings.
Safety labeled both of its new findings architectural. This is the second
remediation round.

| ID | Disposition | Closure test |
|---|---|---|
| Consistency P1-5 | If S1.1 chooses a separate binding package, Phase 2 re-enters: fifth name in S2.1, `gearu.toml` and publish workflow in S2.2 and S2.3, `gearu plan` before Phase 8 step 3. | S1.1 states that re-entry. |
| Consistency P2-6 | S1.1 names Phase 8 step 3 as the separate-binding publisher. Step 4 remains `gwz-git`. | The separate-binding cite is step 3 only. |
| Safety P1-4 | An unsupported mark on a §2 in-release cell fails S5.6 and blocks S7.5 and Phase 8 unless an accepted amendment first removes that cell from §1 and §2. Linux ARM64, macOS x86-64, and exact-agent-until-proven may still be marked without that amendment. | S5.6 states the fail and the exception. |
| Safety P2-6 | S5.6 precedes S7.1 in the normative sketch and in the §4 prose. | Both say S7.1 waits on S5.6. |
