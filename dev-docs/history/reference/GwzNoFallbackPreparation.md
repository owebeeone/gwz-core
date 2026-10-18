# No-fallback preparation: baseline and compatibility

Date: 2026-09-20. Status: **P0 registration and P1 baseline complete; P2 accepted**. Operator resumed the accepted plan with "go". No production
dependency or runtime activation is implied. Current execution state is in
the [first-package checkpoint](GwzNoFallbackCheckpoint.md) and root
[program checkpoint](../../dev-docs/CurrentProgramCheckpoint.md).

## P0: registered fork and source identities

The installed coordinator completed:

```sh
gwz repo clone git@github.com:owebeeone/git2-rs.git git2-rs
```

Member: `mem_git2_rs`, path `git2-rs`, origin the URL above. GWZ generated the
manifest, lock and integrity-marker changes. No manual configuration edit,
remote creation, publication or push was performed. The checkout was clean
after cloning. Read `git2-rs/README.md` and `CONTRIBUTING.md`; no additional
AGENTS.md was found in that checkout.

| Identity | Revision/version |
|---|---|
| Cloned fork HEAD | `f42a01267a3042b26d30e9d8acf286c6c739bd8a` |
| Fork root package at clone | `git2` 0.21.0, Rust minimum 1.87 |
| Fork path sys package at clone | `libgit2-sys` 0.18.7+1.9.6 |
| Fork recorded C gitlink at clone | `26055f5af74ab1cf636d272e8a34315496d3f06f` |
| Qualified published git2 0.21.0 source / local tag `git2-0.21.0` | `dffaf272eb0e62ac15b74283c4e488252db9afc3` |
| Qualified published libgit2-sys 0.18.8+1.9.7 source | `6c93812dbc1c34aef6e6464a645545b4a4299807` |
| C gitlink in that sys source | `49e408b3208bc3093757a1c2db938d3590f3f412` |

Published source identities come from cached `.cargo_vcs_info.json` and are
resolvable in the cloned repository. The matching source commits are distinct:
the production combination is published git2 0.21.0 with sys 0.18.8, not a claim
that one upstream checkout supplies both unchanged.

The fork HEAD is newer than the git2 release but has an older path sys package
than the current GWZ lock. Both binding files differ from the qualified archive
hashes. Therefore the cloned HEAD must not silently replace the qualified source
or carry an unrelated Rust upgrade/native downgrade into GWZ. Lane 2 must define
an exact release-based source and sys resolution before testing its candidate.
No branch/submodule/source alignment has yet been performed. The C gitlink is a
recorded source identity, not proof of an initialized or tested native checkout.

Retain the accepted provenance in `tests/transport_native/binding-pin.json`:
git2 archive SHA-256
`ddddbf932745a6be37109b6112d3ee09696106f848449069d3a57bba937ab82e`,
patch SHA-256
`51586ea398130dbb36e3c9f59a3117b5c19b3a5c223d5bb62e266077a8d6c3e6`.
The local `gwz-git2` 0.21.0-gwz.1 packaging recipe remains a candidate, not a
published package or an automatically approved choice of production source.

## P1: source and command baseline

At resumption: core `0154d36d3412d26b0a8431ece67f64abd214f5d6`, CLI
`7db07bbdefd2897c07fd0e9f550bf032bd8b1314`, root
`d0edc9611c9866983b4e8b3c30aed0c160384da4` before registration.
Local Git oracle: 2.52.0. Fixtures will record the actual oracle executable,
version, object format and configuration rather than assuming ambient defaults.

Direct product Git launches confirmed in core:

| Operation | Source at baseline | Current delegation |
|---|---|---|
| Ordinary commit | `src/git/gitbackend/repository.rs:300` | `git commit`, `-m`, optional `-a` |
| Tag creation | `src/git/gitbackend/refs.rs:172` | `git tag`, explicit annotation/signing and ambient config |
| Tag deletion | `src/git/gitbackend/refs.rs:233` | `git tag -d` |
| Anonymous local fetch fallback | `src/git/gitbackend/transport.rs:341` | `git fetch --no-write-fetch-head --no-tags` with explicit refspecs |
| Path-filtered log | `src/operation/commit_log/mod.rs:320` | repeated `rev-list`, revision exclusions, first-parent and pathspecs |

Tag deletion belongs to lane 3 even though the older gap document describes
creation alone. The final audit must also inspect indirect launch wrappers and
distinguish test fixture calls from product execution. Current call-site counts
do not substitute for the final runtime no-Git gate.

## Consumer and platform baseline

The integrator owns all manifests, locks and native source pins. Initial direct
consumer inventory (refresh again before activation):

| Consumer | Existing git2 requirements |
|---|---|
| core | 0.21; https, ssh, unstable-sha256 |
| core/crates/repo-inspect | 0.21; defaults disabled, unstable-sha256 |
| core/crates/local-testrepo | 0.21; defaults disabled, unstable-sha256 |
| CLI dev dependency | 0.21 with defaults; unified with core in full CLI builds |
| isolated native proof | =0.21.0; https, ssh, unstable-sha256; sys =0.18.8 |

Core's explicit `libgit2-sys =0.18.8` declaration is a dev dependency. The
published production graph and each independent consumer must be checked, not
assumed pinned by that dev-only declaration. The current locks resolve 0.18.8.

Fork features include empty defaults, ssh, https, cred, unstable-sha256,
vendored-libgit2, vendored-openssl and zlib-ng-compat. Preserve existing consumer
feature intent. Native selection may use a suitable system library unless
qualification explicitly fixes vendoring; record the actual linked library.
SHA-256 changes native ABI and requires consistent feature selection.

Current CLI platform gate names five native targets: Windows x86_64 MSVC,
macOS aarch64 and x86_64, Linux GNU aarch64 and x86_64. Core release/matrix jobs
cover Windows and Linux x86_64 plus macOS/Linux aarch64. Activation must satisfy
the controlling release matrix, not merely this host's successful build.
Required object formats are SHA-1 and the existing experimental SHA-256 support.
Local toolchain qualification uses the existing Rust 1.95 fixtures. None of
these platform or format gates was run during registration.

## Compatibility obligations for the first lane checkpoints

| Lane | Preserve/characterize before replacement | Evidence boundary |
|---|---|---|
| Local fetch | Explicit wants/refspecs, unrelated non-commit refs on both sides, object closure, errors, reference effects and FETCH_HEAD | Stock minimal reproduction before selecting C fix or local transfer |
| Per-remote API | Accepted factory signature, owner/lifetime, error/panic, retained transport and callback coexistence | Reuse seven native tests and archive proof with exact source/native identity |
| Commit/tag | Existing option set and ambient identity/hooks/signing; staging/index and ref effects on failures; tag delete | Native Git 2.52.0 differential fixtures with isolated configuration and helpers |
| Filtered history | Exact ordered commits, current ranges/first-parent/pathspec semantics and read-only behavior | Existing native parity suite plus targeted/seeded cases with replay |

No new CLI options or expanded Git capability surface is in scope. The capability
inventory is background discussion, not implementation authority. Mutation tests
must compare failure aftermath as well as return values. Tests may create
fixtures with Git and run an oracle, but the replacement operation must also
run where direct or indirect Git execution is denied. User-configured external
programs remain subject to the accepted plan's explicit boundary.

Next: complete the lead P2 checkpoint with exact ownership, numeric first-package
ceilings and review gates. First packages may be characterization-only where
the design depends on evidence; they do not authorize the subsequent replacement
until its route/interface is settled. Update the controlling fallback/AD1 policy
before changing production behavior. Read `EVIDENCE.md` before experiments;
public regression fixtures stay public and campaign-only data stays private.
