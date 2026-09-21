# SSH N2b — selected admission and pool/worker integration

Status: implementation checkpoint; review pending. This slice composes the
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

## Validation

The isolated locked/offline Rust 1.95 SSH suite passes. The selected-pool gate
covers six concurrent identical opens collapsing to one physical authentication,
credential-offer facts on only the first setup, reuse after an alternate pathname
with identical bytes, reread rejection after file replacement, and queue expiry.
The full suite also passes the existing worker, supervised, network, route and
agent tests. The standalone `gwz-transport` suite passes its absolute-deadline
pool tests and existing tests.

This slice remains test-fixture attached until the later N3 production module and
backend entry-point attachment. Platform and selected-source qualification stay
in the operator-deferred batch. Production activation and the HTTPS path remain
outstanding. The accepted N2 design's 500 production/900 test budget governs
this integration; this working tree stays within that scope.
