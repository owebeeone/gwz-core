# Windows authentication and proxy settings

2026-10-03 MAIN import: original experimental records and provisional clauses
remain historical/DRAFT. Full Windows NO-GO persists. The separately accepted
[SSPI-only design](../../dev-docs/GwzSspiDesign.md) has its own mechanism review;
it does not accept these whole-platform dispositions, guide or proof rows.
Read SSPI lifetime claims through Windows parity's explicit SSPI-only amendment.
Current MAIN accepted helper timing/configuration/SSH clock govern their domains.


Proposed 1.1.0 user documentation, 2026-10-03; revised 2026-10-11 for the
SSH key limits, the proxy rules, redirects and the 407 refusal. Text marked
OPEN ITEM names a review-package question that is not settled. **DRAFT: these capabilities
are not enabled in the current candidate build.** The final wording awaits
the native Windows baseline and design review.

GWZ's transport is the default. It uses Windows Pageant, the local OpenSSH
agent, the machine's WinHTTP proxy and Windows authentication. You choose the
native transport before an operation if you need the behavior of gwz 1.0.
GWZ reports a failed operation; it never changes transports while it runs.

## Choose or remove a transport setting

For one command, use `gwz fetch --transport native`. Omit the option or use
`--transport gwz` to use GWZ's transport. The command option wins over the
environment and global git configuration.

In PowerShell, `$env:GWZ_TRANSPORT = 'native'` selects native for commands
started from that shell. `Remove-Item Env:GWZ_TRANSPORT` removes it. In mingw
Bash, use `export GWZ_TRANSPORT=native`, then `unset GWZ_TRANSPORT` to remove
it. Accepted values are `gwz` and `native`; unset means use global configuration
or the default.

For your account, run `git config --global gwz.transport native`. Remove it
with `git config --global --unset-all gwz.transport`; setting `gwz` also selects
GWZ. Workspace and member repository values do not select the transport.
GWZ names ignored settings and the selected native transport as TR1.5 specifies.

Native defaults in the CLI are 50 jobs, 8 connections per host and a 3-second
connect/read timeout. It has no pooling, setup retry or 30-second setup budget;
`--max-retries` has no effect. Python uses its process's configured timeout,
9 seconds by default, on either transport. Selecting native does not prevent
Windows from offering your logon credentials.

## SSH agent and home

A Pageant window in your Windows logon session wins over the OpenSSH agent.
GWZ selects that source once when the operation starts. If it disappears,
rejects a request or cannot provide a usable key, GWZ reports the failure.
It does not try another agent. With no Pageant, SSH_AUTH_SOCK selects a local
named pipe, or GWZ uses `\\.\pipe\openssh-ssh-agent` when it is unset.
Remote machine pipes and unsupported mingw Unix socket paths are refused, and
the message names `SSH_AUTH_SOCK`. The OpenSSH Authentication Agent service must
be running to be used; it may be disabled on your machine (it was on the test
machine), and starting it is up to you or your administrator.

When Pageant has no usable key, load a key accepted by the server into Pageant,
then retry. When a request expires during confirmation, confirm promptly on
the next operation or adjust the existing setup limits. OPEN ITEM (review
package R10): how a Pageant confirmation wait counts against the setup time
limits is not settled, so this guide does not yet name the bound.

SSH home is selected from HOME, then HOMEDRIVE plus HOMEPATH, then USERPROFILE.
The selected home supplies `.ssh/known_hosts` and `~/` identity paths; gwz 1.0.17
read `~/` identity paths from `USERPROFILE` instead, which differs when `HOME`
and `USERPROFILE` differ. A missing or unusable home reports which home source
needs correction, and an empty `HOME` is an error rather than being skipped.
Folder and key names with non-ASCII characters work; gwz 1.0.17 fails on them.
Host-key checking is still required; choosing native does not repair a
mismatched host key.

## SSH host keys and key files

On Windows GWZ has the same limits as gwz 1.0.17. Only RSA host keys are used:
a `known_hosts` entry that is `ecdsa` or `ed25519` with no RSA entry for the
host fails before connecting. The message names the cause and tells you to add
an RSA entry with `ssh-keyscan -t rsa`. Only an unencrypted RSA key file in PEM
format is offered. A new-format RSA key, an ECDSA or ed25519 key file, or a
passphrase-protected key file is not used; the message says so. For an RSA key
it tells you to convert it with `ssh-keygen -p -m PEM`; for other key types it
tells you to load the key into an agent. Keys held by Pageant or the OpenSSH
agent can be RSA, ECDSA or ed25519. RSA keys from an agent are signed with
`rsa-sha2-512` only, so a server that accepts only `ssh-rsa` or `rsa-sha2-256`
is refused. A connection that drops before login is retried; gwz 1.0.17 made
one attempt. Not tested: a hardware security key, and certificates held by
Pageant or by the OpenSSH agent. An RSA certificate from an agent does not
authenticate.

## Proxy and authentication

On Windows, the machine WinHTTP proxy controls GWZ and nothing else does.
`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` (and their lowercase
forms) and git's `http.proxy` setting are ignored, whether or not a machine proxy
is set. This is the reverse of macOS and Linux. `NO_PROXY` does not stop a machine
proxy, and an empty `http.proxy` does not turn it off. Browser proxy and PAC
settings are separate. Your administrator can inspect the machine configuration
with `netsh winhttp show proxy`.

GWZ uses these machine proxy forms: `host:port` (or `host` alone, which uses the
default port), optionally `http://host:port`, or a list such as
`http=host:port;https=host:port`. In a list, the entry for the connection's
scheme is used and a list without one means no proxy. A proxy host that does not
resolve fails the operation; GWZ does not then go direct. It refuses, naming the
machine proxy setting, any form it cannot interpret: more than one bare entry,
a bad port, a user name, a path, PAC or automatic settings, or a proxy address
that is not `http://`. It never goes direct because it could not read a setting.
Do not reset an administrator's proxy merely to bypass a GWZ error.

These go direct even with no bypass list: `localhost` in any case, any
`127.x.x.x` address, `[::1]` and this computer's own address. `localhost.` with a
trailing dot does not, and the `<-loopback>` token has no effect. A bypass list
may be separated by semicolons, spaces or commas. An entry may be a name, a
name with `*` wildcards (which match across dots), `<local>` (names without a
dot), `host:port` (only that port) or a name with a leading `http://`. Names
compare without regard to case, and a trailing dot must match exactly.

OPEN ITEM (review package R13): these proxy rules were measured with plain-HTTP
addresses. Reaching an HTTPS repository through a machine proxy, and the
`https=` list entry, have not been tested end to end, so treat them as
unverified until the proxy product tests run. OPEN ITEM (review package R14):
which addresses count as this computer's own is measured for one address only.

If a proxy answers 407 and asks for credentials, GWZ refuses and says so,
naming the machine proxy setting and the native transport option. It sends no
credentials to the proxy: not your Windows identity, not a helper's, and an
origin credential is never sent to the proxy. Proxy sign-in is not supported.

GWZ starts HTTPS discovery without credentials. If the server offers
Negotiate or NTLM, alone or beside Basic or Digest, GWZ uses your Windows logon
identity and does not ask your configured git helper. If Basic is the only
scheme the server offers that GWZ can use, your configured git helper is asked;
a GitHub token held by a helper is sent this way. GWZ never answers Digest: a
server that offers only Digest is refused at once, and Digest is ignored when
the server offers another scheme. If the server rejects your logon identity, the
operation stops with an error naming the scheme; GWZ does not try it again or
turn to a helper credential. A helper timeout or cancellation ends the
operation. A server that challenges the upload step of a fetch is refused with a
message, and GWZ does not send the data again.

Your configured helper can show its sign-in window. Git does not prompt on
the terminal, and the helper interaction has a 120-second bound. At most eight
helper lookups run at once; a queued lookup can exhaust its allocation budget
before a window opens. Sign in with the helper once, then retry. A missing git
error names git and the native transport setting as possible next actions.

**Windows logon credentials can be offered to any HTTPS host that challenges
with Negotiate or NTLM, including an Internet host.** NTLM exposes a response
that a hostile server can try to crack or relay. This matches gwz 1.0.17's
offer and is an explicit product decision. The native transport has the same
hazard. Review the remote URL and any discovery redirect before using it.
GWZ validates the HTTPS certificate and stops a redirect during a Windows
authentication exchange; it does not forward that exchange to a new location.

GWZ follows a redirect on its first request, whether the new location is a full
address or only a path, and whether it is another host or the same one. It never
sends the first address's credentials to a different address; a different
address that asks for credentials is answered as if you had gone there
directly. gwz 1.0.17 on Windows follows only a full-address redirect, and only
over HTTPS.

Kerberos is not tested; the Windows sign-in tests used NTLM. A server that
requires channel binding (Extended Protection) was tested only against this
computer.

On macOS and Linux, this Windows logon identity is unavailable. Unsupported
schemes name the offered scheme and the native transport option. The final
migration notes will give the measured gwz 1.0.17 outcome on each platform.
