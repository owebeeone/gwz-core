# GWZ transport credential helpers design (TR1.6) — CONSISTENCY-AXIS CONFIRMATION OF REVISION 4

The Consistency reviewer's confirmation of the forms revision 4 gives its round-3 P3-1 and P3-2, filed verbatim by the lane owner.

**TR1.6 revision 4 — Consistency confirmation** (object `GwzTransportCredentialHelpersDesign-rev4.md`, SHA-256 `9b2db41e0c58205eb5c29a43b18bec6e380b0ee99f82125ed3b84f47e6040ed2`, 426 lines, verified; diff from revision 3 read in full).

- **OQ7 (L416–419):** confirmed. C10 is now the operator's question, with (1) the bounded detail field and (2) shared text, each with its costs, and a recommendation — the form my P3-1 asked for.
- **N14's qualifier (L365):** confirmed. "gwz 1.0 failed some of these instead, when the server offered Negotiate or NTLM" removes the false "as in gwz 1.0" for those schemes.
- **§8's D19 (L169, L173):** confirmed. Both now say which exceptions OQ7 (1) removes, consistent with L292 and L302.
- **§9.1's OQ7 (2) contingency (L215):** confirmed, verbatim my P3-2 text.
- **§4's M8 row (L97):** confirmed — "M8, naming the cause (OQ7 (1)), else M2's text".
- **§1 item 8 (L25):** confirmed.
- **The differing part (L292, L302, L303):** confirmed, and the drafter's reading is the correct one. I re-derived 1.0.17 for `NTLM`-only on macOS/Linux at `v1.0.17`: with no answering helper the callback falls to its `Auth` error (`transport_support.rs:258-262`; `allowed_types` is `USERPASS` only, so `is_default()` is false), which `clone_error` recognizes (`transport.rs:770`), so 1.0.17 skipped the member quietly; with an answering helper it failed loudly. The transport runs no helper for `NTLM`-only (D15), so it cannot tell the two apart, and failing all `NTLM`-only members loudly would break the quiet-skip case, which is a working setup — exactly the invariant my finding protects. Keeping the skip under both options, as a stated exception in the benign direction, is right. Likewise `Negotiate`+`Basic` with no helper answer (M2) staying skipped under both options is a stated exception, since OQ7 (1)'s detail carries schemes for M6 only; that matches the limit I noted in P3-1. `Negotiate`-without-`Basic` (including `Negotiate`+`NTLM`) failing loudly under OQ7 (1) matches 1.0.17, which failed that challenge whether or not a helper answered.
- **One extra change in the diff, checked and consistent:** an unstartable `git` moves from M8 to M1 (`Unavailable`, `external_tool_missing`). §4's M1 row (L100), the latch (L109), Windows (L112), D19 (L169), M1's `<what>` (L311), M8's cause list with "A `git` that cannot start is M1's" (L321), ErrorCatalog row :32 (L327), N14 (L365) and T17(b) (L270) all agree. No objection.

No objections. My round-3 P3-1 and P3-2 are closed on revision 4; this axis stays GO.
