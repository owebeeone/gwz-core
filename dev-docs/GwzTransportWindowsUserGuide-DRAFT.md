# Windows authentication and proxy settings

Proposed 1.1.0 user documentation, 2026-10-03. **DRAFT: these capabilities
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
Remote machine pipes and unsupported mingw Unix socket paths are refused.

When Pageant has no usable key, load a key accepted by the server into Pageant,
then retry. When a request expires during confirmation, confirm promptly on
the next operation or adjust the existing setup limits. The final guide will
name the physically verified confirmation bound before release.

SSH home is selected from HOME, then HOMEDRIVE plus HOMEPATH, then USERPROFILE.
The selected home supplies `.ssh/known_hosts` and `~/` identity paths. A missing
or unusable home reports which home source needs correction. Host-key checking
is still required; choosing native does not repair a mismatched host key.

## Proxy and authentication

On Windows, the machine WinHTTP proxy controls GWZ. HTTPS_PROXY, ALL_PROXY and
NO_PROXY, including their lowercase forms, do not override it. Browser proxy
and PAC settings are separate. Your administrator can inspect the machine
configuration with `netsh winhttp show proxy`. GWZ refuses a proxy or bypass
form it cannot interpret and names that setting. Do not reset an administrator's
proxy merely to bypass a GWZ error.

The final guide will state whether a proxy's 407 authentication challenge is
supported after its 1.0.17 baseline row. Until then proxy authentication support
is unclaimed. An origin credential is never sent to the proxy.

GWZ starts HTTPS discovery without credentials. If the server offers NTLM,
Basic or Digest, your configured git helper is asked first. When the server
also offers Negotiate, a returned helper identity uses Negotiate. A
Negotiate-only server uses your Windows logon identity without asking a helper.
If no helper identity is available and the server offers Negotiate or NTLM,
GWZ can use your Windows logon identity. A helper timeout or cancellation ends
the operation. A credential the server rejected is not replaced by another
identity within that operation.

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

On macOS and Linux, this Windows logon identity is unavailable. Unsupported
schemes name the offered scheme and the native transport option. The final
migration notes will give the measured gwz 1.0.17 outcome on each platform.
