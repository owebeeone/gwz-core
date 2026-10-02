# TR2.22 configuration-view feasibility receipt

2026-10-03; macOS, Rust 1.95.0, actual `/usr/bin/git`. This is executed physical
feasibility for the proposed mechanism, not product implementation acceptance.

The chosen std-only prototype passes the full chain: actual Git NUL discovery,
actual Git parsing of bounded private source copies, ordered unconditional walk,
private flattened configuration and real credential fill. Original hasconfig B
wins; the flattened view offers A. Assertions cover duplicate/reset/null/empty,
representative escapes/non-UTF-8, system/XDG/global and PARAMETERS/COUNT overlays,
captured HOME, relative root/nested/missing/empty include sources, subsection
normalization, repeated identical SYSTEM/GLOBAL file occurrences, final origin
suppression of Apple installation/system/XDG roots, private modes and cleanup.
Synthetic credential bytes are compared/wiped without output. No protected
library source changed; the chosen mechanism needs no new binding or library API.

[Private runner, raw receipts and fingerprints](../../gwz-core-evidence/campaigns/https-integration/runs/2026-10-03-tr222-config-view/README.md)
require private-member access. Evidence was indexed retrospectively. The initial
unknown-scope failures have partial tool-output receipts; their full logs/frozen
sources were not retained and are explicitly unknown. Subsequent repeated-root
RED and corrected GREEN have frozen source/raw logs byte-for-byte. No fixture,
configuration secret copy, build or cache was archived. Archive verification
passes (5,004 migration records, no build/cache directories).

The product hasconfig regression remains RED until the mechanism is reviewed
and implemented. This prototype does not qualify cancellation, quotas, retained
worker/child cleanup, filesystem IO ceilings, or another platform. Initial native
Git root discovery is deadline/output bounded, with no claim that its original
file reads or internal memory allocation are capped. The mechanism draft names
that limitation, new preparation limits and private sensitive-copy ownership
for independent Safety review.
