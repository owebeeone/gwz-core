//! Single owner for the fixed checked-artifact catalog/private-domain names.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogPrivateRootV1 {
    Workspace,
    GitDirectory,
}

impl CatalogPrivateRootV1 {
    const fn prefix(self) -> &'static [u8] {
        match self {
            Self::Workspace => b".gwz",
            Self::GitDirectory => b"gwz",
        }
    }
}

/// R2-F R1.1, 2026-09-01 — one new name, two directories
/// (`GwzM5-8R2F-RelocationPlan.md` §1, ADOPTED).
///
/// `Final` served the catalog's Final directory **and** the legacy leaf
/// writer's private area through `policy.rs::private_parent`. One name, two
/// consumers, one directory — that shared identity is the coexistence bug.
/// The split gives each consumer its own name: `Final` moves to
/// `catalog-final`; `LegacyPrivate` keeps `checked-artifacts`, so the legacy
/// writer's live area is unmoved and nothing is orphaned.
///
/// **Ordering ground.** Both names are in `ALL`, so the member set moves
/// `PrivateControlDomain::version_digest()` (`capability/collision.rs:261`) →
/// `historical_collision_digest`, and `Final`'s bytes move `final_name`. Both
/// are persisted CBOR fields of `CheckedCatalogBootstrapV1` (keys 4 and 8), and
/// the digest is also an input to the on-disk scratch directory name
/// (`capability/pre_catalog.rs:331-335`). All three movements are FREE strictly
/// before the first catalog activation, and only there: `recover_or_create` has
/// zero production callers today (plan §1; R1.2 makes that a tripwire), so no
/// durable record exists to invalidate. After activation this is durable
/// surgery.
///
/// **Legacy stays inside `ALL` deliberately** (plan §1, re-charter [RC-P1-1]):
/// `ALL`'s only consumers are the collision domain and a test pin, so keeping
/// `checked-artifacts` a member preserves today's `reject_private_collisions`
/// guard over a directory that is still live until DR-1 decides its retirement —
/// E4.7's legacy in-place-writer clause RE-OWNS there (2026-09-02,
/// `GwzM5-8R2E-CapabilityFreeAmendment.md` §4) — the actual content of
/// "zero behavioral change for legacy". De-recognition of resident
/// `checked-artifacts/` directories does not come from `ALL` at all: the
/// recognition table is `catalog/enumeration.rs::fixed_roles`' fixed
/// variant-keyed array, so it comes solely from `Final.leaf_bytes()` changing.
///
/// **The ASCII constraint is load-bearing at three enforcement sites**, all
/// `expect` (re-charter [RC-P3-2]): `relative_path` below (`:59`, evaluated on
/// the live git-directory merge-preservation preflight via
/// `observation.rs:93`), `provider/directory_mutation.rs:721` and
/// `provider/completed.rs:561`. A future leaf must satisfy all three.
///
/// **The scratch stem is deliberately unchanged.** `catalog/scratch.rs:6`'s
/// `checked-artifacts-catalog-bootstrap-v1.scratch.` brands the name FAMILY,
/// not this directory, and is compared only against itself; the cosmetic
/// family split is accepted rather than widening the diff (plan §1, [R2-P3-3]).
///
/// Adding a variant breaks no exhaustive match: the only exhaustive matches are
/// `leaf_bytes`/`relative_path` here, and `provider/interior.rs:170-186`'s
/// `directory_fact` already has a `_ => Other` arm — checked, not missed
/// ([RC-P3-4]); `LegacyPrivate` is never passed to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogPrivateNameV1 {
    Final,
    LegacyPrivate,
    BootstrapScratch,
    BootstrapActive,
    BootstrapStaging,
}

impl CatalogPrivateNameV1 {
    pub(in crate::checked_artifact) const ALL: &'static [Self] = &[
        Self::Final,
        Self::LegacyPrivate,
        Self::BootstrapScratch,
        Self::BootstrapActive,
        Self::BootstrapStaging,
    ];

    pub(in crate::checked_artifact) const fn leaf_bytes(self) -> &'static [u8] {
        match self {
            Self::Final => b"catalog-final",
            Self::LegacyPrivate => b"checked-artifacts",
            Self::BootstrapScratch => b"checked-artifacts-catalog-bootstrap-v1.scratch",
            Self::BootstrapActive => b"checked-artifacts-catalog-bootstrap-v1.active",
            Self::BootstrapStaging => b"checked-artifacts-catalog-bootstrap-v1.staging",
        }
    }

    pub(in crate::checked_artifact) fn relative_bytes(self, root: CatalogPrivateRootV1) -> Vec<u8> {
        let mut value = Vec::with_capacity(root.prefix().len() + 1 + self.leaf_bytes().len());
        value.extend_from_slice(root.prefix());
        value.push(b'/');
        value.extend_from_slice(self.leaf_bytes());
        value
    }

    pub(in crate::checked_artifact) fn relative_path(self, root: CatalogPrivateRootV1) -> PathBuf {
        let prefix = match root {
            CatalogPrivateRootV1::Workspace => ".gwz",
            CatalogPrivateRootV1::GitDirectory => "gwz",
        };
        PathBuf::from(prefix).join(
            std::str::from_utf8(self.leaf_bytes()).expect("fixed catalog name is valid ASCII"),
        )
    }
}
