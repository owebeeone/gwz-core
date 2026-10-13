# Windows: authentication, proxy and SSH settings

Proposed 1.1.0 user documentation, 2026-10-03; revised 2026-10-13. **DRAFT until the
release.** The capabilities described here are built into the Windows build of GWZ's
transport (the qualification switch is retired); this page is not yet a published
guide. Texts in quotation marks or in indented blocks are the messages GWZ prints;
they stay proposals until the release.

GWZ's own transport is the default. On Windows it uses Pageant, the OpenSSH
agent, the machine's WinHTTP proxy (Windows' own HTTP proxy setting) and Windows
authentication. If you need the behavior of gwz 1.0, choose the native transport
(libgit2's, as gwz 1.0 used) before an operation. GWZ reports a failed operation;
it never changes transports while it runs. (A plain `http://` remote, a file or
local-path remote and a `git://` remote always use the native transport, as described
next.)

GWZ's transport carries `https://` and SSH remotes. A file or local-path remote and
a `git://` remote are carried by the native transport, which is how they have always
gone. A plain `http://` remote is carried by the native transport instead, under every
setting (see "One exception" below), and GWZ prints this warning before it sends the
first request:

```
warning: <url> is an http:// remote. GWZ carries it on the native transport: a Basic
password is sent unencrypted and a Windows sign-in has no channel binding. Use an
https:// address if the server offers one.
```

Over `http://` a Basic password travels unencrypted, and a Windows sign-in is
answered without channel binding (a protection that ties the sign-in to the
encrypted connection). Use `https://` where you can. GWZ also refuses a redirect
from `https://` to `http://` (see Redirects).

If an authentication or trust error is unclear, run the command again with
`--verbose`, which prints authentication diagnostics.

## Choose or remove a transport setting

For one command, use `gwz fetch --transport native`. Omit the option or use
`--transport gwz` to use GWZ's transport.

The order, highest first: the `--transport` option, the `GWZ_TRANSPORT`
environment variable, your global git configuration (`gwz.transport`), then the
default, GWZ's transport. The environment variable beats the global configuration.
Accepted values are `gwz` and `native`; unset means use the next source. An invalid
value is refused, not ignored, with a message such as:

```
GWZ_TRANSPORT must be gwz or native, not "bogus"; --transport decides without it
gwz.transport in <file> must be gwz or native, not "true"; remove it with
  git config --file "<file>" --unset-all gwz.transport, or fix it;
  --transport or GWZ_TRANSPORT decides without it
```

On Windows the printed command quotes the path in double quotes, which pastes
correctly in cmd, PowerShell and Git Bash (on macOS and Linux it is single-quoted).
For a value that a file included by `~/.gitconfig` holds, the message prints
`git -C / config --file "<file>" --includes --show-origin --get-all gwz.transport`,
which names the file that holds it. If the path holds a character that one of those
shells would expand (`$`, a backtick, `%`, `!`, a curly double quote), starts with
`\\` (a network path) or is not a drive path, GWZ prints no command and says to
remove the key from that file, or from the file that holds it.

In PowerShell, `$env:GWZ_TRANSPORT = 'native'` selects native for commands
started from that shell, and `Remove-Item Env:GWZ_TRANSPORT` removes it. In Git
Bash, use `export GWZ_TRANSPORT=native`, then `unset GWZ_TRANSPORT`.

For your account, run `git config --global gwz.transport native`. Remove it with
`git config --global --unset-all gwz.transport`; setting `gwz` also selects GWZ.
Workspace and member repository values do not select the transport. GWZ tells you
when it ignores one:

```
gwz: note: ignoring <setting> in <file> (<scope>): only --transport, GWZ_TRANSPORT and your global git configuration select the transport; <how to remove it>
```

**One exception: `http://` remotes.** The order above does not apply to a plain
`http://` remote. It goes by the native transport even under `--transport gwz`, and
the warning above is printed before its first request, on every operation, whether
or not you give `--verbose`. Nothing turns that warning off. For that remote the
native transport's behavior applies, not GWZ's: no connection pooling, no setup
retry (`--max-retries` has no effect on it) and no 30-second setup budget, and
libgit2's connect and read timeout is your `--ssh-timeout` value (9 seconds by
default under GWZ's transport; 3 only when you choose `--transport native`). The
only way to avoid all of this is an `https://` address.

### Setup limits

These are command-line options only; there is no environment variable or git
configuration key for them. They apply to GWZ's transport.

| Option | Default | What it does |
|---|---|---|
| `--ssh-timeout <secs>` | 9 on GWZ's transport (3 with `--transport native`) | The stall limit for one SSH setup attempt, and for a stalled read of a fetch, push or pull body on SSH **and on HTTPS**, whatever the option's name suggests. `0` removes the stall limit **and** the 30-second setup budget below, on SSH and on HTTPS. When a stall ends an attempt, the error names `--ssh-timeout` and its value |
| (no option) | 30 s | The setup budget. HTTPS setup has no stall limit, only this budget; SSH setup has both. Only `--ssh-timeout 0` clears it |
| `--max-retries <n>` | 3 | Extra setup attempts after the first failure (four attempts). Waits of 1, 2 then 4 seconds, doubling and capped at 30 seconds, plus up to 0.25 second. `0` retries nothing. `--ssh-timeout 0` does not turn retries off |

With `--transport native` the defaults are 50 jobs, 8 connections per host and a
3-second connect and read timeout (`--ssh-timeout` is libgit2's connect and read
timeout there), as in gwz 1.0. Native has no pooling, no setup retry and no
30-second setup budget, so `--max-retries` has no effect. The Python package `gwz`
(`pip install gwz`) uses the timeout configured for its process, 9 seconds by
default, on either transport; see that package's documentation to change it.

## SSH agent and home

GWZ picks **one** agent source when the operation starts and does not switch if it
fails:

1. **Pageant**, if a Pageant window is running in your Windows logon session. It
   wins even when `SSH_AUTH_SOCK` is set, and even when the OpenSSH agent is also
   running (gwz 1.0.17 does the same). When `SSH_AUTH_SOCK` is set, the message when
   Pageant is selected names Pageant and says `SSH_AUTH_SOCK` was not consulted. To use the OpenSSH agent
   instead, close Pageant.
2. Otherwise, the local named pipe `SSH_AUTH_SOCK` names.
3. Otherwise, the default pipe `\\.\pipe\openssh-ssh-agent`.

`SSH_AUTH_SOCK` must be a pipe on this computer in the form `\\.\pipe\<name>`.
A pipe on another machine (`\\server\pipe\...`, including `\\localhost\pipe\...`)
and Git Bash's Unix-style socket paths are refused, and the message names
`SSH_AUTH_SOCK`.

**The OpenSSH Authentication Agent service must be running** to use the OpenSSH
agent, and on many Windows machines it is disabled. Starting it is up to you or
your administrator. In PowerShell:

```
Get-Service ssh-agent
# as administrator (Automatic starts it at every restart; with Manual you must
# run Start-Service again after each restart):
Set-Service ssh-agent -StartupType Automatic
Start-Service ssh-agent
# then, as yourself, with the name of your own key file:
ssh-add "$HOME\.ssh\<your-key-file>"
```

When the service is stopped, GWZ says:

```
The OpenSSH Authentication Agent service (ssh-agent) is not running. As administrator
run: Set-Service ssh-agent -StartupType Automatic; Start-Service ssh-agent. Then add a
key with ssh-add. To use Pageant instead, load the key into Pageant.
```

For Pageant, load the key into Pageant ("Add Key") instead.

If the agent has no key the server accepts, GWZ reports that and does not try
another agent. If you pinned a key with `--identity`, `--remote-identity` or
`gwz auth identity`, GWZ offers only that key, and the message names the pin: remove
it first (`gwz auth identity <remote> --unset`, or drop `--identity`), or load that
key into the agent.

While Pageant waits for your confirmation, the connection's stall and setup-budget
clocks **keep running**, as for any other wait on the network. The wait ends the
attempt at the stall limit, `--ssh-timeout` (9 seconds by default on GWZ's
transport), or at the 30-second setup budget, whichever comes first; `--ssh-timeout
0` removes both. Even then, each Pageant request has a finite upper limit of its
own (never zero), so one request cannot wait forever; Ctrl-C returns sooner. The
fixed limit that applies to a git helper's sign-in window (120 seconds) does not
apply to Pageant.

**If your Pageant is set to confirm every use and its prompt stays open longer than
that, the attempt fails**, with:

```
Pageant did not answer within <n> seconds (the SSH stall limit, set by --ssh-timeout).
Pageant may still be waiting for you to confirm the key or to enter its passphrase.
Answer it and run the command again, raise --ssh-timeout (up to 30 seconds; 0 removes
the limit), or use --transport native, which waits for Pageant as gwz 1.0 did.
```

Raising `--ssh-timeout` helps only up to the 30-second setup budget. If the prompt
outlasts that budget, the attempt ends with:

```
Pageant did not answer within 30 seconds, the longest GWZ spends setting up one
connection, and raising --ssh-timeout does not lengthen that. Pageant may still be
waiting for you to confirm the key or to enter its passphrase. Answer it sooner and
run the command again, use --ssh-timeout 0 (which removes the limit), or use
--transport native, which waits for Pageant as gwz 1.0 did.
```

GWZ retries a failed setup attempt, and each retry asks Pageant again, so with
confirm-every-use a retry can fail the same way. If Pageant asks for a passphrase
the first time it uses a key, enter it: the key then stays loaded and the next
attempt succeeds. An ssh-agent that confirms every use (`ssh-add -c`) behaves the
same way on macOS and Linux today. gwz 1.0.17 waits for the prompt (see the
changes table).

**If Pageant refuses the request.** A Pageant that runs at a higher privilege level
than the command (for example as administrator), or as another account, may not
serve the request at all; Windows blocks the message between privilege levels. GWZ
then reports a refusal, not a missing Pageant, and asks no other agent:

```
Pageant refused the request. Pageant is the SSH agent GWZ selected, because a Pageant
window is running. It may be running as another user, or at a higher privilege level
than this command (for example as administrator), so it does not serve this account.
GWZ does not ask another SSH agent in its place. Run Pageant as the same user and at
the same privilege level as this command, or close Pageant to use the OpenSSH agent.
```

**What GWZ does not check about the agent.** GWZ does not refuse a Pageant that
belongs to another user on the same desktop, and it does not check who created
the agent pipe. It does check that the Pageant window is still the same program it
first found, and it creates the memory it shares with Pageant so that only your
account and the system can open it; code running as you at a lower privilege level
is not separated from you. Because GWZ does not check who owns the pipe, another
account on the same computer can create `\\.\pipe\openssh-ssh-agent` while the
agent service is not running. GWZ would then connect to it, and that account could
learn who you are, which keys it offers and what GWZ asks it to sign. It cannot
sign for you, because your keys are not in it.

### Home

The SSH home is selected from `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then
`USERPROFILE`; the first one that exists is used. It supplies both
`.ssh\known_hosts` and `~/` key paths, so **a key path that starts with `~/`
now uses the same home as `known_hosts`** (gwz 1.0.17 expanded `~/` from
`USERPROFILE`, which differs when `HOME` and `USERPROFILE` differ). A missing or
unusable home reports which home source needs correcting, and an empty `HOME` is
an error rather than being skipped. Folder and key names with non-ASCII
characters work; gwz 1.0.17 fails on them.

Host-key checking is required before any login, and choosing native does not
repair a mismatched host key. GWZ tells two refusals apart and words each its own
way. Both print the file GWZ read and which home it came from.

**The host is not listed** (the file has no entry for it, or does not exist):

```
gwz cannot trust the host key of <host>: <path> does not list it, or does not exist
or cannot be read. gwz read <path>; its home came from <HOME, HOMEDRIVE and HOMEPATH,
or USERPROFILE>. If <host> is new to you, first check its key fingerprint with its
administrator or provider, then add the key. <the commands below>  Then compare the
fingerprint: ssh-keygen -lf "<path>". Then retry.
```

To add an RSA entry (see the next section for why RSA), replace `<path>` with the
printed path and `<host>` with the server's name:

```
# PowerShell
ssh-keyscan -t rsa <host> | Add-Content -Encoding ascii "<path>"
# Git Bash
ssh-keyscan -t rsa <host> >> "<path>"
```

Then compare the fingerprint, `ssh-keygen -lf "<path>"`, with the one the provider
publishes before you rely on it: a key fetched over the network proves nothing by
itself.

**The key changed** (the file lists the host with a different key):

```
gwz refuses to connect to <host>: the host key it presented differs from the one
recorded in <path> at line <n>. gwz read <path>; its home came from <variable>. This
may be an attack, with someone between you and the server, or the server may have
been re-keyed. Verify the server's key fingerprint out of band, with its
administrator or provider, before you trust it. ssh-keygen -lf "<path>" shows the
fingerprints on record. Remove the old line only after you have verified the new
key, then retry.
```

The line number is the first line of the file that lists the host with another key,
and is left out when GWZ cannot find one line that does. This message prints no
command that adds a key or removes an entry: with a changed key, the fingerprint
check comes first and the edit is yours. When GWZ cannot tell the two refusals apart
(the endpoint is in another process), it uses one message that names both causes and
shows the add and remove commands after the fingerprint check.

## SSH host keys and key files

On Windows GWZ has the same limits as gwz 1.0.17. Only **RSA host keys** are used:
a `known_hosts` entry that is `ecdsa` or `ed25519`, with no RSA entry for the
host, fails before connecting. The message names the cause and tells you to add an
RSA entry with `ssh-keyscan -t rsa` (above).

A key file is offered only when you name it (see "Use the converted copy" below);
otherwise GWZ uses the agent (or a password, for a server that accepts nothing
else). Of the key files you can name, only an **unencrypted RSA key file in PEM
format** is offered. A new-format RSA key (what `ssh-keygen -t rsa` writes today),
an ECDSA or ed25519 key file, and a passphrase-protected key file are not used when
you name one, and the message says so. Your options:

- **Load the key into an agent** (best; it works for RSA, ECDSA and ed25519 keys,
  protected or not).
- For an RSA key, convert **a copy** to PEM. `ssh-keygen -p -m PEM` rewrites the
  file it is given in place, so never run it on your only copy:

  ```
  # PowerShell
  Copy-Item "$HOME\.ssh\id_rsa" "$HOME\.ssh\id_rsa_pem"
  ssh-keygen -p -m PEM -f "$HOME\.ssh\id_rsa_pem"
  # Git Bash
  cp ~/.ssh/id_rsa ~/.ssh/id_rsa_pem && ssh-keygen -p -m PEM -f ~/.ssh/id_rsa_pem
  ```

  Press Enter for the new passphrase. That **removes the passphrase**, so the copy
  is unprotected: store it with care and delete it when you no longer need it.
  Your original is unchanged. To turn a file back into the new format, run
  `ssh-keygen -p -o -f <file>` (tested with OpenSSH 10.3 on macOS, not with
  Windows' OpenSSH).

**Use the converted copy.** Making the copy does not make GWZ use it. When no key is
named, GWZ tries no key file at all: it offers the keys of the agent (Pageant or the
OpenSSH agent) only, and it does not look for `id_rsa`, `id_ed25519` or any other
file in your `.ssh` folder. Name the copy for one command, or save it for a remote:

```
gwz fetch --identity C:\Users\you\.ssh\id_rsa_pem
gwz auth identity origin --set C:\Users\you\.ssh\id_rsa_pem
```

`--identity` applies to that command only. `gwz auth identity` saves the path in the
local git configuration of the selected repositories (the active members; add
`--member @root` for the workspace root only, or `--all`), and
`gwz auth identity origin` with no option shows what is saved. **To undo it:**
`gwz auth identity origin --unset`. While a key is named, GWZ offers only that key and
does not turn to the agent, so unset it again when you go back to an agent. Give the
full path rather than `~/`: PowerShell's `$HOME` is your user profile folder, while GWZ's
SSH home is chosen from `HOME`, `HOMEDRIVE` plus `HOMEPATH`, then `USERPROFILE` (see
Home above) and can be a different folder; `Resolve-Path "$HOME\.ssh\id_rsa_pem"`
prints the full path.

Keys held by Pageant or the OpenSSH agent can be RSA, ECDSA or ed25519. RSA keys
from an agent are signed with `rsa-sha2-512` only, so a server that accepts only
`ssh-rsa` or `rsa-sha2-256` is refused, with a message that names the signature
type the server lacks; use an ECDSA or ed25519 key, or have the server accept
`rsa-sha2-512`. An RSA **certificate** held by an agent does not authenticate; the
message says so. Not tested: a hardware security key, and certificates held by
Pageant or by the OpenSSH agent itself (an ed25519 certificate served by a test
agent on a pipe did authenticate).
A connection that drops before login is retried; gwz 1.0.17 made one attempt.

## Proxy

**What is supported:** one static proxy set in the machine's WinHTTP settings, and
a proxy that does not ask for a login. Nothing else controls GWZ's proxy use.
Check the setting with `netsh winhttp show proxy`. If it says "Direct access", GWZ
connects directly, and when a direct connection fails before the server answers the
error adds: "The Windows machine proxy setting (netsh winhttp show proxy) says direct
access, so GWZ connected to the server directly, and that connection failed." and
then how an administrator sets a proxy. It says nothing of the kind when a proxy is
named, or when the server answered. Your administrator can set it with `netsh winhttp set proxy
proxy-server="host:port" bypass-list="..."` or, to copy your Internet Options
settings, `netsh winhttp import proxy source=ie`. A proxy that exists only as a PAC
script (automatic configuration) is not supported; restate it as a static proxy.
Whether the native transport follows a PAC script was not tested here, so do not
assume it does.

`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` (and their lowercase
forms) and git's `http.proxy` setting are ignored, whether or not a machine proxy
is set. This is the reverse of macOS and Linux. `NO_PROXY` does not stop a machine
proxy, and an empty `http.proxy` does not turn it off.

GWZ accepts these machine proxy forms: `host:port`, `host` alone (port 80, WinHTTP's
default for a bare entry; accepted by the operator's decision of 2026-10-12 and not
measured on gwz 1.0.17), `http://host:port`, or a list such as
`http=host:port;https=host:port`. An `https://` repository goes through the proxy
by asking it to open a tunnel to the server (a CONNECT request). **That was measured
with gwz 1.0.17 against one public HTTPS server, and GWZ's own transport was then
run through the same kind of tunnel on a Windows test host (sixteen rows, plan step
4.7), with the results below.** In a list, the entry for the connection's scheme is used (a bare entry
beside a list applies to https, and an explicit `https=` entry beats it). **If the
setting is a list with no entry for `https` (for example `http=proxy:8080` or
`socks=proxy:1080` only), GWZ refuses**, with:

```
The Windows machine proxy setting has no entry for https: "<the setting as netsh winhttp
show proxy prints it>". GWZ does not send https traffic around a proxy that was set up
for other protocols only. Ask your administrator to add an https entry, for example
netsh winhttp set proxy proxy-server="http=proxy:8080;https=proxy:8080". Only if a
direct connection is acceptable on your network, --transport native connects directly
as gwz 1.0 did.
```

The administrator's fix comes first. gwz 1.0.17 connects directly in that case and
ignores the proxy, and `--transport native` does the same for one command, without
changing any setting on the computer; use it only where your network allows direct
connections and your administrator accepts that. If your network blocks direct
connections, `--transport native` fails too, and only the administrator's fix helps.
A proxy host that does not resolve fails
the operation; GWZ does not then go direct.

A setting that GWZ refuses (the cases above) refuses **only the https remotes**:
an SSH remote or a local operation runs as if no proxy were set, because it never
uses one, and an https remote fails before any connection with the same text. GWZ also refuses, naming the setting,
any form it cannot interpret: more than one bare entry, a bad port, a user name, a
path, PAC or automatic settings, or a proxy address that is not `http://`. It
never goes direct because it could not read a setting. Do not reset an
administrator's proxy merely to bypass a GWZ error: `--transport native` is a choice
for one command that leaves the proxy setting alone, not a way to change it.

These go direct even with no bypass list: `localhost` in any case, any `127.x.x.x`
address, `[::1]`, and this computer's own address when it is written as an IP
address (GWZ lists the computer's addresses once, when the operation starts, and
never looks a name up to decide this). `localhost.` with a trailing dot does not,
and the `<-loopback>` token has no effect. A bypass list may be separated by
semicolons, spaces or commas. An entry may be a name, a name with `*` wildcards
(which match across dots), `<local>` (names without a dot), `host:port` (only
that port) or a name with a leading `http://` or `https://` (for an `https://`
repository, `https://host` matches and `http://host` does not). Names compare
without regard to case, and a trailing dot must match exactly.

Some cases were not measured even with gwz 1.0.17: an HTTPS server written as an IP
address, a redirect from one HTTPS server to another through a proxy, this
computer's own address as an HTTPS server, and a proxy entry with no port. For them
GWZ follows one rule: it applies the rules on this page, so the request goes to the
proxy, goes direct only where a bypass rule above matches (this computer's own
address included), or GWZ refuses and names the setting. It does not go direct
because it could not decide.

If a proxy answers 407 and asks for credentials, GWZ refuses and says that proxy
sign-in is not supported, naming the machine proxy setting. The message does not
point to the native transport, because the native transport refuses a 407 the same
way (measured with gwz 1.0.17). GWZ sends no credentials to the proxy: not your
Windows identity, not a helper's, and an origin credential is never sent to the
proxy.

## Web (HTTPS) authentication

GWZ starts discovery without credentials. What answers a login challenge depends
on what the server offers:

- If the server offers **Negotiate or NTLM** (Windows sign-in), alone or beside
  Basic or Digest, GWZ uses **your Windows account** and does not ask your
  configured git helper.
- If **Basic** is the only scheme the server offers that GWZ can use, your
  configured git helper is asked once and what it returns is sent. A GitHub token
  held by a helper is sent this way.
- GWZ **never answers Digest**. A server that offers only Digest is refused at
  once, with a message that says Digest is not supported by either transport and
  suggests using SSH or having the server offer another scheme; the native
  transport does not help. If the server offers Digest and another scheme, Digest
  is ignored.
- If the server **rejects your Windows account**, the operation stops with an error
  naming the scheme:

  ```
  <host> rejected your Windows account (<NTLM or Negotiate>). GWZ tries nothing else.
  Ask the server's administrator for access for this account, or use an SSH remote.
  ```

  GWZ does not try it again and does not turn to a helper
  credential, and no setting changes this in 1.1.0. (Selecting the native
  transport behaves differently only in that, with a server that offers just NTLM
  or Negotiate with Basic, it repeats the attempt and asks your helper each time;
  no helper sign-in was ever seen to succeed that way, so this guide does not
  promise it helps.)
- If the server offers only Basic and git has no credential helper for the remote,
  GWZ says so (this text is only for that case; the two below are not it):

  ```
  The server asks for a user name and password (Basic) and no git credential helper
  is configured. Configure one (git config --global credential.helper manager;
  manager is Git Credential Manager, which comes with Git for Windows) and retry.
  ```

- If a helper is configured for the remote and still gave no password (you cancelled
  it, it holds nothing for this server, or it failed), GWZ names the host and sends
  nothing. *Text for the Surface look, not yet seen by Surface:*

  ```
  The credential helper in your git configuration gave no password for <host>: it may
  have been cancelled, may hold no sign-in for this server, or may have failed. No
  credential was sent. Check the helper's sign-in: `git ls-remote` with this member's
  URL runs the helper where it can ask you or show its own error. Then retry.
  ```

- If GWZ refused to read your git configuration (it is too large, its includes are too
  deep or too many or cannot be followed, a value is too large for the environment of
  the git it starts, or `git config` itself fails), it asks no helper and says so.
  *Text for the Surface look, not yet seen by Surface:*

  ```
  gwz did not use your git configuration to find a credential helper, because it refused
  to read it. It asked no helper and sent no credential. It refuses a configuration that
  is too large, include.path chains that are too deep or too many, an include it cannot
  follow (unreadable, not a regular file, or written as `~user/` or `%(prefix)/`), a
  value too large for the environment of the git it starts, and a `git config` that
  itself fails. `git config --list --show-origin` shows what git reads: reduce it or fix
  the include it names, then retry.
  ```

- If the server offers only a scheme GWZ does not support at all, the message names
  the offered scheme as the server sent it and says that gwz 1.0 did not answer it
  either. It points to no transport option, because the native transport has nothing
  to answer it with; use git itself for that remote, or ask the server's
  administrator to accept Basic with a token. On macOS and Linux the Windows logon
  identity is unavailable, so a Negotiate-only server is refused there as well.
- If a helper times out or is cancelled, the operation ends. If the server
  challenges the upload step of a fetch (after the first request succeeded), GWZ
  refuses with a message and does not send the data again.

Your configured helper can show its sign-in window. Git does not prompt on the
terminal. At most eight helper lookups run at once. A lookup waits for a free
place for at most the setup allocation budget, normally 30 seconds (less if the
first request has already used part of it), and one that queues too long fails
before a window opens, with the message below, where `<m>` is the number of seconds
that were available and the URL is the member's own:

```
No credential helper could start in the <m> seconds available to wait for resources.
Other helpers may be busy, waiting for sign-ins or stuck. Finish any open sign-in,
or sign in once with `git ls-remote <the member's URL, printed here>`, then retry.
```

Warm up the credential before a large operation: run `git ls-remote <url>` for one
repository, or fetch one repository first, and sign in; the others reuse it. Whether
`--max-per-host 1` avoids the queue was not measured. Once a helper has a place, its
sign-in window may stay unanswered for at most 120 seconds (less if the command's own
limit is shorter); then the operation ends, and the error names the number of seconds.
`--ssh-timeout` does not shorten either wait. A missing git is reported with the
native transport setting as a possible next action.

**Warning: Windows logon credentials can be offered to any host that challenges
with Negotiate or NTLM, including an Internet host, over `https://` (and over
`http://` through the native transport).** What the host receives is a NetNTLMv2
response: an attacker who gets it can try to crack it offline, and can relay it to
services that do not require Extended Protection or signing. Administrators can
restrict outgoing NTLM by Windows policy. This matches gwz 1.0.17's offer and is
an explicit product decision. **No setting turns off offering your Windows
credentials in 1.1.0, and the native transport offers them too.** Review the remote
URL and any redirect before using it. GWZ validates the HTTPS certificate and ends
the operation if a redirect arrives during a Windows authentication exchange; it
does not forward that exchange to a new location.

**Which certificates are trusted.** GWZ trusts the Windows certificate store (the
current user's and the machine's) **and also the certificates in one CA file**: the
file named by `GIT_SSL_CAINFO` if that variable is set and not empty, otherwise the
file named by `SSL_CERT_FILE`. Only that one file is used. gwz 1.0.17 on Windows
ignores both variables. A CA file named in your environment is trusted, so a
corporate interception CA placed there will be trusted too: check what those
variables point to.

**A stale variable can now fail.** Because gwz 1.0.17 ignored those variables, a
leftover value that never mattered there can stop GWZ. Before it connects, GWZ
refuses the one CA file it uses if that file is empty, has no certificate, is
garbled or malformed, or is in the `TRUSTED CERTIFICATE` format (measured: the run
`2026-10-08-ca-trust-parity` refused all four on Windows, each variable set on its
own, where gwz 1.0.17 cloned anyway). A file that is missing or cannot be read is
refused with `cannot read endpoint CA file`, and one over 1 MiB with `endpoint CA
file exceeds 1 MiB`; those two cases are from the code and were not measured on
Windows. If `GIT_SSL_CAINFO` is set and not empty, a bad file named by
`SSL_CERT_FILE` is not read and does not stop GWZ; it matters only when
`GIT_SSL_CAINFO` is unset or empty. To clear the problem, correct or remove the
variable that is in use (PowerShell: `Remove-Item Env:GIT_SSL_CAINFO`,
`Remove-Item Env:SSL_CERT_FILE`).

### Redirects

GWZ follows a redirect on its first request, whether the new location is a full
address or only a path, and whether it is another host or the same one: at most
five redirects (301, 302, 303, 307 or 308), never from `https://` to `http://`, and
none after a Windows authentication exchange has started. With `--verbose`, GWZ
prints each redirect it follows. A different **origin** (a different scheme, host
or port, so a changed host name counts even if both names reach the same server)
gets none of the first origin's credentials; a different origin that asks for
credentials is answered as if you had gone there directly. **That includes your
Windows sign-in: if the new address challenges with NTLM or Negotiate, it receives
your Windows account's response**, exactly as the first address would (see the
warning above). gwz 1.0.17 on Windows follows a full-address redirect over HTTPS and
never a path-only one.

A redirect from `https://` to `http://` is refused, with:

```
<url> redirected to "<new url>", which is not https. GWZ does not follow a redirect from
https to http, because the request would then travel unencrypted. The address in quotes
was sent by the server: treat it as untrusted text, and do not use it. Ask the server's
administrator to fix the redirect.
```

The address in the message is only shown to you; it is the server's choice, not
something to copy into a remote.

Kerberos is not tested; the Windows sign-in tests used NTLM. A server that
requires channel binding (Extended Protection) was tested only against this
computer.

## What changed from gwz 1.0.17 on Windows

| # | Change | gwz 1.0.17 | The new transport |
|---|---|---|---|
| 1 | Non-ASCII folder and key names | fails | works |
| 2 | Digest | Digest only: with a git helper configured, 15 retries then an error; with none, fails at once. Digest beside Basic fails the same way | Digest only fails at once; Digest beside another scheme is ignored |
| 3 | Server rejects your Windows account | retried up to 15 times; may ask a helper each time | one error naming the scheme; no retry; no helper fallback; no setting changes this |
| 4 | Redirects | full-address redirects followed over HTTPS only; path-only never; none over `http://` | path-only redirects are followed too (within five); credentials never cross to another origin |
| 5 | Server challenges the upload step | fails (empty resend, or "request must be resent") | refused with a plain message; data never resent |
| 6 | `~/` in `--identity` | uses `USERPROFILE` | uses the same home as `known_hosts` |
| 7 | SSH drop before login | one attempt | retried |
| 8 | Error messages | opaque or shared text | text that names the cause and the fix |
| 9 | Machine proxy with no entry for `https` | connects directly | refused, naming the machine proxy setting |
| 10 | Plain `http://` remote | carried like any remote | carried by the native transport, with a warning printed before the first request that nothing turns off |
| 11 | Trusted certificates | the Windows store only | the Windows store plus one CA file: the one `GIT_SSL_CAINFO` names if it is not empty, otherwise the one `SSL_CERT_FILE` names |
| 12 | Machine proxy with more than one bare entry | uses the first | refused as ambiguous, naming the machine proxy setting |
| 13 | A proxy answering 407 | opaque error | one plain message: proxy sign-in is not supported |
| 14 | A Pageant prompt that stays open longer than `--ssh-timeout` | waits for the prompt | the attempt fails with a message naming `--ssh-timeout` (`--transport native` waits as 1.0.17 did) |

**Unchanged on purpose:** environment and git proxies are ignored on Windows; your
Windows account can be offered to any host that asks; only RSA host keys and
unencrypted RSA PEM key files; RSA agent signatures at `rsa-sha2-512` only; a
Pageant owned by another user is not refused; the agent pipe's creator is not
checked.

**Not promised:** Kerberos; a hardware security key; certificates held in Pageant
or in the OpenSSH agent; Pageant and the OpenSSH agent together; SSH host keys
other than RSA; key files other than unencrypted RSA PEM; proxy sign-in; PAC
scripts (and whether the native transport follows them); GWZ's own transport through
a proxy beyond the forms the step 4.7 rows ran; through a proxy, an HTTPS server written as an IP address, a redirect from one HTTPS
server to another, this computer's own address as an HTTPS server, and a proxy entry
with no port (the request goes to the proxy, goes direct only where a bypass rule
above matches (this computer's own address included), or GWZ refuses; it does not go
direct on a guess); a server that requires channel binding on a remote machine; the OpenSSH
agent beyond RSA, ECDSA and ed25519 keys through its default pipe; filenames on
macOS and Linux (not audited).
