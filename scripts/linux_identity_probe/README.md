# Linux durable-identity probe

This package is the executable R0-L gate for the checked-artifact Linux
provider. It is evidence tooling, not the production identity implementation.

## What the gate proves

The bar is the **identity contract**, not a filesystem name. A Linux volume is
above the bar when it can prove, through the same two syscalls the production
provider uses:

- a nonzero 16-byte external volume UUID from `FS_IOC_GETFSUUID`; and
- a 1..=128-byte persistent handle from `name_to_handle_at` on a retained
  no-follow descriptor, with an empty path and `AT_EMPTY_PATH` only.

Nothing here tests the filesystem's name to admit it. The name is *recorded*,
so evidence says which substrate answered, and so the warning a below-bar merge
prints can name it.

The one name-shaped test that remains is **volatility**: `tmpfs` and `ramfs`
are refused because their contents do not survive power loss, even though they
answer both syscalls (tmpfs publishes a random per-mount UUID on every kernel
that has the ioctl at all). The production provider takes that refusal by
superblock magic; this package mirrors it by name, and `run_probe.py` always
derives the name from the magic the kernel reported.

## The strong table

`provider.STRONG_TABLE` is `{ext4, f2fs, xfs}` — the Linux filesystems known to
call `super_set_uuid` in the kernel and therefore to satisfy the contract
today. It is **evidence vocabulary, not a production denylist**: it documents
what `gwz merge --filesystem-strict` admits on Linux at this commit. Nothing in
the provider, in this package, or in the shipped binary consults it to refuse,
and a filesystem outside it that answers the contract is admitted.

`ext4` is the **reference positive row**: it is the one built with a fixed
external UUID and carried through remount, descriptor substitution, and the
whole negative table. Every other strong-table filesystem the runner can build
is proved separately, under `strong_table` in the evidence, to satisfy the same
contract across a remount.

Below the bar today: `btrfs` (it never publishes its UUID to the VFS, so the
ioctl answers `ENOTTY`), every kernel before 6.9 (no ioctl at all), `tmpfs` and
`ramfs` (volatile), every network mount, and every filesystem without
persistent handles.

## Fail, not skip

The native workflow deliberately fails rather than skips unless both release
architectures can:

- create and remount an ext4 filesystem with a fixed external UUID;
- read that UUID through `FS_IOC_GETFSUUID`;
- query a persistent handle as described above;
- reacquire the same UUID, handle type, handle bytes, and sensitive/casefold
  parent modes after remount;
- prove that pathname replacement does not change the retained descriptor's
  handle;
- build every `provider.STRONG_TABLE_ROWS` filesystem on a loop device and
  reproduce its tuple across a remount (today: `xfs`, which has no
  case-folding mode, so its path-mode evidence is two `Sensitive` rows); and
- reject the exact 15-row negative provider table — including **real** tmpfs
  and overlay mounts, on which the contract is attempted for real, malformed
  and absent UUIDs, forbidden query forms, injected permission denial, and an
  injected unsupported empty-path query.

The tmpfs row's verdict comes from the volatility refusal; the overlay row's
from `name_to_handle_at` answering `EOPNOTSUPP` on an overlay without
`nfs_export`; the network row from a remote volume publishing no UUID.

The ordinary mount ID is recorded only as a diagnostic. It is excluded from
the durable tuple and may change across remount.

## Running it

Run the portable contract tests with:

```sh
python -m unittest scripts/linux_identity_probe/test_probe_contract.py -v
```

The native probe requires Linux root privileges, loop mounts, `mkfs.ext4`,
`mkfs.xfs`, `mount`, `umount`, and `chattr`. CI runs it on `ubuntu-24.04` and
`ubuntu-24.04-arm`, then validates that exactly `linux-x86_64` and
`linux-aarch64` evidence rows share the same core commit, workflow run, probe
source digest, and provider-table digest. The aggregate checkout recomputes
both digests, binds the rows to `GITHUB_SHA` and `GITHUB_RUN_ID`, and rejects
missing, unknown, or false fields anywhere in the closed evidence schema. Each
row must also report the native machine expected for its declared release
architecture.

The combined artifact is named `linux-durable-identity-evidence`. A release
build or a run missing either architecture does not satisfy R0-L.

## What this gate is NOT

Being below the bar is not a merge refusal. Since DR-1 ship (1) a `gwz merge`
runs on every filesystem; crash recovery is a capability decided once at merge
start, and a below-bar volume gets one warning and a merge that runs without
the catalog. `--filesystem-strict` is what turns the bar back into a refusal.
Read `docs/OperationModel.md`, "Checked Merge Artifacts And Filesystem
Identity", for the user-facing rule.
