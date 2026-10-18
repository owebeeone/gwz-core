# Remote transport: bug report from the 2026-09-23 measurements

Three defects found while measuring `gwz fetch` for the transport announce. Evidence is in
`garnets-wz/dev-docs/gwz-go-to-market/research/gwz-fetch-scale.md` (private); the verbatim output is copied
here. Builds: `gwz` 1.0.17 (released, `~/.cargo/bin/gwz`) and `gwz-alpha` 0.2.0-alpha.transport
(`~/.cargo/bin/gwz-alpha`). Machine: Gianni's Mac, github.com, `gh` 2.x logged in as owebeeone.
The pool-capacity issue is NOT here; it has its own brief, `GwzRemoteTransportPoolCapacity.md`.

---

## BUG 1 - alpha: https credential is resolved but never offered; private members fail

**Build:** gwz-alpha 0.2.0-alpha.transport only. gwz 1.0.17 is correct.
**Severity:** release blocker for the transport build (any workspace with a private https member fails).

**Reproduction**
```sh
# helper already configured: credential.https://github.com.helper = !/opt/homebrew/bin/gh auth git-credential
gwz clone --url-scheme https git@github.com:owebeeone/gwz-dev.git /tmp/gwz-dev-https   # 1.0.17: exit 0, 15.5 s
cd /tmp/gwz-dev-https
gwz fetch --verbose        # 1.0.17: Noop, exit 0
gwz-alpha fetch --verbose  # alpha: Partial, exit 1, every run
```

**Observed (alpha), member row, every run:**
```
mem_gwz_core_evidence  gwz-core-evidence  failed     RemoteRejected: member 'mem_gwz_core_evidence' at 'gwz-core-evidence': HTTPS endpoint request failed: Authentication
```
`--verbose` transport line for that member:
- 1.0.17: `credential=helper source=ambient offered=true authenticated=yes`
- alpha:  `credential=helper source=ambient offered=false authenticated=unknown`

Both builds emit 13 transport rows, all `operation: fetch`, `selection_source: ambient`; the 12 public repos show
`credential_method: unknown, credential_offered: false` in both (public https needs no credential). Only the
private member differs: the alpha resolves `credential_method: helper` but `credential_offered` stays false.

**Expected:** the alpha offers the helper credential on the https path exactly as 1.0.17 does.
**Where to look:** the https endpoint request path in the alpha transport (`GwzRemoteTransportHttpsDesign.md`,
`GwzRemoteTransportHttpsH1.md`/`H2.md`): the credential is looked up (the row says `helper`) but the request is
sent without it, or the 401 challenge is not answered with a retry carrying it.

---

## BUG 2 - alpha: `Partial` result carries an empty top-level `errors` array

**Build:** observed on gwz-alpha 0.2.0-alpha.transport (the run above). Not checked on 1.0.17 because 1.0.17
produced no Partial on https; check it with the ssh-timeout repro in BUG 3.
**Severity:** high for machine callers - anything reading `errors` to decide success misses the failure.

**Observed:** `gwz-alpha fetch --json` returns `aggregate_status: "Partial"`, exit 1, `errors: []`, and the only
record of the failure is the member entry (`RemoteRejected ... Authentication`). MachineOutput.md documents
`Partial` -> exit 1; it should be checked what it promises for `errors` on a Partial and either the field or the
doc fixed so a member failure is visible at the top level.

**Expected:** a Partial result lists each failed member's error in the top-level `errors` array (or the contract
says explicitly that member failures live only in member rows, and the docs and the Python driver agree).

---

## BUG 3 - 1.0.17: default `--ssh-timeout 3` is shorter than a GitHub SSH handshake under load; fetch goes Partial

**Build:** gwz 1.0.17. Not reproduced on the alpha (38 runs, no partial), which shares connections.
**Severity:** medium - intermittent, exit 1 with a correct workspace, worse with more members per host.

**Reproduction:** a workspace of 16 github.com members over SSH, defaults (`--jobs 50 --max-per-host 8`):
```sh
for i in 1 2 3 4 5; do gwz fetch >/dev/null; echo "exit $?"; done
```
Observed 2 of 5 attempts exit 1 with empty stdout and one stderr line:
```
gwz: gwz operation returned partial
```
`gwz fetch --json` on a failing attempt shows two members with `RemoteRejected` and
```
failed to start SSH session: Timed out waiting on socket
```
Separately, 12 scripted back-to-back runs on the 13-member gwz-dev workspace all exited 1 while an interactive run
afterwards exited 0; not diagnosed, consistent with the same timeout.

**Context:** a new SSH connection to GitHub was measured at 2.0-3.5 s per operation (`GwzRemoteTransportRequirements.md`
measurement table and the 2026-09-15 probe). With 8 connections opening at once the slowest handshakes exceed 3 s.

**Expected:** the default timeout covers a normal handshake under the default concurrency. Options: raise the
default (10 s is the pool's `connect_timeout_ms` default in `gwz-transport/src/pool/mod.rs`); apply the timeout to
connect and to first-byte separately; or retry once on the session-start timeout before marking the member failed.
Also worth confirming that the `errors` array is populated on this Partial (BUG 2).

---

## Not a gwz bug, but found on the way
- The gwz-dev root repo's GitHub language bar reads C++ 40k / Python 18k / Rust 13k because the root holds
  `trial/`, `scripts/` and Bazel files and none of the member sources. A reader in the r/git thread concluded
  "written in python". A `.gitattributes` with `trial/** linguist-vendored` and `scripts/** linguist-vendored`
  fixes the bar on the next push.

## Verification for any fix
`cargo fmt --all -- --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`;
gwz-core's gate via `python3.13 run_tests.py`. Re-run the reproduction above for the bug being fixed; for BUG 3,
five consecutive default fetches on a 16-member SSH workspace must all exit 0.
