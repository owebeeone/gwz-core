# Credential helpers in the transport candidate

This guide describes the transport candidate for the planned 1.1.0 release.
It is not a claim that Windows transport parity or the release is qualified.
The CLI and Python client use the same core credential-helper implementation.

## Which helper runs

GWZ uses the `git` executable from the environment captured when the operation
starts and asks it to run the configured credential helpers. `gh` is one possible
helper, not a requirement. Helper matching follows Git's URL rules, including
`credential.useHttpPath`, configured usernames, ordered helper entries and empty
entries that reset the chain. GWZ does not run credential `store` or `erase`.

The configuration view includes the captured system, global and XDG sources,
session `GIT_CONFIG_*` overrides, and their unconditional includes. It excludes
repository-local configuration and every `includeIf`. A helper selected by
native Git inside a repository can therefore differ from GWZ's helper.

For HTTPS, discovery starts anonymously. A supported authentication challenge
allows a lookup; each credential route asks once per operation. A redirected
route starts anonymously and gets its own lookup. Rejected credentials are not
resent or looked up again for that route. The next operation looks up afresh.
This guide does not claim the pending Windows authentication schemes work yet.

For SSH, configured helpers apply only to the ambient password-only route, after
host-key verification and discovery of the server's authentication methods.
Explicit keys, public-key-only or combined offers, and disabled helpers retain
key/agent selection. Helpers are not an alternative to host-key verification.

## A helper could not start or finish

The helper's interaction allowance is at most 120 seconds, shortened by a
smaller configured or caller allowance. It starts after admission; waiting for
the endpoint and shared host helper slots uses a separate allocation allowance.
There are eight slots at each level, including slots retained during cleanup.
These limits are separate from member jobs, connections and network timeouts.
Network aggregate and stall clocks pause during the admitted local SSH helper
phases; finishing a helper does not manufacture network progress.

`credential_helper_timeout` is public error code **75**, not a 75-second limit.
The message says whether no helper could start or a helper failed to answer,
and names the exact allowance in seconds, including fractions. A zero allocation
allowance starts no helper and does not prove that other helpers were busy.
The same helper timeout guidance applies to HTTPS and SSH password helpers;
ordinary network timeouts keep their ordinary classification.

If a helper is waiting for a sign-in or unlock, finish it or sign in using its
own tool. `git ls-remote <member-url>` can also unlock a store, but its success
alone does not prove that GWZ's helper was repaired. For an existing member,
`git -C <member-path> remote get-url origin` identifies its URL. Before cloning,
use the manifest URL; with an HTTPS URL conversion, use the effective URL shown
after `->` by `gwz --verbose materialize --lock`.

Check the actual unconditional helper chain and the captured overrides. If Git
succeeds through helper B from a repository file or conditional include while
GWZ uses helper A, repair/sign in to A or deliberately configure the intended
helper in GWZ's unconditional scope. Retain legitimate unconditional includes.
Do not substitute `--no-includes` as a diagnostic configuration view. Before
editing configuration, privately record the original source, entries, order and
empty resets; undo by removing only the entries introduced and restoring that
original chain. Do not display or save a helper's credential answer.

## Other failures and recovery

- `external_tool_missing`: the captured environment cannot find or execute
  `git`. Repair its `PATH`/installation and start a new operation. Clone reports
  this even for a private member; it is not a quiet authentication skip.
- An unusable answer: the error names a fixed parser/configuration cause and no
  credential is sent. Check the chain GWZ uses. An ordinary
  `git config --get-urlmatch credential.helper <member-url>` can select a
  different helper through repository settings or conditional includes.
- A rejected credential: renew it in the provider/store that owns it, then
  start a new operation. GWZ never erases or replaces it for you.

To compare with the explicitly selected native transport, the candidate CLI
supports `gwz --transport native fetch`; `GWZ_TRANSPORT=native` also selects it
for CLI/Python operations. Unset that variable or use the higher-priority CLI
`--transport gwz` to return. This is an explicit selection, not automatic
fallback. See the CLI authentication guide or Python README for the global-file
setting, precedence and paired removal recipe.
