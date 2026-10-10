# Skim package: amendment 2, revision 8 (TD14 A)

Date: 2026-10-10. Status: **a package for a skim review; no review has run.** It describes the difference between revision 7 and the revision 8 DRAFT of [`GwzTransportReleasePlanAmendment-2.md`](GwzTransportReleasePlanAmendment-2.md). It decides nothing and authorizes no implementation, commit, tag, push or publication.

**Reviewer's object.** The amendment's revision 8 DRAFT, SHA-256 `a39f47649575dcd790667e901819bf93c3bf62938475fbebd497396d28bb94ae`; its base is revision 7 (gwz-core `HEAD` at the time of drafting), SHA-256 `c37f0a5a2e5f89542fe6bb86455fceaa304d0e0725059132755d505327d9da02`. Review the difference only. Revision 7 is itself an unreviewed DRAFT (its status bullet says so), and this package asks nothing about it. Line numbers below are **revision 7's**; revision 8 adds one line to the header and one to the changelog, so its own numbers sit one to two lines later.

## 1. What changed and why

On 2026-10-10 the operator decided TD1 to TD4 and TD14 A ([decision list](GwzTransportTR18-OperatorDecisions.md), the "Operator discussion record"). Windows 1.0.17's rows contradict the amendment's statement of which identity answers which scheme. The decisions, with their qualifications:

| Decision | Recorded answer | Effect on the amendment's text |
|---|---|---|
| TD1 | A accepted. The logon session answers `Negotiate`/`NTLM`; the configured helper answers `Basic`. It is not a Basic-only restriction. Stop after rejection of the logon identity; do not replay or downgrade to helper credentials. | The helper-first rule and the helper-identity-over-`Negotiate` rows go. The helper is asked only when `Basic` is the only workable scheme. A rejected logon identity ends the operation. |
| TD2 | A accepted. Refuse Digest-only promptly; ignore Digest when another supported scheme is available. | The `Digest` rows (helper identity over SSPI) go; a `Digest`-only refusal and a `Digest, Basic` row take their place. |
| TD3 | A accepted. Follow discovery redirects on Windows; no credentials across origins; HTTPS confirmation owed. | The redirect row stays and is marked a transport-only assertion and a deliberate difference (1.0.17 follows none on Windows). |
| TD4 | A accepted. Refuse a challenged POST with a clear diagnostic; never replay its body. | A sentence and one fixture row are added. They state the outcome Windows 1.0.17 already has (a failure), with the transport's own message. |
| TD14 | A accepted. Reconcile the contradicted text in revision 8 before step 2.4's review; no new mechanism. | This revision. |

Two clarifications from the discussion record are carried into the text as written: NTLM and Kerberos are distinct (the successful fixture exchanges used NTLM, so they do not qualify Kerberos), and GitHub token authentication remains the helper's Basic-over-HTTPS path. The HTTPS confirmation of TD1 and TD3 is owed, so the new text says so rather than asserting an HTTPS result.

## 2. Beyond the three cited lines (flag for the lane owner)

The decision list cites §3.5's lines 166, 178 and 226. The same claim stands in three other places, and leaving them would make the amendment contradict itself. Revision 8 changes them minimally. If the lane owner wants the change confined to §3.5, revert items 4 to 6 in section 3 below and the half-sentence in the status bullet that names them; nothing else depends on them.

## 3. The superseded sentences, verbatim, and their replacements

### Item 1. §3.5, line 166 (TR1.8, logon session bullet "Precedence of the credential helpers")

Superseded:

>     - **Precedence of the credential helpers.** When the challenge offers `NTLM`, `Basic` or `Digest`, a configured credential helper (TR1.6), `gh` among them, with a credential for the URL the credential will be sent to answers first. On Windows it goes over the scheme WinHTTP picks for a user and password: `Negotiate`, then `NTLM`, then `Digest`, then `Basic` (`winhttp.c:139-147`), through SSPI with the helper's identity (`SEC_WINNT_AUTH_IDENTITY_W`) for the first two. A challenge that offers only `Negotiate` takes the logon session's default credentials, and no helper is asked (`winhttp.c:624-627`). 1.0.17's credential callback orders them so: its helper branch runs only when a user and password are allowed (`transport_support.rs:265-274`). TR1.8 states the precedence and how `Digest` is answered, and S7.2 (1.1.0)'s notes record both.

Replacement:

>     - **Which identity answers (revision 8; TD1, TD2 and TD4).** The transport does what Windows 1.0.17 measurably does (plain-HTTP rows; the HTTPS confirmation is owed). When the challenge offers `Negotiate` or `NTLM`, alone or beside `Basic` or `Digest`, the logon session's default credentials answer and no credential helper is asked, `gh` included. When `Basic` is the only workable scheme, a configured credential helper (TR1.6), `gh` among them, with a credential for the URL the credential will be sent to, answers over `Basic`; GitHub token authentication stays this path. `Digest` is never answered, as in 1.0.17: a challenge whose only workable scheme is `Digest` is refused at once, naming `Digest` and the off switch, and `Digest` is ignored when a supported scheme is offered beside it. If the server rejects the logon identity, the operation stops with an error naming the scheme; the transport neither replays that identity nor turns to a helper credential. A 401 that answers a `POST` is refused with a plain diagnostic and its body is never sent again. `Negotiate` selects a mechanism, and the successful fixture exchanges used NTLM, so they do not qualify Kerberos. TR1.8 states these rules, and S7.2 (1.1.0)'s notes record the differences from 1.0.17.

### Item 2. §3.5, line 178 (TR1.8's fixtures, the OD16 bullet; only this part of the line changes)

Superseded:

> A row whose challenge offers `Negotiate, Basic`, with a configured helper able to answer, the fake `gh` and then a non-gh helper, authenticates with the helper's identity over `Negotiate`, as WinHTTP picks it, with no default-credential offer. An `NTLM`-only row and a `Digest` row, each with a helper able to answer, authenticate with the helper's identity. A `Negotiate`-only row with a helper credential present asserts that the helper is not asked and the logon session authenticates. A row in which one name redirects discovery to the other authenticates at the second. 

Replacement:

> A `Negotiate, Basic` row and an `NTLM, Basic` row, each with a configured helper able to answer, the fake `gh` and then a non-gh helper, assert that the helper is not asked and the logon session authenticates, as do a `Negotiate`-only row and an `NTLM`-only row with a helper credential present. A `Basic`-only row with a helper able to answer asserts that the helper is asked once and answers. A `Digest`-only row is refused at once, naming `Digest` and the off switch, and a `Digest, Basic` row ignores `Digest`. A row whose server rejects the logon identity asserts that the operation stops with an error naming the scheme, with no replay and no helper asked. A row whose server challenges the `POST` after a successful discovery asserts the refusal and that the body is sent once. A row in which one name redirects discovery to the other authenticates at the second; Windows 1.0.17 follows no discovery redirect, so this row is a transport-only assertion and a deliberate difference, to be confirmed over HTTPS. 

The rest of the line stands: the Intranet and Internet names, "Under either name the operation authenticates on the transport, as on 1.0.17 (OD16)", and the final "A row whose server requires channel binding authenticates;" (see section 4).

### Item 3. §3.5, line 226 (TR4.10 "Test first")

Superseded:

>   - **Test first,** TR1.8's `Negotiate` fixture rows on dabeest: both names authenticate on the transport; a `Negotiate, Basic` challenge with a configured helper able to answer authenticates with the helper's identity over `Negotiate`, with no default-credential offer; an `NTLM`-only challenge and a `Digest` challenge, each with a helper able to answer, authenticate with the helper's identity; a `Negotiate`-only challenge with a helper credential present takes the logon session, and the helper is not asked; a discovery redirect from one name to the other authenticates at the second; a server that requires channel binding authenticates.

Replacement:

>   - **Test first,** TR1.8's `Negotiate` fixture rows on dabeest: both names authenticate on the transport; a `Negotiate, Basic` challenge and an `NTLM, Basic` challenge, and a `Negotiate`-only challenge and an `NTLM`-only challenge, each with a helper credential present, take the logon session, and the helper is not asked; a `Basic`-only challenge with a helper able to answer asks the helper once and authenticates; a `Digest`-only challenge is refused at once, and `Digest` beside `Basic` is ignored; a rejected logon identity ends the operation with an error naming the scheme, with no replay and no helper asked; a challenged `POST` is refused and its body is never replayed; a discovery redirect from one name to the other authenticates at the second (a transport-only row, since 1.0.17 follows none on Windows); a server that requires channel binding authenticates.

### Item 4. §3.14, line 367 (OD16's "precedence" bullet) *(beyond the cited lines)*

Superseded:

>     - **precedence:** when the challenge offers `NTLM`, `Basic` or `Digest`, a configured credential helper, `gh` among them, with a credential for the URL the credential goes to answers first, with its own identity, as TR1.8 states. A challenge that offers only `Negotiate` takes the logon session's default credentials, and no helper is asked, as on 1.0.17;

Replacement:

>     - **precedence:** when the challenge offers `Negotiate` or `NTLM`, alone or beside `Basic` or `Digest`, the logon session's default credentials answer and no helper is asked, as on 1.0.17. When `Basic` is the only workable scheme, a configured credential helper, `gh` among them, with a credential for the URL the credential goes to answers, as TR1.8 states (revision 8);

### Item 5. §3.11, line 286 (S7.2 (1.1.0) notes; only this phrase changes) *(beyond the cited lines)*

Superseded phrase: "the hazard of the default-credential offer to any host (OD16) and the credential helpers' precedence over it,"

Replacement phrase: "the hazard of the default-credential offer to any host (OD16) and which identity answers which scheme (§3.5),"

### Item 6. §3.19, line 471 (TR1.6's "Schemes" bullet; only these sentences change) *(beyond the cited lines)*

Superseded:

> A helper's credential answers `Basic`. On Windows it also answers `Negotiate`, `NTLM` and `Digest`, as 1.0.17's WinHTTP does with a helper's user and password (`winhttp.c:139-151`, `:618-640`). TR1.8 designs that part.

Replacement:

> A helper's credential answers `Basic`. On Windows it answers nothing else: 1.0.17 uses the logon session for `Negotiate` and `NTLM`, never answers `Digest`, and asks a helper only when `Basic` is the only workable scheme (revision 8; TR1.8 states it).

The sentence after it, on macOS and Linux, is unchanged.

### Item 7. The status block and the changelog

The date line gains "revision 8, 2026-10-10". The status block gains one bullet, "Revision 8 is a DRAFT, pending a skim review.", and the changelog gains one entry. Both state what is in this package and that revision 7 remains unreviewed. No earlier sentence of either is edited.

### Companion edit: the user guide, lines 75 to 82 (`GwzTransportWindowsUserGuide-DRAFT.md`)

The decision list also cites the guide's lines 76 to 82, and the guide is in this repository, so its paragraph is reconciled the same way. It is a Surface input at step 2.4, not an amendment text, so a skim review of the amendment need not read it; it is listed so that nothing superseded is left. Superseded:

> GWZ starts HTTPS discovery without credentials. If the server offers NTLM,
> Basic or Digest, your configured git helper is asked first. When the server
> also offers Negotiate, a returned helper identity uses Negotiate. A
> Negotiate-only server uses your Windows logon identity without asking a helper.
> If no helper identity is available and the server offers Negotiate or NTLM,
> GWZ can use your Windows logon identity. A helper timeout or cancellation ends
> the operation. A credential the server rejected is not replaced by another
> identity within that operation.

Replacement:

> GWZ starts HTTPS discovery without credentials. If the server offers
> Negotiate or NTLM, alone or beside Basic or Digest, GWZ uses your Windows logon
> identity and does not ask your configured git helper. If Basic is the only
> scheme the server offers that GWZ can use, your configured git helper is asked;
> a GitHub token held by a helper is sent this way. GWZ never answers Digest: a
> server that offers only Digest is refused at once, and Digest is ignored when
> the server offers another scheme. If the server rejects your logon identity, the
> operation stops with an error naming the scheme; GWZ does not try it again or
> turn to a helper credential. A helper timeout or cancellation ends the
> operation. A server that challenges the upload step of a fetch is refused with a
> message, and GWZ does not send the data again.

## 4. What was deliberately left unchanged

- **Line 164, the trigger.** "After the transport's one validated discovery redirect" and "A redirect that arrives during the exchange ends it" agree with TD3: a discovery redirect is followed; credentials do not move across an exchange. Whether "one" is a cap is not decided by TD3 (see the design's open points).
- **Line 168 and the channel-binding sentence in line 178 and line 226.** TD9 (the Extended Protection rows) is approved but its rows have not run. The sentences stay as requirements; nothing claims a result.
- **Line 171, macOS and Linux.** The step-2.3b rows answered it: 1.0.17 refuses `Negotiate` there, as the sentence's "if it does" branch anticipates. The sentence is conditional and still true; no pointer was added.
- **Line 180, the review sentence** (the forced-authentication hazard, Pageant, the machine proxy, the default-credential exchange with the machine proxy).
- **The changelog entries of revision 5** (lines 700, 704 and 711) that describe the old precedence. They record what that revision and its skim reviews said, and are history. The status bullet for revision 5 (line 10) does not state the precedence.
- **Steps, phases, dependencies and the sketch.** No step is added, removed or reordered. TR4.10's size budget and TR1.8's timing are untouched.

## 5. For the reviewer

1. Does each replacement say what the decisions say, and no more? In particular: "the logon session answers `Negotiate` and `NTLM`, alone or beside `Basic` or `Digest`"; "the helper is asked only when `Basic` is the only workable scheme"; "`Digest` refused when alone, ignored beside a supported scheme"; "a rejected logon identity ends the operation, with no replay and no helper credential"; "a challenged `POST` is refused, body not replayed".
2. Is any mechanism proposed that the decisions did not approve? The author found none: every sentence states an outcome that 1.0.17 has, or a refusal with a message, and TD14 asks for no new mechanism.
3. Is any sentence in the amendment, outside the six items, still contradicted by TD1 to TD4? A search for "helper" together with "Negotiate", "NTLM" or "Digest", and for "precedence", found only the sites above, the revision 5 history lines (700, 704 and 711), and line 468 (a parity reference to the credential callback, which is not a claim about schemes).
4. Is the HTTPS qualification honest? The text says the rows were plain HTTP and that the HTTPS confirmation is owed.
5. Does the "transport-only assertion" wording for the redirect row match TD3 ("follow ... on Windows"; "HTTPS confirmation remains owed")?

## 6. What this package does not do

It runs no review, records no verdict, and edits no file other than the amendment, the user guide paragraph above, the TR1.8 design revision and this package. It does not change the amendment's accepted status, which still rests on revision 2's text.
