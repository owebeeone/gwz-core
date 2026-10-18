# GWZ 1.1.0 help audit

Status: **DRAFT, 2026-10-17** (plan step C5 / S7.2: "configuration and help match the shipped switches"). It lists what the 1.1.0 build ships for users to type or set, the help text, the default, and whether the help matches the code. It lists mismatches and fixes none of them: no product code or help text was changed. It is finalized after C4, because C4 turns the candidate help into the only help.

## 0. Method and limits

- **Read from source, not rendered.** I read the clap definitions and the help constants at this lane's base (gwz-cli `src/globalargs/{parser,transport,transport_help,retry}.rs`, `src/help.rs`, `src/fetch_long.rs` and the `pull` and `push` equivalents; gwz-py `src/gwz/cli_shared.py`, `client.py`, `native/src/route/transport.rs`; gwz-core's default constants). I did not build or run the candidate binary, so a help string below is the constant's text, not captured output. A pin test exists for most of the strings (cited where it matters).
- **Which help is the product's.** gwz-cli has two help shapes today: the candidate shape (`cfg(gwz_transport_candidate, any(unix, windows))`) and the ordinary shape. After C4 the candidate shape is the only one. This audit judges the candidate shape. Findings that exist only in the ordinary shape are marked *moot after C4*.
- **Windows-only help.** `windows_verbose_help!` and `windows_ssh_timeout_help!` add Windows paragraphs to `--verbose` and `--ssh-timeout` only in a Windows build.
- **Scope.** All global switches of `gwz` and `gwz-py`, the transport-related settings (environment variables, git configuration keys, host settings), and the retry sentence of the network commands. Per-command flags the transport does not touch (selectors, `commit`, `branch`, `merge`, `local`, and so on) are unchanged by 1.1.0 and are covered by the generated reference check (`scripts/generate_cli_reference.py --check`).
- **Defaults are provisional** where marked *S5.4*: step S5.4 may change a construction default.

## 1. gwz switches (global)

| Switch | Short help (candidate shape) | Default in code | Source of the default | Help matches code |
|---|---|---|---|---|
| `--transport gwz\|native` | Transport for network operations: gwz (the default) or native, libgit2's own, as gwz 1.0 used | `gwz` | `transport_setting::resolve` (flag, then `GWZ_TRANSPORT`, then global `gwz.transport`, then default) | yes (long help: see H9 for the commands it prints) |
| `--max-retries <n>` | Extra attempts after a failed setup or a refusal (0 = no retries, and --max-per-host is never lowered; default 3) | 3 extra attempts (4 in all); any value up to 4294967295 accepted | `setup_retry::DEFAULT_MAX_RETRIES = 3`; `parse_max_retries` | yes. Pinned by `retry.rs` `pins` |
| `--ssh-timeout <secs>` | Per-attempt stall limit for setup and for a body read; stalled setup is retried (0 = no timeout, default 9; 3 with --transport native) | 9; 3 with native; 0 disables | `lib.rs` `report.timeout_seconds` (`transport.rs`); gwz-core `DEFAULT_SERVER_TIMEOUT_MS` 9000 | the numbers match; the short help's "for setup" does not (H2); one sentence in the long help is doubtful (H3) |
| `--jobs <n>` | Global ceiling on concurrent member operations (default 100; 50 with --transport native) | 100; 50 with native | `resolve_jobs::DEFAULT_JOBS`; `prepare_transport` `get_or_insert(50)` | yes (*S5.4*) |
| `--max-per-host <n>` | Maximum (ceiling) concurrent member operations to any one hostname (default 32; 8 with --transport native) | 32; 8 with native | `resolve_per_host::DEFAULT_MAX_PER_HOST`; `prepare_transport` `get_or_insert(8)` | yes (*S5.4*). The adaptive lowering is described in the long help and pinned in `retry.rs` |
| `--verbose` | Show transport authentication diagnostics | off | | yes. The long help also says it names the transport selected and what selected it, and that a followed redirect prints only with it; `transport.rs` `verbose_line` does the first |
| `--progress-interval <ms>` | Min milliseconds between progress events per repo (0 = every update) | 100 | `DEFAULT_PROGRESS_MIN_INTERVAL_MS` | yes |
| `--identity <PATH>` | Use only this SSH private-key file; no agent fallback | none | | partly (H11) |
| `--remote-identity <NAME=PATH>` | Override SSH identity for this remote name across selected repositories; repeatable | none | | partly (H11) |
| `--remote <name>` | Select the git remote name | per command | | unchanged by 1.1.0 |
| `--json`, `--jsonl` | Render one JSON response; render newline-delimited JSON events | off | | yes. The `meta.transport_setting` key they gain is in `docs/MachineOutput.md`, not in the help |
| `--dry-run`, `--partial`, `--force`, `--sync`, `--root`, `--target` family, `--all` | unchanged | | | unchanged by 1.1.0 |

The root help (`gwz --help`, `src/help.rs` `ROOT_HELP`) lists only `--root`, `--target`, `--no-target`, `--remote`, `--json`, `--verbose`, `--dry-run`, `-h`, `-V` and points to `gwz help COMMAND`. It does not name `--transport`, `--max-retries`, `--ssh-timeout`, `--jobs` or `--max-per-host` (H4). Each command's own help lists all of them, because the flags are `global = true`.

### 1.1 Network commands' long help

`fetch`, `push` and `pull` end their long help with the sentence from `retry_sentence!`: "A connection that fails during setup is retried before that repository counts as failed. See --max-retries and --ssh-timeout. A repository that succeeds on a later attempt counts as having answered." (pinned in `retry.rs`). `clone`, `materialize`, `init` (from sources), `repo clone` and `tag` (when it reaches a remote) also take the network route and the retry; their long help carries no such sentence (H5, an observation).

## 2. Settings that the build reads

| Setting | What it does | Default | Where it is documented | Help matches code |
|---|---|---|---|---|
| `GWZ_TRANSPORT` (`gwz` or `native`) | transport for network operations, above the git setting, below `--transport` | unset | `--transport` long help; `docs/commands/auth.md`; the Windows guide draft; gwz-py README | yes |
| `gwz.transport` in the global git configuration (`~/.gitconfig`, `$XDG_CONFIG_HOME/git/config`) | same, lowest precedence | unset | same | yes. No `includeIf`, no system file, not `GIT_CONFIG_GLOBAL`; a repository-local value is ignored with a note |
| `GWZ_URL_SCHEME` and `--url-scheme manifest\|ssh\|https` (`clone`, `materialize`) | URL form to clone from | `manifest` | `clone` and `materialize` long help | unchanged by 1.1.0 |
| `gwzSshIdentity` (per remote), `gwz auth identity <remote> [--set PATH \| --unset]` | saved SSH key file | none | `docs/commands/auth.md` | partly (H11, H6) |
| `SSH_AUTH_SOCK` | Unix: the agent socket. Windows: a local pipe `\\.\pipe\<name>` only | unset | Windows paragraph of `--verbose`; Windows guide draft | yes (Windows build only) |
| `HOME`, `HOMEDRIVE` + `HOMEPATH`, `USERPROFILE` (Windows) | the SSH home for `known_hosts` and `~/` key paths | first that exists | Windows guide draft | not in any help text (H10) |
| `GIT_SSL_CAINFO`, `SSL_CERT_FILE` | extra CA file for HTTPS (Windows: both; macOS: both; Linux: `GIT_SSL_CAINFO`, and OpenSSL's own reading of `SSL_CERT_FILE` and `SSL_CERT_DIR`) | unset | Windows guide draft only | not in any help text (H10) |
| `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` (and lowercase) | HTTPS proxy on macOS and Linux. Ignored on Windows | unset | Windows paragraph of `--verbose` says they are ignored on Windows; nothing says they are read elsewhere | partly (H10) |
| WinHTTP machine proxy (`netsh winhttp show proxy`) | HTTPS proxy on Windows | direct | Windows paragraph of `--verbose` | yes |
| git `credential.helper` (system, global, XDG; `GIT_CONFIG_*` overrides of the session) | HTTPS Basic credentials | none | `docs/Troubleshooting.md` "HTTPS Credential Failure" | yes |
| `known_hosts` | host-key check; an unlisted or changed host key is refused | the SSH home's `.ssh/known_hosts` | `docs/Troubleshooting.md`; Windows guide draft | yes |

## 3. gwz-py

The Python package has no `--transport` and no `--max-retries`; `GWZ_TRANSPORT` and `gwz.transport` are its only transport selectors, read at the start of each network operation. `Client` takes no transport or retry parameter.

| Switch or setting | Help text | Default in code | Help matches code |
|---|---|---|---|
| `gwz-py --jobs` | Global ceiling on concurrent member operations; when omitted, 100 on gwz or 50 on native in the transport candidate; an explicit positive value overrides the default | 100; 50 on native (`native/src/route/transport.rs` `get_or_insert(50)`) | the numbers match; "in the transport candidate" goes stale at C4 (P1) |
| `gwz-py --max-per-host` | Max concurrent connections to any one host; when omitted, 32 on gwz or 8 on native in the transport candidate; an explicit positive value overrides the default | 32; 8 on native | the numbers match; the help calls it a number of connections and does not say it is a ceiling that gwz may undercut (H12); stale phrase (P1) |
| `gwz-py --ssh-timeout <secs>` | Abort a stalled SSH/network read after N seconds, on gwz's transport and, with GWZ_TRANSPORT=native, as libgit2's connect and read timeout (0 = no timeout, default 9 on either transport) | 9 on either transport (core default 9000 ms; the CLI calls `configure_transport_timeout` only when given) | yes. Differs from gwz-cli's native default of 3 on purpose (one process clock) |
| `gwz-py --verbose` | Show transport authentication diagnostics in human output | off | yes |
| `gwz-py --progress-interval`, `--json`, `--jsonl`, `--identity`, `--remote-identity` | as gwz-cli's, shorter | 100 / off | yes (H11 for the identity pair) |
| `TRANSPORT_HELP` (the epilog text) | "In the 1.1.0 transport candidate, network operations select gwz (default) or native..." | gwz | commands are right; wording goes stale at C4 (P1) |
| `Client(max_connections_per_host=...)` | docstring: None leaves the limit unset: 32 on gwz's transport, or 8 when native is selected in the transport candidate | 32 / 8 | yes; stale phrase (P1) |
| `Client.configure_transport_timeout(seconds)` | README: before the first operation; zero disables | 9 | yes. A later call with another value is refused once a backend exists (core test `runtime_timeout_refuses_changes_after_backend_creation`) |

## 4. Mismatches and gaps

Confirmed means the text and the code or a second text were read side by side. A hypothesis says what would settle it.

| Id | Where | Finding | Evidence | Kind | Impact | Suggested change |
|---|---|---|---|---|---|---|
| H1 | `gwz-cli/docs/CLI.md` | The generated reference has no `--transport` and no `--max-retries`, says "Defaults to 100" and "Defaults to 32" with no native caveat, and "Defaults to 9" for `--ssh-timeout`. It is generated from the ordinary build (`scripts/generate_cli_reference.py` runs the `generate_cli_docs` example with no cfg) | `grep` of the file for `--transport` and `--max-retries <n>` finds no option entry | confirmed | hosted docs (built from the release tag) would omit the two flags that 1.1.0 adds | after C4: `generate_cli_reference.py --write`. Also decide which host generates it, because the Windows-only paragraphs exist only in a Windows build (H13) |
| H2 | `gwz-cli/src/globalargs/transport_help.rs` line 42 | The candidate short help for `--ssh-timeout` says "Per-attempt stall limit for setup and for a body read". The ordinary short help (line 52) and its pin `tests/g09.rs:256` say "for SSH setup". Commit `5f2c159` (TR2.1) corrected the ordinary shape only. HTTPS setup has no stall clock, so "setup" overstates | text of the two constants; the pin is in the ordinary-only `cfg_if` arm | confirmed | `gwz fetch -h` in the 1.1.0 build tells an HTTPS user that setup has a stall limit it does not have | change line 42 to "SSH setup" and add the candidate-shape pin |
| H3 | `--ssh-timeout` long help, both shapes, and `CLI.md` | "A host-key prompt can add up to 120 seconds on each attempt." I found no host-key prompt in the transport build: an unlisted or changed host key is refused with a message (`ssh_network.rs`, `ssh_host_key`), and libgit2 does not prompt either. The 120 second figure is a credential-helper interaction allowance, which the same paragraph states separately | `grep` for a prompt in `git/endpoint/ssh_host_key*` and `ssh_network.rs` finds none; Surface review `GwzTransportCredentialHelpers-ReviewSurface.md` line 97 quotes the sentence as "its stated local-interaction exception" | hypothesis (the owner of the SSH clocks should confirm no prompt can occur) | a user may wait for a prompt that never comes, or believe a fail can take 120 s longer than it can | remove the sentence, or say what prompt it means |
| H4 | `src/help.rs` `ROOT_HELP` | The root help names none of `--transport`, `--max-retries`, `--ssh-timeout`, `--jobs`, `--max-per-host` | file text | confirmed (design choice unknown) | a user looking for the escape hatch in `gwz --help` does not find it; `gwz fetch -h` does list it | consider adding `--transport` to "Common options"; the messages that mention `--transport native` already say where it goes |
| H5 | long help of `clone`, `materialize`, `init`, `repo clone`, `tag` | Only `fetch`, `push` and `pull` carry the retry sentence; the other network commands run on the same retried route | `grep retry_sentence!` finds three uses | observation | none that is wrong; an incomplete cue | optional |
| H6 | `gwz-cli/docs/commands/auth.md` (`--ssh-timeout` paragraph) | Describes `--ssh-timeout` only as "the native connection/read timeout ... at process startup"; on gwz's transport it is the stall limit of an SSH setup attempt and of a body read | the paragraph and the candidate long help | confirmed | the default transport's meaning of the flag is missing from this page | reword the paragraph to the long help's meaning, native as the exception |
| H7 | `docs/Concepts.md:133`, `docs/Troubleshooting.md:132,193,343`, `docs/MachineOutput.md:203,253,282`, `docs/commands/auth.md:56` | Future-tense and "candidate" phrases ("planned for 1.1.0", "the transport candidate build") that become false when 1.1.0 ships | `grep -n -i "candidate\|planned for 1.1"` | confirmed | wrong tense in the released docs | reword at C4; the Releases entry already uses the 1.1.0 tense |
| H8 | `gwz-cli/README.md:77-83` (after "License"), `docs/HostPackaging.md:44-49`, `gwz-py/README.md` last paragraph | Statements that the Windows endpoint is "not enabled" ("this draft does not enable that endpoint", "full Windows transport/release remains NO-GO"). The qualification switch was retired in Phase 5 and 1.1.0 ships Windows | file text; `GwzV110RemainingPlan.md` §1 (WPP 5.1 to 5.4 done) | confirmed that the text is stale; whether each sentence is still true for some narrower claim is the owner's call | a reader of the README is told the Windows transport is off | rewrite or delete at C5 finalization (these are the owners of the claim, so I left them) |
| H9 | `--transport` long help | The commands it prints use `$HOME`, which works in sh and PowerShell, not in `cmd.exe`. Which home the transport setting reads on Windows (`HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE`, as the SSH home does, or one fixed rule) is not stated in the help | long help text; the Windows guide draft prints the path in double quotes | hypothesis for the home question; confirmed for `$HOME` and `cmd` | a Windows `cmd` user gets a path that does not expand | the Windows guide's form (the resolved path in double quotes) is printed by the error messages; consider the same in the help or add "PowerShell or sh" |
| H10 | help text and docs | No help or CLI doc names `HOME`/`HOMEDRIVE`/`HOMEPATH`/`USERPROFILE` (Windows SSH home), `GIT_SSL_CAINFO`/`SSL_CERT_FILE`, or says that `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` are read on macOS and Linux (only that Windows ignores them). The Windows guide draft has the first two | grep of `src` and `docs` | confirmed | settings the build reads are documented only in a draft in dev-docs | when the Windows guide moves into the hosted docs, link it from `--verbose` and from Troubleshooting; add a short "Settings" section |
| H11 | `--identity`, `--remote-identity` | The help says "Use only this SSH private-key file; no agent fallback" and nothing else. In 1.1.0: a passphrase-protected key file is refused on every platform; on Windows only an unencrypted RSA PEM file is read; exact selection among keys in an agent is not offered (`exact_agent_identity`, Windows decision D30 pending) | `Troubleshooting.md` "An SSH Key File Is Protected By A Passphrase"; `auth.md`; Windows guide draft | confirmed (the help is not wrong; it is incomplete) | the refusal message is the first the user learns of the limit | add one sentence and a pointer to Troubleshooting |
| H12 | `gwz-py --max-per-host` help | Says "Max concurrent connections to any one host". gwz-cli's help calls it a ceiling and says gwz may use fewer. gwz-py has the same adaptive limit (it emits the `gwz: ... asked gwz to wait` records) | `cli_shared.py` lines 322 to 330; README "Choosing the network transport" | confirmed | gwz-py users are not told the number can be lowered | align with gwz-cli's wording |
| H13 | `CLI.md` and any generated reference | The Windows-only help paragraphs (agent sources, machine proxy, `http://` warning, Pageant wait) exist only in a Windows build, so a reference generated on macOS or Linux can never contain them | `transport_help.rs` `cfg(windows)` arms | confirmed (by design) | the hosted docs omit Windows help text that the Windows binary shows | publish the Windows guide beside the reference (H10) |
| H14 | defaults of `--jobs`, `--max-per-host`, `--ssh-timeout` | The help states 100 / 32 / 9 as defaults. They are right at this base. S5.4 may change a construction default | `GwzV110RemainingPlan.md` B7 | limitation | the help and the Releases entry need the same edit if S5.4 changes them | recheck after S5.4 |
| H15 | ordinary-shape help (moot after C4) | The ordinary build already shows 100 / 32 / 9 as the defaults with no `--transport native` to restore 50 / 8 / 3, because gwz-core's constants changed ahead of the switch | `resolve_jobs.rs`, `resolve_per_host.rs`, `transport_support.rs` against the `v1.0.17` tag (50 / 8 / 3000 ms) | confirmed, moot after C4 | none after C4 | none |

## 5. Texts that go stale when C4 retires the switch (P-list)

- P1 gwz-py: `src/gwz/cli_shared.py` (`TRANSPORT_HELP`, the `--jobs` and `--max-per-host` help), the `Client` docstring, README "Choosing the network transport" ("In the 1.1.0 transport candidate") and the README's closing paragraph on the candidate native HTTPS composition.
- P2 gwz-cli: the items under H7 and H8.
- P3 `src/globalargs/transport_help.rs` and `transport.rs`: the `else` (ordinary) arms are removed by C4; the candidate arm's doc comments ("Candidate transport help ...") are reworded. `scripts/candidate_switch_inventory.txt` and `scripts/test_candidate_switches.py` track the switch sites.

## 6. Questions for the owners

1. H3: is there any host-key prompt in either route? If not, delete the sentence.
2. H4: is the root help meant to be the short list it is, or should `--transport` join it?
3. H9: which home does the transport-setting reader use on Windows?
