# GWZ transport: the SSH key types and signature algorithms 1.0.17 uses

Date: 2026-10-02. Status: TR2.8's list ([amendment 2](GwzTransportReleasePlanAmendment-2.md)
§3.19), recorded before its code; revision 2 adds the transport's TR2.8 column, which TR2.8's
tests (§6) evidence, and §5's row that those tests found.

## 1. How 1.0.17 built and calls libssh2

- **Versions.** gwz-core `v1.0.17` locks git2 0.21.0, libgit2-sys 0.18.8+1.9.7, libssh2-sys
  0.3.3 and openssl-sys 0.9.117, with no openssl-src (`Cargo.lock:291, 678, 692, 769`; gwz-cli
  `v1.0.17` locks the same, `Cargo.lock:406, 830, 844, 927`). libssh2-sys 0.3.3 bundles libssh2
  `1.11.1_DEV` (`include/libssh2.h:51`). Line numbers below are in that crate's `libssh2/src/`,
  in libgit2-sys 0.18.8's `libgit2/src/libgit2/transports/`, and in gwz-core at `v1.0.17` or, for
  the transport, at `bb67a826`.
- **The crypto backend** (libssh2-sys `build.rs`):
  - macOS and Linux: `LIBSSH2_OPENSSL` (`build.rs:128`), with the system OpenSSL that
    openssl-sys finds. The installed 1.0.17 binary links Homebrew's `openssl@3`. With OpenSSL
    1.1.1 or later RSA (SHA-1 and SHA-2), ECDSA and Ed25519 are on (`openssl.h:132-161`).
  - Windows: `LIBSSH2_WINCNG` (`build.rs:97-115`; the `openssl-on-win32` feature is off). WinCNG
    has no Ed25519 (`wincng.h:74`), and no ECDSA without `LIBSSH2_ECDSA_WINCNG` (`wincng.h:88-91`),
    which nothing defines.
  - Everywhere, DSA is off: nothing defines `LIBSSH2_DSA_ENABLE` (`crypto_config.h:23-26`).
- **The agent.** 1.0.17 answers an SSH challenge with `Cred::ssh_key_from_agent`
  (`transport_support.rs:243`). libgit2 lists the agent's keys and calls `libssh2_agent_userauth`
  on each in turn, moving to the next key after any failure (`ssh_libssh2.c:236-289`). libssh2
  keeps every key the agent lists, whatever its type (`agent.c:610-742`), and authenticates each
  through `_libssh2_userauth_publickey` with `agent_sign` as its signer (`agent.c:888-909`).
- **Key files.** `--identity` and `--remote-identity` use `Cred::ssh_key(user, None, path, None)`
  (`transport_support.rs:211`): `libssh2_userauth_publickey_fromfile` with no public key and no
  passphrase (`ssh_libssh2.c:306-313`).

## 2. What decides a key's fate on the agent path

1. **The backend signs nothing.** The agent signs and libssh2 forwards its bytes, so the
   backend matters only for RSA's upgrade (§3), host keys and key files. WinCNG's missing
   Ed25519 and ECDSA do not shorten 1.0.17's agent list on Windows.
2. **The method** offered and signed is the key blob's type name (`userauth.c:1556-1575`), except
   that an RSA key, and with OpenSSL an RSA certificate, is upgraded from `server-sig-algs` (§3).
3. **The agent's flags.** `agent_sign` asks for SHA-2 only for exactly `rsa-sha2-512` or
   `rsa-sha2-256` (`agent.c:476-489`). Every other method, an RSA certificate's included, asks
   with no flags.
4. **The reply** must name the method, or a certificate's base type (`plain_method`,
   `userauth.c:1259-1311`). Otherwise `agent_sign` fails with `ALGO_UNSUPPORTED`
   (`agent.c:556-570`), and libssh2 offers the key once more under its own type name
   (`userauth.c:1754-1764`). Any other signer error fails the key (`userauth.c:1765-1773`).
5. **The framing.** libssh2 keeps only the reply's first signature string (`agent.c:572-591`)
   and sends `string(method) string(signature)`. For a security-key method it sends the signer's
   bytes unframed instead, expecting the signature string, the flags and the counter
   (`userauth.c:1808-1821`), which `agent_sign` dropped.
6. **After any failure** of a key, the server's refusal, the agent's refusal or a mismatch,
   libgit2 tries the next key (§1).

## 3. RSA and SHA-1

With OpenSSL, `ssh-rsa` and `ssh-rsa-cert-v01@openssh.com` are upgraded to the first of
`rsa-sha2-512`, `rsa-sha2-256`, `ssh-rsa` that the server's `server-sig-algs` lists
(`openssl.c:5207-5219`, `userauth.c:1410-1474`), a certificate keeping its suffix
(`userauth.c:1476-1487`). WinCNG upgrades `ssh-rsa` only (`wincng.c:4168-4176`). libssh2 sends
`ext-info-c` (`kex.c:4199`) and reads `server-sig-algs` from `SSH_MSG_EXT_INFO`
(`packet.c:848-898`), which travels encrypted, after the key exchange.

`ssh-rsa` (SHA-1) is used in exactly three cases, which the operator kept on 2026-10-02
("keep sha-1 fallback"):

1. **No `server-sig-algs`:** nothing is upgraded (`userauth.c:1379-1382`).
2. **`server-sig-algs` lists `ssh-rsa` and no `rsa-sha2-*`:** the upgrade picks `ssh-rsa`.
3. **Once per key, the agent answers a `rsa-sha2-*` request with an `ssh-rsa` signature:**
   `ALGO_UNSUPPORTED` (`agent.c:560-563`), and the key is offered again as `ssh-rsa`
   (`userauth.c:1754-1763`).

When `server-sig-algs` lists none of the three, the upgrade fails with `METHOD_NONE` before
anything is sent (`userauth.c:1503-1506`), and the key fails. libssh2 then keeps that key's
method (`userauth.c:1556`, `:1585-1586`), so every later key of the login fails the same way:
an RSA key ahead of a usable key, against a server that accepts no RSA algorithm, fails the
login. The transport calls the same function, so it inherits this.

RSA certificates take SHA-1 more often in 1.0.17:

- the agent is never asked for SHA-2 for `rsa-sha2-*-cert-v01@openssh.com` (§2, item 3), so its
  `ssh-rsa` reply makes case 3, and the certificate is offered again as
  `ssh-rsa-cert-v01@openssh.com`, which OpenSSH 8.8 and later refuse by default;
- against OpenSSH 7.7 or older the certificate is not upgraded (`userauth.c:1391-1408`);
- on WinCNG it is never upgraded.

The 28-byte tests at `userauth.c:1402-1404` and `:1477-1478` compare the address of the method
pointer with the name, so they hold for every 28-byte method, which is only
`ssh-rsa-cert-v01@openssh.com`.

## 4. The list

"Ends the login": the transport's signer fails and the error ends the whole authentication
(`agent_auth.rs:106-108`). It happens only after the server accepts the key's query
(`SSH_MSG_USERAUTH_PK_OK`); a key the server refuses moves the transport to the next key, as in
1.0.17.

| Agent key type | Algorithm | 1.0.17, macOS and Linux | 1.0.17, Windows | Transport at `bb67a826` | Transport, TR2.8 |
| --- | --- | --- | --- | --- | --- |
| `ssh-ed25519` | `ssh-ed25519` | yes | yes | yes | yes |
| `ecdsa-sha2-nistp256`, `-nistp384`, `-nistp521` | the same | yes | yes | ends the login (`agent_auth.rs:179-181`) | yes |
| `ssh-rsa` | `rsa-sha2-512`, `rsa-sha2-256` | yes, when `server-sig-algs` lists it | yes | yes | yes |
| `ssh-rsa` | `ssh-rsa` | yes, in §3's three cases | yes | ends the login: `ssh-rsa` is refused, and an `ssh-rsa` reply is malformed (`agent_client.rs:82-84`) | yes, in §3's three cases |
| `ssh-dss` | `ssh-dss` | yes, where the server accepts it | yes | ends the login | yes, where the server accepts it |
| `sk-ssh-ed25519@openssh.com`, `sk-ecdsa-sha2-nistp256@openssh.com` | the same | no: the agent signs (asking for the touch) and the server refuses the malformed signature (§2, item 5) | no | ends the login | yes |
| `ssh-ed25519-cert-v01@openssh.com`, `ecdsa-sha2-nistp*-cert-v01@openssh.com` | the base type | yes | yes | ends the login | yes |
| `ssh-rsa-cert-v01@openssh.com` | `rsa-sha2-*-cert-v01`, `ssh-rsa-cert-v01` | only `ssh-rsa-cert-v01`, where the server accepts it (§3) | only `ssh-rsa-cert-v01` | ends the login | yes: `rsa-sha2-*` when `server-sig-algs` lists it, else as §3 |
| `sk-*-cert-v01@openssh.com` | the security key's | no, as the security key | no | ends the login | yes |
| `ssh-dss-cert-v01@openssh.com` | — | no: no `plain_method` entry, so the reply mismatches twice | no | ends the login | skipped, the next key tried |
| any other, such as `ssh-xmss@openssh.com` | its type name | offered as is; the next key after a refusal | the same | offered; ends the login if the server accepts the query | skipped, the next key tried |

And two outcomes that are not types:

- **The agent refuses to sign** (a security key whose device is absent, a declined
  confirmation): 1.0.17 tries the next key. The transport at `bb67a826` ends the login
  (`PermissionDenied`); with TR2.8 it tries the next key.
- **`METHOD_NONE`** (§3): 1.0.17 reports an authentication failure. The transport at `bb67a826`
  ends the login with an I/O error (`agent_auth.rs:124-128`); with TR2.8 it reports an
  authentication failure, the later keys failing as §3 says.

Evidence: the macOS and Linux column is read from the source above and was observed with the
installed 1.0.17 binary (`gwz 1.0.17`, Homebrew OpenSSL 3), cloning over SSH from a disposable
OpenSSH 10.3p1 `sshd` or from `tests/transport_backend/password_sshd.py`, whose logs name each key
and algorithm the server accepted or refused. Each row matched the reading. Security keys were
served by a software authenticator whose signatures OpenSSH 10.3p1's own client authenticates
with. The Windows column is read from the source alone.

## 5. Key files (`--identity`, `--remote-identity`)

1.0.17 derives the public key with OpenSSL's PEM reader, then the OpenSSH container
(`openssl.c:4638-4731`), and signs with the type's file reader: RSA (`openssl.c:1648-1673`) and
ECDSA (`:3998-4023`) read PEM, then the OpenSSH container; Ed25519 reads the OpenSSH container only
(`:2536-2597`). On WinCNG only the traditional `RSA PRIVATE KEY` container is read
(`wincng.c:915-944`, `:3225-3248`).

The transport reads the file itself, checks the container (`ssh_key_container.rs`), and signs
through `libssh2_userauth_publickey_frommemory`, whose readers take PEM or the OpenSSH container
for every type (`openssl.c:4985-5092`, `:1238-1269`, `:2035-2066`, `:2674-2704`). OpenSSL's PEM
reader and libssh2's OpenSSH reader both skip whatever surrounds the key's block
(`pem.c:741-747`, `:815-827`).

| File | 1.0.17, macOS and Linux | Transport at `bb67a826` | Transport, TR2.8 |
| --- | --- | --- | --- |
| RSA, ECDSA P-256, P-384, P-521: OpenSSH, PEM (`RSA PRIVATE KEY`, `EC PRIVATE KEY`), PKCS#8 with a named curve | yes | yes | yes |
| ECDSA: PKCS#8 with explicit curve parameters, as LibreSSL's `ssh-keygen -m PKCS8` writes | yes | refused: the parameters end past the 128 decoded bytes checked (`ssh_key_container.rs:6, 185`) | yes |
| Ed25519: OpenSSH | yes | yes | yes |
| Ed25519: PKCS#8 (`openssl genpkey`) | no: the public key is offered, then signing fails | yes | yes |
| text before the key's block (`openssl pkcs12 -nodes`'s "Bag Attributes") | yes | refused before any open (`ssh_key_container.rs:236-238`) | yes |
| another PEM block first (`openssl ecparam -genkey`'s `EC PARAMETERS`) | yes | refused | yes |
| a PEM block after the key's (a certificate) | yes | refused (`ssh_key_container.rs:260-262`) | yes |
| PKCS#8 with its optional attributes (`[0]`) | yes | refused (`ssh_key_container.rs:189`) | yes |
| encrypted, in any container | no: no passphrase is passed. An encrypted OpenSSH key fails (`pem.c:468-473`). For an encrypted PEM key OpenSSL's default callback asks the terminal while the public key is derived (`openssl.c:4666`), and the signing read passes a null passphrase to `passphrase_cb`, which takes its `strlen` (`openssl.c:1166-1168`, `:1228-1229`); read from the source, not run | refused before any open, as designed | refused before any open |
| DSA; a security key's file | no: DSA is off (§1), and no host-key method signs for a security key (`hostkey.c:1346-1375`) | no, for the same reasons | no |

The five rows refused at `bb67a826` were gaps: 1.0.17 authenticates with each of them (observed,
with RSA and ECDSA keys). The first revision of this list missed the explicit-parameter row, which
TR2.8's tests found: ECDSA PKCS#8 files made by macOS's `ssh-keygen` carry explicit parameters.

## 6. What TR2.8 changes, and its tests

- **Every type 1.0.17 uses, and more.** `agent_keys.rs` lists the types of §4's TR2.8 column,
  the algorithm each method asks the agent for, RFC 8332's flags, and each signature's shape,
  which is checked before libssh2 sees it. A security key's signature string, flags and counter
  go to libssh2 unframed, as it sends them. An RSA certificate asks for SHA-2, which 1.0.17's
  `agent_sign` never does. The list is then at least 1.0.17's on every platform.
  `agent_keys.rs` is platform-neutral; the signing bridge (`agent_auth.rs`) is still Unix-only,
  so Windows takes the same list when Phase 4 qualifies its agent bridge.
- **SHA-1 exactly as libssh2.** The method is still libssh2's: §3's cases 1 and 2 reach the
  signer as `ssh-rsa`. For case 3 the signer returns `ALGO_UNSUPPORTED` once per key, and only
  for an `ssh-rsa` answer to a `rsa-sha2-*` request, so libssh2 offers the key once more as
  `ssh-rsa`. That second offer of one key is the only one.
- **Skipped types.** A key of a type outside the list is never offered, as the agent design's §5
  now says, and the next key is tried. 1.0.17 offers such a key, and would sign with it if a
  server accepted its type; none of OpenSSH's default builds does.
- **The agent's refusal** fails that key only, and the agent connection stays usable, so the
  next key is tried, as in 1.0.17. A malformed reply still ends the login.
- **`METHOD_NONE`** fails the key as an authentication failure, as in 1.0.17.
- **Key files.** The container check accepts §5's five refused forms: text and other PEM
  blocks around the key's are skipped, PKCS#8's optional fields after the key are allowed, and
  the structure check reads 1024 decoded bytes. It still requires exactly one unencrypted key,
  and refuses `ENCRYPTED PRIVATE KEY`, any other unknown `PRIVATE KEY` label, and a PEM header in
  any block, which OpenSSL decrypts whatever the block's label.
- **No route goes.** Amendment 1's route check, which would have sent such remotes down the
  native path before any open, was never built.

Tests, in `src/git/endpoint/ssh_tests/`, against a disposable OpenSSH `sshd` (`ssh_fixture.rs`,
now with configuration lines and a log) or `password_sshd.py`, which now sends a test's
`server-sig-algs`, or none, and verifies DSA:

- `key_types.rs`: ECDSA of each curve; both security keys; a certificate of each type, RSA with
  `rsa-sha2-512`, against an `sshd` that trusts a test user CA; DSA; a skipped DSA certificate and
  `ssh-xmss@openssh.com` before a usable key; and a security key whose device is absent before
  a usable key. Ed25519 and RSA with SHA-2 remain `agent_auth.rs`'s rows.
- `rsa_sha1.rs`: §3's three cases; `rsa-sha2-*` whenever `server-sig-algs` lists it, first or
  not; the key failing when it lists none of the three; and the later keys failing after that.
- `local_endpoint.rs`: end to end, the endpoint production builds authenticates with a security
  key, then with an ECDSA key, and serves a Git exchange on each connection.
- `agent_keys.rs`: the list, each method's algorithm and flags, each signature's shape, the one
  downgrade, and the agent's refusal, without a server.
- `key_files.rs` and `key_container.rs`: §5's forms on the production path, and the refusals.

The security keys sign through **the fixture's software authenticator**:
`tests/transport_backend/key_agent.py`'s `sk` signer, driven by `ssh_tests/key_fixture.rs`. It
builds what a FIDO authenticator signs (PROTOCOL.u2f: SHA-256 of the application, the flags with
user presence set, a counter, SHA-256 of the data) and has a private `ssh-agent` sign it with
the plain key that backs the security key, so its signature is the one a hardware key holding
that private key would make. OpenSSH 10.3p1's own client authenticates with it. The manual row
with a hardware key needs the operator's go and was not run.
