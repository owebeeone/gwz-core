# Linux durable-identity probe

This package is the executable R0-L gate for the checked-artifact Linux
provider. It is evidence tooling, not the production identity implementation.

The native workflow deliberately fails rather than skips unless both release
architectures can:

- create and remount an ext4 filesystem with a fixed external UUID;
- read that UUID through `FS_IOC_GETFSUUID`;
- query a 1..=128-byte persistent handle from a retained no-follow descriptor
  with an empty path and `AT_EMPTY_PATH` only;
- reacquire the same UUID, handle type, handle bytes, and sensitive/casefold
  parent modes after remount;
- prove that pathname replacement does not change the retained descriptor's
  handle; and
- reject the exact 15-row negative provider table, including real overlay and
  tmpfs mounts, malformed and absent UUIDs, forbidden query forms, injected
  permission denial, and an injected unsupported empty-path query.

The ordinary mount ID is recorded only as a diagnostic. It is excluded from
the durable tuple and may change across remount.

Run the portable contract tests with:

```sh
python -m unittest scripts/linux_identity_probe/test_probe_contract.py -v
```

The native probe requires Linux root privileges, loop mounts, `mkfs.ext4`,
`mount`, `umount`, and `chattr`. CI runs it on `ubuntu-24.04` and
`ubuntu-24.04-arm`, then validates that exactly `linux-x86_64` and
`linux-aarch64` evidence rows share the same core commit, workflow run, probe
source digest, and provider-table digest. The aggregate checkout recomputes
both digests, binds the rows to `GITHUB_SHA` and `GITHUB_RUN_ID`, and rejects
missing, unknown, or false fields anywhere in the closed evidence schema. Each
row must also report the native machine expected for its declared release
architecture.

The combined artifact is named `linux-durable-identity-evidence`. A release
build or a run missing either architecture does not satisfy R0-L.
