# TR1.8 freeze review (Safety), final re-verdict

**Tuple.** Root `a7dc0859`, gwz-core `b6ef02af`, gwz-core-evidence `928f6d4`, gwz-transport `6146bd58`, gwz-cli `21d62310`, gwz-py `5950ba38`, gwz-sspi `73de8c8e`, git2-rs `d13951f7`. All match at start and end.
- The only untracked file is the out-of-scope `GwzRemoteTransportBugReport.md`.
- I modified nothing.
- File references: D = design, G = guide, P = plan, all under `gwz-core/dev-docs/`.

## 1. Closure of my round-2 findings

| ID | Status | Evidence, and the re-run sequence |
|---|---|---|
| **R2-1** (P2, architectural; operator chose (b) as D5) | **closed** | D `:115-119` (Bound and cost), L20, register item 14, §14 items 13 (T6) and 14, G `:140-162`, P `:423` and `:425`, decisions file `:39`. The clock amendment is unchanged, so no route is admitted that the amendment forbids. **Re-run:** a confirm-every-use Pageant prompt runs past the 9 s stall. The attempt fails with T6, which says the prompt may still be open. Each retry prompts again, and the guide says so. `--ssh-timeout` and `--transport native` are named as the ways out. Nothing hangs, nothing is disclosed, and there is no read after timeout. The cost is stated in every place D5 lists. Plan 3.6 tests the stall bound in both directions (`:425`). |
| S1/S8 partial (the owed test) | **closed** | D `:185` names both refusal points, `prepare.rs:501` and `native/exchange.rs:338`. P `:522` and `:524` carry one test that covers both and asserts that no credential goes to the new location. §14 item 13 has the matching row. |
| S3 residue | **closed** | D `:165` states one rule; D `:423` ("Claims kept") no longer says the forms are "refused before any open". L8 agrees. |
| S5 trace | **closed** | P `:398-399` admits exactly `\\.\pipe\<name>`. The grammar's refusal rows name `SSH_AUTH_SOCK`. |
| S7 wording | **closed** | D `:208`: "per operation, not per exchange". |
| R2-2 (D2 warning timing) | **closed** | D `:173`, §14 item 13 (T2), P `:482` and `:484`. The warning is emitted at route selection, before the first request, on every operation, including under `--transport gwz`, `--verbose`, quiet mode and the Python entry by default. Nothing suppresses it. **Re-run:** an `http://` remote cannot reach cleartext without the warning first. |
| R2-3 | **closed** | P `:524` (step 4.9), P `:398-399` (step 3.3), P `:506` (step 4.7 no longer points to the native transport). |

## 2. The single rule for proxy forms with no executed row (D `:165`, L8)

I ran each unmeasured form through the rule.

**Credentials go nowhere unintended.** CONNECT carries no credential. Every 407 is refused. Origin credentials travel only inside TLS that has been validated against the stated trust sources (D4).

**Traffic goes direct only where a stated rule says so:**
- **IP-literal https origin.** The explicit bypass is matched as text, and names are never resolved. Only the implicit set (localhost, 127/8, `[::1]`, this machine's own address) goes direct, and it ends on this host.
- **Bare entry with no port.** The default port is not measured. A wrong guess sends a CONNECT to the configured proxy host, which either fails or tunnels, but never goes direct and carries no credential.
- **Redirect through a proxy.** The bypass is re-evaluated on the new host. That is direct only when a rule the administrator wrote, or a loopback or own-address rule, matches. The same redirect goes direct with no proxy at all, so the proxy adds no new path.
- **This machine's own address as an https origin.** It goes direct, on-host.
- **No https entry** (`http=`-only, `socks=`-only, and by extension any mapping without an https entry). Refused by D1.
- **Values outside the grammar.** Refused.
- **Unresolvable or silent proxy.** The operation ends.

The drafter's qualifier holds as written. Every direct outcome comes from a stated bypass rule, and the only unmeasured one, the implicit set applied to https, keeps traffic on this machine.

**Verdict on the rule: safe.** It fails toward the proxy or toward refusal, never silently direct off-host, and it never misdirects credentials. Whether admitting a form by WinHTTP's rule without an executed row satisfies the GO rule is Consistency's question; the direction of failure is safe.

## 3. New findings

**F3-1 (P2; not architectural): the https→http refusal text tells the user how to downgrade, to an address the server chose.**
- **Where:**
  - Design §14 item 13, text T5 (D `:407`, owed to step 4.9).
  - G `:452-458`: "…if you accept the risk, set the remote to the http:// address, which GWZ carries on the native transport with a warning."
- **Violated clause:**
  - HTTPS design §5: "Reject HTTPS downgrade before any new host/helper access".
  - D2's purpose: no silent cleartext.
- **State sequence:**
  1. An https origin, misconfigured or hostile, answers discovery with `302 Location: http://<host2>/…/info/refs?service=git-upload-pack`.
  2. The transport refuses, correctly, and prints T5 with `<new url>`.
  3. The user does what the message says and sets the remote to that http:// address. The address is server-chosen and may be a different host.
  4. On the native route, a Basic challenge asks the git helper for `http://<host2>`. Git Credential Manager prompts for the user's password, and the password or helper token travels in clear to everyone on the path.
  5. An NTLM challenge gets the logon session's response with no channel binding, which an on-path attacker can relay.
- **Impact:** the refusal itself coaches the downgrade it exists to prevent. The commonest realistic trigger is a benign misconfiguration on an untrusted network, which leads straight to token exposure. T2's warning arrives after the user has already followed the advice.
- **Correction (text only):** T5 and G `:455-458` state the refusal and the reason, and say to ask the server's administrator. Drop the "set the remote to the http:// address" advice, and print `<new url>` only as untrusted quoted text, never as something to act on. The guide may say, separately and not in the refusal path, that an `http://` remote sends credentials in clear.

**F3-2 (P3): the unbounded Pageant wait has no stated timeout value.**
- **Where:** D `:119`. "`--ssh-timeout 0` removes both", and T6 recommends it.
- **Gap:** with both clocks off, the value passed to `SendMessageTimeoutW` is not stated. A `uTimeout` of 0 makes the call time out at once, so every Pageant request would fail: the opposite of what T6 promises.
- **Correction:** state the mapping (a finite cap, never 0), and that Ctrl-C returns while the blocked thread retires, as "Cancel may return…" already allows.

**F3-3 (P3): Pageant timeouts multiply prompts.**
- **Where:** G `:157` discloses it ("each retry asks Pageant again").
- **Gap:** with `--max-retries` 3 and up to 32 connections per host, a confirm-every-use agent can stack many prompts. A confirmed stale prompt signs only a dead session hash, so this is a usability cost, not a safety one.
- **Suggestion (optional, 3.6):** do not retry an attempt whose agent request timed out.

**F3-4 (P3): T1 points to the native transport as a way around the administrator's proxy.**
- **Where:** T1, G `:297-304`. The fallback is explicit and conditional ("if a direct connection is acceptable"), and TLS still applies.
- **Note:** acceptable, but G's own "Do not reset an administrator's proxy merely to bypass a GWZ error" sits in tension with it. Consider naming the administrator's fix first and only.

**Architectural root causes:** none new. R2-1's root cause is closed by D5 (b).

Verdict: NO-GO

F3-1 is a P2 that this remediation introduced. It is a two-sentence text fix to T5 and G `:455-458` and needs no design change. Once that wording is applied and checked, nothing on the Safety axis stands in the way of GO.