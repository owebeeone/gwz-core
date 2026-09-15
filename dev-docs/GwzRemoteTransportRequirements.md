# GWZ Remote Transport Requirements

Status: draft, 2026-09-15. Requirements only. §7 lists the design decisions still
to make; none is taken.

## 1. Purpose

Two related pieces of work on how GWZ Core reaches Git hosts:

1. **Connection reuse.** A push or pull opens a new SSH connection for every
   libgit2 call, and each one costs 2–3.5 s to github.com. Reusing connections
   removes most of that.
2. **Placement.** Today the driver and core run on one machine, so core's network
   and credentials are the user's. Once a real link separates them, gwz has to
   decide where the connection to the Git host runs and where the credentials
   that authenticate it live. The gryth (formerly grip-lab) peer-to-peer network
   makes this pressing: credentials should sit on as few machines as possible,
   and a core that is not trusted to sign should still be able to work.

Both cover SSH and HTTPS, including HTTPS authenticated through `gh auth`.

## 2. Terms

- **Driver**: the CLI, gwz-py or another caller of GWZ Core (as in
  `GWZRequirements.md`).
- **Core**: GWZ Core, running where the workspace repositories are.
- **Link**: the connection between driver and core. Today it is in-process or on
  one machine.
- **Git host**: the server a remote URL names, such as github.com.
- **Traffic**: the network connection to the Git host and the Git data on it.
- **Authority**: what proves who gwz is and whom it trusts: SSH keys and agents,
  SSH host-key trust (`known_hosts`), HTTPS tokens and credential helpers.
- **Connection**: one SSH or TLS connection to a Git host. A **channel** is one
  Git command (`git-upload-pack`, `git-receive-pack`) on an SSH connection.
- **Placement**: which machine runs the traffic and which uses the authority.

## 3. Where things stand

### 3.1 Measurements

Measured 2026-09-15 from the operator's Mac to github.com. The prototype was a
throwaway (not committed): a registered libgit2 SSH transport whose streams are
channels on pooled `ssh2` sessions.

| Case | Result |
|---|---|
| libgit2's own SSH transport: 4 reads, a receive-pack advertisement, a depth-1 fetch | 6 connections, 16.5 s; 2.2–3.5 s each |
| The same six operations through the prototype | 1 connection, 7.1 s; reads after the first 0.67–0.74 s |
| OpenSSH, a new connection per advertisement | 2.0–3.1 s each |
| OpenSSH, 24 advertisements as channels on one connection | 0.44–0.63 s each, no failures |
| OpenSSH, 4 concurrent channels on one connection | 0.62 s in total |
| HTTPS, new connection | TLS complete within 45 ms; request about 0.30 s |
| HTTPS, reused connection | requests 0.20–0.34 s |

Not measured: pushing a pack through the prototype, and the cost of running
`gh auth git-credential` for each operation.

### 3.2 Code

- libgit2's SSH transport opens a socket and an SSH session for each stream and
  frees the session with it (libgit2 1.9.7 `transports/ssh_libssh2.c:180-199`,
  `774-897`). libgit2-sys 0.18.8 builds only this libssh2 backend
  (`build.rs:248-254`), and libgit2 never reads `~/.ssh/config`.
- A transport registered through `git2::transport::register` takes precedence over
  the built-in one, including for `git@host:path` URLs (`transport.c:56-60`,
  `98-99`). With `Transport::smart(remote, false, …)`, git2 hands the
  advertisement stream on to the negotiation (git2 0.21.0 `src/transport.rs:254-279`).
- libgit2 keeps an HTTP connection alive only within one remote connection
  (`transports/httpclient.c:1078`).
- Credentials (`src/git/gitbackend/transport_support.rs`): core reads an explicit
  identity file from its own filesystem, with no agent fallback (`:183`).
  Otherwise the agent is offered once (`:203-216`). HTTPS uses Git credential
  helpers under `CredentialHelperPolicy::AllowConfigured` (`:220-226`; the default,
  `backend.rs:44`). On the operator's machine the github.com helper is
  `gh auth git-credential`, so `gh auth` reaches gwz only through Git's helper
  configuration.
- Host keys: gwz sets no certificate callback, so libgit2 refuses keys that are not
  in `known_hosts` (`ssh_libssh2.c:752-767`).
- Timeouts are process-wide libgit2 socket settings (`transport_support.rs:56-64`).
- Remote work runs up to `max_connections_per_host` at a time, default 8
  (`src/operation/resolve_per_host.rs:2`; used by `push_member.rs:275` and
  `pull_head_member_preflight.rs:563-586`).
- Protocol (`protocol/gwz.taut.py`):
  - `RemoteSshIdentity.private_key_path` (`:1034-1036`).
  - `OperationAttribution.credential_ref`, a "driver-local credential handle; never
    a secret value". It is carried through but never used to authenticate (`:994`).
  - `TransportCapabilitiesResponse` (`:1056-1060`).
  - `TransportObservation`, with credential method, offered and authenticated
    flags, and key fingerprint (`:1064-1072`).

### 3.3 Existing requirements and rulings

- Core MUST NOT own credential storage (`GWZRequirements.md`, Non-Goals).
  Credential acquisition is delegated to caller policy, host configuration or
  adapter APIs (REQ-124).
- Explicit SSH identity MUST fail closed, without a Git CLI fallback (debt recovery
  requirements, 2026-09-06). The owner ruled out a Git CLI transport on 2026-09-06
  (gwz-dev `dev-docs/GwzRemoteAuthProposal.md` §6).
- Transport observations MUST distinguish offered from authenticated credentials.
- Core MUST be callable in-process and MUST NOT require a daemon (REQ-011,
  Non-Goals).
- Policy that varies by driver is a typed input (REQ-012).
- v0 may assume local caller authority; remote capability enforcement is deferred
  (REQ-013).
- The push plan (gwz-dev `dev-docs/GwzUrlSchemePushPlan.md`, phase 3) reduces how
  many remotes an operation contacts. This work reduces what each contact costs.

## 4. Deployments

| Deployment | What it needs |
|---|---|
| Same machine (today) | Speed; nothing else changes |
| Remote core with a person at the driver | The person's credentials, used without copying them to core |
| Unattended core (CI, servers) | Core's own credentials, as today |
| gryth peers | Credentials on as few peers as possible; peers not trusted to sign can still work |
| Git host reachable only from the driver (VPN) | Traffic that leaves from the driver's machine |

## 5. Requirements

Conventions follow `GWZRequirements.md`: `MUST`, `SHOULD`, `MAY`.

### 5.1 General

- **G1.** On one machine, behaviour MUST stay as it is today: the same identity
  selection, host-key refusals, credential helpers, errors and observations. Only
  the number of connections changes.
- **G2.** In every placement, core MUST NOT store credentials, and explicit
  identity MUST keep failing closed with no fallback.
- **G3.** Every connection MUST be observable: which identity authenticated it (key
  fingerprint or account), where its traffic ran, and where its authority was used.
- **G4.** These requirements apply to SSH and to HTTPS, including HTTPS
  authenticated through `gh auth`.
- **G5.** Behaviour MUST be the same on Windows, macOS and Linux.
- **G6.** Existing drivers MUST keep working against a newer core, and the reverse.
  Anything new on the link MUST be negotiated, not assumed.

### 5.2 Connection reuse

- **C1.** Within one operation, gwz MUST reuse an open SSH connection to the same
  Git host and authority. Reuse applies across repositories and across phases:
  reads, fetches, pushes and post-push reads.
- **C2.** A connection MUST be reused only for the same authority: user, host, port
  and credential. A request that selects one identity MUST NOT run on a connection
  authenticated as another.
- **C3.** Reuse MUST NOT change outcomes. A host key or identity refused on a new
  connection is still refused, and a failure after request bytes were sent surfaces
  as it does today. A cached connection found dead MAY be replaced, but only before
  any request bytes were sent.
- **C4.** Open connections to a Git host MUST NOT exceed `max_connections_per_host`.
- **C5.** Configured transport timeouts MUST apply to cached connections and their
  channels.
- **C6.** Connections MUST NOT outlive the gwz process, and idle connections MUST
  close within a bounded time.
- **C7.** Tests MUST be able to count the connections and channels an operation
  opens.
- **C8.** HTTPS SHOULD follow C1–C7 where that measurably helps. Connection setup is
  a small part of each HTTPS request (§3.1), so it ranks below SSH.

### 5.3 Placement

- **P1.** Core MUST be able to use authority held by the driver without that
  authority being copied to, or stored on, core's machine.
- **P2.** Authority a driver lends MUST be limited to the operation that needs it:
  only while that operation runs, and only for its Git hosts.
- **P3.** Core MUST be able to use its own authority with no credentials on the
  driver side, as today.
- **P4.** gwz SHOULD offer a placement in which core never holds a credential or a
  signing capability, not even during an operation.
- **P5.** gwz SHOULD be able to run traffic from the driver's machine for Git hosts
  core cannot reach.
- **P6.** Identity selection MUST mean the same thing in every placement. This
  covers `--identity`, configured remote identities, and offering only the
  selected key.
- **P7.** SSH host keys MUST be verified in every placement, and unknown or
  mismatched keys MUST be refused as today. The observation MUST say whose
  `known_hosts` decided.
- **P8.** A placement that keeps authority with the driver MUST NOT pass core the
  driver's full HTTPS token (such as the `gh auth` token) unless that placement
  says so. A way to keep tokens off core, or to narrow them to the operation,
  SHOULD exist.
- **P9.** A prompt that needs the person MUST reach them at the driver: Touch ID, a
  hardware-key touch, a `gh` login. Waiting for one MUST NOT trip network timeouts.

## 6. Out of scope

- Building the driver–core link itself. This document says what it must carry.
- Changing how identities are selected (gwz-dev
  `dev-docs/GwzRemoteAuthProposal.md` §2.2).

## 7. Design decisions to make

None is taken. Each lists the options seen so far and what follows from them.

### Connection reuse

**D1. How long cached connections live.** Options: for one operation, for the
process with an idle timeout, or for a driver session.
- Per operation is simplest and leaves nothing open between operations, but a
  driver that runs several operations pays for connections each time.
- Per process or per session reuses more for long-lived drivers (gwz-py, daemons),
  but keeps authenticated connections open between operations, which loosens P2.

**D2. Exclusive or shared connections.** Options: one connection per concurrent
worker, or all work to a host multiplexed on one connection.
- Exclusive: the first parallel wave opens up to the per-host limit, and later
  phases reuse those connections. It drives libssh2 the way libgit2 does today:
  blocking, one thread per session.
- Shared: one connection even for parallel work (GitHub accepted 4 concurrent
  channels). A libssh2 session can't be used from several threads at once, so
  this needs a non-blocking I/O loop per connection. It gains little over
  exclusive when the work is already parallel.

**D3. SSH implementation.** Options: the `ssh2` crate over the libssh2 that core
already links (the prototype), `russh`, or the OpenSSH executable with
ControlMaster.
- `ssh2` keeps today's crypto, key formats and agent support (Windows agents
  included). Its blocking API fits exclusive connections.
- `russh` adds a second crypto stack and an async runtime to core (gwz-core has no
  tokio today). It fits shared connections, but agent and key-format parity would
  have to be proved again.
- OpenSSH needs the least code, but has no ControlMaster on Windows (G5). It also
  reads `~/.ssh/config`, which is ambient authority at odds with explicit
  identity, and it sits close to the rejected Git CLI transport.

**D4. What counts as the same authority (C2).** Candidates: user, host and port,
plus some of the identity file path, its content fingerprint, the agent socket,
the key fingerprint that authenticated, and, for lent authority, the driver
session.
- Keyed on the agent socket alone, a connection authenticated by one agent key can
  serve a request meant for another. That is the wrong-account hazard in
  `GwzRemoteAuthProposal.md` §2.1.
- Lent authority keyed without the driver session lets two drivers, or two people
  on a shared core, share a connection.

**D5. HTTPS reuse.** Options: leave libgit2's HTTP transport alone, register an
HTTPS transport backed by a pooled Rust HTTP client, or only reuse credential
helper results within an operation.
- Leaving it alone costs little speed (§3.1).
- A registered HTTPS transport replaces libgit2's TLS, proxy, redirect and
  authentication-challenge handling, which is a large surface to match. It is also
  what carrying HTTPS through the driver would need (D7).
- Reusing helper results avoids running `gh auth git-credential` for every
  repository, but holds a token in core's memory for the whole operation.

**D6. Reporting reused connections.** A reused connection offers no credential, so
today's `TransportObservation` would read as "nothing offered".
- A new optional field, such as a connection id or a reused flag, is an additive
  protocol change for gwz-cli and gwz-py.
- Copying the authenticating observation keeps the protocol as it is, but hides how
  many connections were opened (C7).

### Placement

**D7. Which placements to offer, per transport.**
- **Core:** as today.
- **Forwarded:** traffic at core, authority at the driver.
- **Relay:** traffic and authority both at the driver.

Implications:
- Forwarded SSH needs a few small driver calls per new connection to sign the
  login. Reusing connections means fewer of them.
- Forwarded HTTPS hands core a bearer token (D10).
- Relay needs two-way byte streams with flow control over the link. Every clone and
  push then travels through the driver's network, such as a laptop on hotel wifi
  carrying a datacenter core's data. It is the only placement that satisfies P4.

**D8. How placement is chosen.** Options: automatically from what the driver says it
can lend, by a command-line option or request policy, by workspace or per-remote
configuration, or a default with overrides.
- Automatic spares a person at a remote core from thinking about it. But a driver
  that gains or loses an agent silently changes who authenticates, unless G3
  reports it prominently.
- An option is explicit but repetitive.
- Per-remote configuration fits hosts that only one side can reach, and gryth peers
  with fixed roles.
- Whichever is chosen, placement is a typed policy input to core (REQ-012).

**D9. Host-key trust when authority is at the driver.** Options: core's
`known_hosts`, the driver's (core reports the key it saw), or both must agree.
- Core's file is simplest, but the driver then signs a login it cannot vouch for.
- The driver's file keeps trust with the person. A compromised core can still
  report a false key, so it protects against the network, not against core.
- Requiring both is strictest, but means keeping two files current.
- In relay the driver owns the connection, so its file decides.

**D10. HTTPS and `gh auth` when authority is at the driver.** Options: the driver
runs the credential helper and passes the token; the driver mints a narrower,
short-lived token (a fine-grained token, or a GitHub App installation token for the
operation's repositories); or HTTPS is relayed, so tokens never leave the driver.
- Passing the token gives core the `gh auth` token, with every scope granted at
  login and valid until revoked. That conflicts with P8 unless the placement says
  so.
- Narrow tokens need GitHub App or token setup that `gh` doesn't do today, and they
  only work for GitHub.
- Relay keeps tokens off core, but needs a registered HTTPS transport (D5) and
  routes traffic through the driver.

**D11. Limits on lent authority (P2).** Options: sign or supply only during an
operation the driver started, only for that operation's hosts, with or without
confirming each connection.
- An SSH login signature doesn't name a repository. A driver can therefore limit
  hosts but not repositories, unless it owns the connection (relay).
- Limits need the operation's remotes, which come from core's manifest. The driver
  either trusts core's list or reads the manifest itself.

**D12. Identity references across machines.** Options: paths resolved on the
machine that holds authority, a key reference (public-key fingerprint or driver
handle, possibly `credential_ref`), or both.
- A path in `gwz.yml` would name a file on a different machine whenever placement
  changes, so one workspace could mean different keys.
- Fingerprints are stable across machines and can select keys held in an agent.
  That relies on exact-agent support, which is unavailable today
  (`TransportCapabilitiesResponse.exact_agent_identity`).
- Reusing `credential_ref` joins attribution to authority, a question
  `GwzRemoteAuthProposal.md` §2.2 left open.

### Link and deployments

**D13. What the link carries, and what happens without it.** A forwarded placement
needs requests from core to the driver during an operation, some of which wait for
a person (P9). Relay needs byte streams with flow control and cancellation.
- Older drivers support neither, so core has to detect the capability (G6) and then
  fall back to core placement or refuse. A silent fallback changes who
  authenticates.

**D14. gryth peers.** Can a peer lend authority to a peer that isn't its direct
driver? Can a peer carry another peer's traffic? Is placement a fixed role per peer,
or chosen per operation?
- Every hop that passes on a signing request or a token extends exposure to that
  hop. Lending across several hops needs limits (D11) that the peers in between
  cannot widen.
- Fixed peer roles fit configuration (D8).

**D15. `~/.ssh/config`.** libgit2 has never read it. When the driver owns the
connection (relay), users may expect host aliases and `ProxyJump` to work.
- Honouring it brings in ambient settings such as `IdentityFile`, which can
  conflict with explicit identity (G2, P6).
