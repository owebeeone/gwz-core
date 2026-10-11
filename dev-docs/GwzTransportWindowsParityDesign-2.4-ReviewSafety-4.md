# TR1.8 freeze review (Safety), round 3: F3-1 check

**Tuple.** Root `48b61b3f`, gwz-core `6fae490c`, gwz-core-evidence `928f6d4`, gwz-transport `6146bd58`, gwz-cli `21d62310`, gwz-py `5950ba38`, gwz-sspi `73de8c8e`, git2-rs `d13951f7`. All match at start and end. I modified nothing.

| ID | Status | Evidence, and the re-run |
|---|---|---|
| **F3-1** (P2) | **closed** | T5 at D:413-417 and G:466-475; owed row at D:381; step 4.9 at P:525; Annex S.2 at K:235. **Re-run:** the server sends `302` to `http://<host2>/…`. The transport refuses and prints T5. T5 quotes `<new url>`, calls it untrusted, says "do not use it", and points to the server's administrator. G adds that the address "is the server's choice, not something to copy into a remote". I searched for "set the remote to", "http:// address" and "accept the risk" across D, G, K and P: the only hits are the changelog lines recording the withdrawal. The remaining `http://` text (G:13-28) is the general warning about a remote the user configured as `http://`, and it says "Use `https://` where you can". Nothing invites the user to act on an address the server chose, so the sequence no longer reaches cleartext. |
| F3-2 (P3) | **closed** | D:116, D:122 and D:385; P:423-425; G:144-146; K:253. The `SendMessageTimeoutW` timeout is never 0, including under `--ssh-timeout 0`. It has a finite cap that step 3.6 fixes and tests, and Ctrl-C returns while the blocked thread retires. |
| F3-3 (P3) | **closed** | P:426: not retrying a timed-out Pageant attempt is an optional consideration. It is correctly framed as usability, not safety. |
| F3-4 (P3) | **closed** | D:363, D:389-395; G:296-319; P:501; K:241. T1 names the administrator's `https=` entry first. `--transport native` comes only as the conditional "if a direct connection is acceptable", with the note that it changes no setting and fails where direct connections are blocked. |

**This round's other text changes, checked on the Safety axis:**
- **P3-A (bypass wording):** "goes to the proxy, goes direct only where a bypass rule above matches (this computer's own address included), or GWZ refuses". This matches the rule I judged safe last round: nothing goes direct off-host on a guess, and no credential is misdirected.
- **P3-B (CA-file precedence):** the text says one file, `GIT_SSL_CAINFO` if non-empty, otherwise `SSL_CERT_FILE`. That matches MAIN (`endpoint_environment.rs`: `value()` returns the first non-empty name in `CA_FILE_VARIABLES`). The trust hazard is still stated, and the refusals are attributed correctly to measured runs or to the code.
- **P3-C to P3-F:** owners, help text and message wording only. None widens trust, credentials or routing.

**New findings:** none at P0 to P2, and no architectural root cause.

Verdict: GO