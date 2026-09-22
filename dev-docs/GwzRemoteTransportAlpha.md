# Local macOS transport alpha

2026-09-22. Operator authorized enabling HTTPS in the installed local alpha,
while asking to conserve quota. This is a bounded candidate activation, not
production/release qualification. Review pending at this checkpoint.

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
