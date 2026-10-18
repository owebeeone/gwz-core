# HTTPS credential helper failures

A credential helper stores or supplies the password or token Git uses for a
private HTTPS repository. GWZ asks the helpers configured in your global and
system Git configuration, its XDG files and the `GIT_CONFIG_*` settings used
to start it, including unconditional includes. It does not ask for your
password itself.

In the messages below, this member's URL means the remote repository URL.
`git -C <path> remote get-url origin` prints it for a cloned member. Before the
member is cloned, use the manifest's URL. Under `--url-scheme https`, use the
URL that `gwz --verbose materialize --lock` prints after `->`.

## A helper did not answer in time

> No credential helper answered within <n> seconds, a fixed bound, so gwz gave
> up on it. If a helper is waiting for you to sign in or unlock it, sign in once
> with `git ls-remote` and this member's URL (`git -C <path> remote get-url origin`
> prints it; before the member is cloned, use the manifest's URL, or under
> `--url-scheme https` the one `gwz --verbose materialize --lock` prints after
> `->`), then retry.

`<n>` is the allowance this lookup had: normally 120 seconds, or a shorter
allowance supplied by the program running GWZ. Waiting for a helper to become
available does not use up this allowance. Fractions of a second are shown
exactly: for example `1.25` means 1,250 milliseconds, and `0.001` means one
millisecond.

GWZ stops the lookup when this bound expires. A sign-in window opened by
another program may remain open. Finish or close that window, unlock the
credential store if needed, and run `git ls-remote <url>` once. Git may use a
different helper inside a repository; use the configuration guidance below
to check the helper GWZ actually uses before retrying.
Do not put a password or token into the URL or a shell command.

## The helper's answer could not be used

> gwz could not use `git credential fill`'s answer: <cause>. No credential was
> sent. `git config --get-urlmatch credential.helper` with this member's URL
> (`git -C <path> remote get-url origin` prints it; before the member is cloned,
> use the manifest's URL, or under `--url-scheme https` the one
> `gwz --verbose materialize --lock` prints after `->`) can select a different
> helper through repository settings or conditional includes. Check the helper
> chain GWZ uses. See Troubleshooting: HTTPS Credential Failure.

`<cause>` is one of these fixed descriptions; the message never includes the
helper's answer or a credential value:

- `the pipe to git failed (<error kind>)`
- `its answer passed 16 KiB`
- `the credential holds a control character`
- `the username holds a colon`
- `its answer is not UTF-8 text`
- `its answer has no final newline`
- `a field is missing`
- `its answer has a malformed or duplicate username/password field`

An ordinary `git config --get-urlmatch credential.helper <url>` inside a
repository can show a different helper. Follow the configuration guidance
below to identify the helper GWZ uses.
Check a script's output format without printing its password or token: one
`username=` field and one `password=` field, no duplicate fields, UTF-8 text,
a final newline, no control characters in either value, and no colon in the
username. Either value may be empty. One trailing carriage return on each line
is accepted; a control character inside a value is not. The complete answer
must fit within 16 KiB.

A storing helper may need `git ls-remote <url>` to obtain a usable credential.
A sign-in tool may need its own sign-in or refresh, for example `gh auth login`
or `gh auth refresh`. A successful Git command may have used a different
helper: repair the helper GWZ uses, then retry. GWZ does not erase or
replace a credential on your behalf.

## A helper could not start in time

> No credential helper could start in the <m> seconds available to wait for
> resources. Other helpers may be busy, waiting for sign-ins or stuck. Finish
> any open sign-in, or sign in once with `git ls-remote` and this member's URL
> (`git -C <path> remote get-url origin` prints it; before the member is cloned,
> use the manifest's URL, or under `--url-scheme https` the one
> `gwz --verbose materialize --lock` prints after `->`), then retry. See
> Troubleshooting: HTTPS Credential Failure.

`<m>` is the resource allowance available when this lookup began waiting.
The normal command starts with 30 seconds for resources; anonymous discovery
and earlier resource waits may leave less. The program running GWZ may supply
another allowance. The message shows the actual allowance, including exact
fractions of a second; it does not restart a fresh 30-second wait. No helper
was started for this lookup. An earlier wait can leave zero seconds, even
when helper slots are free. This message does not mean every helper slot
was busy.

Finish any sign-in or unlock already waiting, or close an unwanted sign-in
window. Run `git ls-remote <url>` once if your helper needs to sign in, then
check the configuration guidance below and retry the command. A successful
Git command may have used a different helper. Retrying repeatedly while
every helper remains busy does not free a slot.

## HTTPS URLs with a user name

An HTTPS URL may name the account your credential helper should select, for
example `https://account@example.com/team/repo.git`. That user name reaches
only the credential helpers, inside Git's URL input. It is not sent as HTTP
userinfo and is not added to a helper failure message. With no user name in
the URL, Git uses the applicable configured username setting or asks its
helpers to choose one.

The URL remains percent-encoded when passed to Git: for example a user name
written as `a%3Ab` stays encoded there, and Git decodes it for the helper.
A decoded control character in the user name or path is refused before any
request or helper starts. A password in an HTTPS URL, a query, or a fragment
is also refused. Keep passwords and tokens in a credential helper.

## Configuration and retry choices

Git run inside a member may read its local configuration and `includeIf`
settings. GWZ reads no member's local configuration and no conditional include.
Its lookup runs outside the repository, with the environment captured when
GWZ started, and reads the system, global and XDG files and `GIT_CONFIG_*`
settings. Unconditional includes from those files still apply. A successful
`git ls-remote` or the helper shown by an ordinary
`git config --get-urlmatch credential.helper` inside the member can therefore
use a different helper and leave GWZ's helper broken.

Inspect the helper entries for this URL in that system/global/XDG chain,
following its unconditional includes and any `GIT_CONFIG_*` overrides used
to start GWZ. Start with `~/.gitconfig` and `$XDG_CONFIG_HOME/git/config`
(or `~/.config/git/config` when XDG_CONFIG_HOME is unset), and the system
configuration belonging to the Git installation GWZ uses. `GIT_CONFIG_GLOBAL`
and `GIT_CONFIG_SYSTEM` can replace those file paths; `GIT_CONFIG_NOSYSTEM`
can disable the system file. `GIT_CONFIG_PARAMETERS`, or `GIT_CONFIG_COUNT`
with its numbered `GIT_CONFIG_KEY_*`/`GIT_CONFIG_VALUE_*` pairs, can override
entries. Inspect those settings privately; do not print their values, since
they can contain secrets. Follow each unconditional `include.path` to its
file, checking URL-specific credential sections as well as general ones.
Check the entries in order: an empty `credential.helper` resets
the helpers listed before it. Find the script or credential store actually
selected by that chain, and repair or sign into that helper. Keep credential
settings in the global chain outside `includeIf` for GWZ to use them. Do not
disable all includes: that would also remove legitimate unconditional helpers.

For example, your global file may include another file unconditionally that
selects helper A, which needs repair or sign-in. A matching `includeIf` can
reset that choice and select working helper B inside the member; a local
repository setting can do the same. Git can then succeed with B while GWZ
still fails with A. Follow the unconditional include to A's helper entry and
repair or sign into A. If you intend to use B for GWZ too, put its helper entry
in that unconditional global chain, outside `includeIf`, with the intended
ordering/reset. Keep the unconditional include. If only A is configured,
repair or sign into A directly. An XDG file or `GIT_CONFIG_*` override can also
supply or reset the chain; check those sources rather than assuming the main
global file is the only source.

Record prior helper entries and their order before changing them. Restore
those entries to undo the change. Do not erase unrelated helper settings or
paste credentials into configuration or diagnostic output. For a sign-in tool,
use that tool's own account/sign-in controls. For a stored stale credential,
Git's first `git ls-remote <url>` may fail and erase it; the next run can ask
for the new credential and store it. Confirm that Git used the helper GWZ
needs; success through a repository-only helper does not repair another store.

For a one-command retry through the native transport, use `--transport native`.
For a program using the Python binding, set `GWZ_TRANSPORT=native` for that
operation. Omit the command-line flag or remove the environment setting to undo
that choice. Native does not supply a configured-helper time bound; it can
still require the same sign-in or helper repair.
