# SSH N2b — selected admission and pool/worker integration

Status: **accepted at root `9459ae4c2f5ad5061a2eaba92785a1f87bece938`, core
`6616a2cd66d64f04f9eb3c370e8a3a677fba1c36`, evidence
`593d2c36780d6278eee21e67dc0cc102673ed887`, and transport
`16a383e7d1c0e7e3234006688986afc2c6e54ca5` after retained Code and State
re-verdicts GO; this accepts the fixture-attached N2b scope only.** This slice composes the
accepted N2a selected-key snapshot and native authentication seams with the
existing SSH worker and physical pool. It does not attach a production route,
change the CLI/core interface, or activate the capability.

Selected opens now carry an owned pathname into the worker. The worker creates
one absolute request deadline, admits the file through a supervised Job before
pool lookup, and interns the exact bytes into the endpoint-local registry. A
successful admission changes the request to its opaque explicit identity and
only then invokes the existing pool checkout. Every selected open rereads the
current file, including a request that may reuse an idle physical connection.

The connector resolves an admitted identity to its strong snapshot pin; it does
not reopen the path. The native selected-key result transfers the connection and
pin into the resource. The resource retains that pin through idle, active,
reclaim and disposal states. A joined successful setup promotes the candidate to
proven; candidate status alone never authorizes a lease. Existing ambient and
agent paths retain their prior behavior.

The endpoint shutdown owner now contains both the physical pool and pending file
admission Jobs. Shutdown cancels and disposes both domains, reports pending
admissions separately, and retains the owner when either domain has not actually
finished. The pool receives the same absolute deadline used by file admission,
so allocation and connect clocks cannot extend the caller's budget. The
transport pool changes are covered by direct absolute-deadline tests.

Remediation round 1 closed the review findings for absolute deadline handling
through interaction and ready states, and for causal admission ownership and
retained-cleanup evidence. See [Code review](../../dev-docs/GwzRemoteTransportSshN2b-ReviewCode-1.md),
[State review](../../dev-docs/GwzRemoteTransportSshN2b-ReviewState-1.md), and the
[remediation plan](../../dev-docs/GwzRemoteTransportSshN2b-RemPlan-1.md).

## Validation

The isolated locked/offline Rust 1.95 SSH suite passes. The selected-pool gate
covers six concurrent identical opens collapsing to one physical authentication,
credential-offer facts on only the first setup, reuse after an alternate pathname
with identical bytes, reread rejection after file replacement, queue expiry,
stalled-admission expiry without native setup, retained admission charge through
combined shutdown, late-result disposal, and active-stream progress while a
later admission is stalled.
The full suite also passes the existing worker, supervised, network, route and
agent tests. The standalone `gwz-transport` suite passes its absolute-deadline
pool tests and existing tests.

This slice remains test-fixture attached until the later N3 production module and
backend entry-point attachment. Platform and selected-source qualification stay
in the operator-deferred batch. Production activation and the HTTPS path remain
outstanding. The accepted N2 design's 500 production/900 test budget governs
this integration; this working tree stays within that scope.
