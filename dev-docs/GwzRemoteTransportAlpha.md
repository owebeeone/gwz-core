# Local macOS transport alpha

2026-09-22. Operator authorized enabling HTTPS in the installed local alpha,
while asking to conserve quota. This is a bounded candidate activation, not
production/release qualification. Accepted at the tuple below after retained Code/State GO; installed as
`/Users/owebeeone/.cargo/bin/gwz-alpha`, version `0.2.0-alpha.transport`.

`gwz_transport_candidate` continues to isolate the implementation/dependencies.
CLI network commands construct one local SSH+HTTPS host runtime per invocation,
register the exact request metadata and operation ID, use its backend throughout
dispatch, then finish the request and runtime. Normal errors and panics retire
the scope. Pending physical cleanup is reported separately from Git success.
Ordinary builds retain existing routing; the small dispatch extraction shares the
same handler match. The alpha has no physical carrier or remote endpoint mode.

Endpoint configuration uses HOME/SSH_AUTH_SOCK, system TLS trust, an optional
single additional PEM root from GIT_SSL_CAINFO or SSL_CERT_FILE, and endpoint-local
`gh` resolved through endpoint PATH with the endpoint environment. Authentication
uses existing anonymous discovery/gh challenge semantics; no generic credential
helper is introduced. Standard HTTPS_PROXY/https_proxy and ALL_PROXY/all_proxy
support HTTP(S) CONNECT proxies without URL credentials. Unsupported schemes,
URL credentials, paths and unsupported NO_PROXY syntax fail explicitly. Git
configuration proxy/trust parity, CA-directory/bundle support and other platforms
are not claimed by this alpha. No certificate-verification bypass is added.

Focused actual-binary tests use disposable keys, repositories, TLS CA/server and
a fake `gh` protocol peer: HTTPS clone/fetch/push (exact remote commit), gh failure,
unsupported proxy refusal, and SSH clone/fetch. No real GitHub account was tested.
The first HTTPS baseline fails certificate trust on the prior SSH-only binary;
a first self-signed server fixture also failed native trust and was corrected to
the established CA+leaf fixture. Failures remain evidence, not passing claims.
Private logs, build inputs and smoke scripts: transport-qualification/runs/2026-09-22-alpha-https
in gwz-core-evidence (private access). No public build depends on that archive.

The latest Q6 retirement fix is included, but Q6 aggregate review, sustained-memory
qualification, performance tuning, Windows integration and normal release gates
remain open. This local alpha exception does not resume that larger programme.

## Acceptance

Retained reports: root dev-docs/GwzRemoteTransportAlpha-ReviewCode-1.md and
GwzRemoteTransportAlpha-ReviewState-1.md. One P2 found during review: local
snapshots/tags depended on HTTPS configuration. One correction excludes those
variants; actual-binary regression proves isolation while remote variants retain
transport. Both axes GO, no open alpha findings. Q6/release scope remains separate.

- .: `84961962fb4879f591c7195fb035d8c34d920b38`
- gwz-core: `3b79fb26d731cce565a2319a2ae2d512d4d54c51`
- gwz-cli: `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`
- gwz-core-evidence: `73370827da996b8bdfdcc817e81edc0286b6abe1`
- gwz-transport: `aa40936d0805e8cb60f8027615abe20d4f2045e4`
- git2-rs: `ce78628308e11b4e8901d5061602619109bce21a`
- libgit2: `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`

Installed SHA256: `af16378a957786628f7be4c7feee8b4a1722ecd297392bdb39e902219aecf031`.
