use sha2::{Digest, Sha256};

use super::super::*;
use super::{
    RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogInteriorFactV1,
    RawCatalogInteriorObservationV1, RawCatalogRoleRowV1,
};
use crate::checked_artifact::capability::{
    LosslessIndexEntry, PathComponentMode, PrivateControlDomain, TrackedWorktreeEntry,
};

pub(super) struct CollisionModes {
    pub(super) root: PathComponentMode,
    pub(super) private_parent: Option<PathComponentMode>,
}

pub(super) struct IndexSnapshotFacts {
    pub(super) file_identity: Option<Vec<u8>>,
    pub(super) file_durable_identity: Option<Vec<u8>>,
    pub(super) content_digest: Option<[u8; 32]>,
    pub(super) entries: Vec<LosslessIndexEntry>,
    pub(super) worktree: Vec<TrackedWorktreeEntry>,
}

pub(super) struct SnapshotParts<'a> {
    pub(super) root_kind: PreCatalogRootKindV1,
    pub(super) domain: &'a PrivateControlDomain,
    pub(super) root_identity: &'a [u8],
    pub(super) repository_identity: &'a [u8],
    pub(super) common_directory_identity: &'a [u8],
    pub(super) private_parent_fact: Option<&'a [u8]>,
    pub(super) path_profile: &'a CanonicalPathIdentityV1,
    pub(super) index: Option<&'a IndexSnapshotFacts>,
    pub(super) namespace: &'a [RawCatalogRoleRowV1],
    pub(super) namespace_entry_count: usize,
    pub(super) namespace_encoded_name_bytes: usize,
}

pub(super) fn reject_private_collisions(
    entries: &[LosslessIndexEntry],
    domain: &PrivateControlDomain,
    modes: CollisionModes,
) -> Result<(), CheckedFsError> {
    for entry in entries {
        for owned in domain.members() {
            if paths_collide(entry.path().as_bytes(), owned.as_bytes(), &modes) {
                return Err(CheckedFsError::ambiguous(
                    "private namespace collision",
                    format!(
                        "Git index path {} overlaps reserved path {}",
                        render_bytes(entry.path().as_bytes()),
                        render_bytes(owned.as_bytes())
                    ),
                ));
            }
        }
        if scratch_family_collides(
            entry.path().as_bytes(),
            domain.scratch_family().as_bytes(),
            &modes,
        ) {
            return Err(CheckedFsError::ambiguous(
                "private namespace collision",
                format!(
                    "Git index path {} overlaps reserved dynamic scratch family {}.<attempt>",
                    render_bytes(entry.path().as_bytes()),
                    render_bytes(domain.scratch_family().as_bytes())
                ),
            ));
        }
    }
    Ok(())
}

fn scratch_family_collides(candidate: &[u8], family: &[u8], modes: &CollisionModes) -> bool {
    let candidate = candidate.split(|byte| *byte == b'/').collect::<Vec<_>>();
    let family = family.split(|byte| *byte == b'/').collect::<Vec<_>>();
    if candidate.len() < 2 || family.len() != 2 {
        return false;
    }
    if !component_equivalent(candidate[0], family[0], modes.root) {
        return false;
    }
    let Some(mode) = modes.private_parent else {
        return false;
    };
    component_has_dot_suffix(candidate[1], family[1], mode)
}

fn component_has_dot_suffix(candidate: &[u8], family: &[u8], mode: PathComponentMode) -> bool {
    if candidate.len() <= family.len() || candidate.get(family.len()) != Some(&b'.') {
        return false;
    }
    component_equivalent(&candidate[..family.len()], family, mode)
}

pub(super) fn digest(parts: SnapshotParts<'_>) -> [u8; 32] {
    let mut digest = Sha256::new();
    frame(&mut digest, b"gwz-pre-catalog-snapshot-v1\0");
    frame(
        &mut digest,
        &[match parts.root_kind {
            PreCatalogRootKindV1::Workspace => 0,
            PreCatalogRootKindV1::GitDirectory => 1,
        }],
    );
    frame(&mut digest, &parts.domain.version_digest());
    frame(&mut digest, parts.root_identity);
    frame(&mut digest, parts.repository_identity);
    frame(&mut digest, parts.common_directory_identity);
    frame_optional(&mut digest, parts.private_parent_fact);
    frame(&mut digest, &parts.path_profile.fresh_digest_material());

    match parts.index {
        None => frame(&mut digest, &[0]),
        Some(index) => {
            frame(&mut digest, &[1]);
            frame_optional(&mut digest, index.file_identity.as_deref());
            frame_optional(
                &mut digest,
                index.content_digest.as_ref().map(<[u8; 32]>::as_slice),
            );
            frame(&mut digest, &(index.entries.len() as u64).to_be_bytes());
            for entry in &index.entries {
                frame(&mut digest, entry.path().as_bytes());
                frame(&mut digest, &[entry.stage().code()]);
                frame(&mut digest, &entry.mode().to_be_bytes());
                frame(&mut digest, &entry.raw_flags().to_be_bytes());
                frame(&mut digest, &entry.raw_extended_flags().to_be_bytes());
                frame(
                    &mut digest,
                    &entry.metadata().ctime().seconds().to_be_bytes(),
                );
                frame(
                    &mut digest,
                    &entry.metadata().ctime().nanoseconds().to_be_bytes(),
                );
                frame(
                    &mut digest,
                    &entry.metadata().mtime().seconds().to_be_bytes(),
                );
                frame(
                    &mut digest,
                    &entry.metadata().mtime().nanoseconds().to_be_bytes(),
                );
                for value in entry.metadata().stat() {
                    frame(&mut digest, &value.to_be_bytes());
                }
                frame(&mut digest, entry.metadata().object_id());
            }

            frame(&mut digest, &(index.worktree.len() as u64).to_be_bytes());
            for entry in &index.worktree {
                frame(&mut digest, entry.path().as_bytes());
                frame(&mut digest, &[entry.kind().code()]);
            }
        }
    }

    frame(
        &mut digest,
        &(parts.namespace_entry_count as u64).to_be_bytes(),
    );
    frame(
        &mut digest,
        &(parts.namespace_encoded_name_bytes as u64).to_be_bytes(),
    );
    frame(&mut digest, &(parts.namespace.len() as u64).to_be_bytes());
    debug_assert!(
        parts
            .namespace
            .windows(2)
            .all(|rows| rows[0].path < rows[1].path)
    );
    for row in parts.namespace {
        frame(&mut digest, &row.path);
        frame_catalog_fact(&mut digest, &row.fact);
    }
    digest.finalize().into()
}

fn frame_catalog_fact(digest: &mut Sha256, fact: &RawCatalogEntryFactV1) {
    match fact {
        RawCatalogEntryFactV1::Directory {
            identity, interior, ..
        } => {
            frame(digest, &[1]);
            frame(digest, identity);
            frame_catalog_interior(digest, interior);
        }
        RawCatalogEntryFactV1::RegularFile { identity, bytes } => {
            frame(digest, &[2]);
            frame(digest, identity);
            frame_catalog_bytes(digest, bytes);
        }
        RawCatalogEntryFactV1::Other(value) => {
            frame(digest, &[3]);
            frame(digest, value);
        }
    }
}

fn frame_catalog_interior(digest: &mut Sha256, interior: &RawCatalogInteriorObservationV1) {
    frame(digest, &(interior.entry_count as u64).to_be_bytes());
    frame(digest, &(interior.encoded_name_bytes as u64).to_be_bytes());
    frame(digest, &(interior.rows.len() as u64).to_be_bytes());
    for row in &interior.rows {
        frame(digest, row.slot.name().as_bytes());
        match &row.fact {
            RawCatalogInteriorFactV1::EmptyDirectory { identity, .. } => {
                frame(digest, &[1]);
                frame(digest, identity);
            }
            // T1 widening: a populated retired root frames under its own tag
            // and carries its bounded counts, so the fresh-observation digest
            // moves when a terminal retirement lands and moves again when the
            // retired root's shape changes. An empty retired root still frames
            // as tag 1 with its identity alone, so no already-recorded digest
            // is disturbed by the widening.
            RawCatalogInteriorFactV1::RetiredActionRoot {
                identity,
                infrastructure_rows,
                retired_action_dirs,
                ..
            } => {
                frame(digest, &[4]);
                frame(digest, identity);
                frame(digest, &(*infrastructure_rows as u64).to_be_bytes());
                frame(digest, &(*retired_action_dirs as u64).to_be_bytes());
            }
            RawCatalogInteriorFactV1::RegularFile {
                identity, bytes, ..
            } => {
                frame(digest, &[2]);
                frame(digest, identity);
                frame_catalog_bytes(digest, bytes);
            }
            RawCatalogInteriorFactV1::Other(value) => {
                frame(digest, &[3]);
                frame(digest, value);
            }
        }
    }
    // C-3 widening: the catalog root's active-action rows join the fresh
    // observation digest, so a same-length substitution of one action row for
    // another is caught by `revalidate_ready_observation` exactly as an
    // infrastructure-row substitution already is.
    frame(digest, &(interior.action_rows.len() as u64).to_be_bytes());
    for action in &interior.action_rows {
        frame(digest, &action.bytes());
    }
}

fn frame_catalog_bytes(digest: &mut Sha256, bytes: &RawCatalogBytesV1) {
    match bytes {
        RawCatalogBytesV1::Bounded(value) => {
            frame(digest, &[1]);
            frame(digest, value);
        }
        RawCatalogBytesV1::Oversize => frame(digest, &[2]),
    }
}

fn paths_collide(candidate: &[u8], owned: &[u8], modes: &CollisionModes) -> bool {
    if path_prefix(candidate, owned) || path_prefix(owned, candidate) {
        return true;
    }
    let candidate = candidate.split(|byte| *byte == b'/').collect::<Vec<_>>();
    let owned = owned.split(|byte| *byte == b'/').collect::<Vec<_>>();
    let common = candidate.len().min(owned.len());
    if common == 0 {
        return false;
    }
    for index in 0..common {
        let mode = match index {
            0 => Some(modes.root),
            1 => modes.private_parent,
            _ => Some(PathComponentMode::Sensitive),
        };
        let Some(mode) = mode else {
            return false;
        };
        if !component_equivalent(candidate[index], owned[index], mode) {
            return false;
        }
    }
    true
}

fn path_prefix(prefix: &[u8], value: &[u8]) -> bool {
    value == prefix
        || (value.starts_with(prefix)
            && value
                .get(prefix.len())
                .is_some_and(|separator| *separator == b'/'))
}

fn component_equivalent(left: &[u8], right: &[u8], mode: PathComponentMode) -> bool {
    match mode {
        PathComponentMode::Sensitive => left == right,
        PathComponentMode::AsciiCaseFold => {
            left.is_ascii()
                && right.is_ascii()
                && left
                    .iter()
                    .map(u8::to_ascii_lowercase)
                    .eq(right.iter().map(u8::to_ascii_lowercase))
        }
    }
}

fn frame(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn frame_optional(digest: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            frame(digest, &[1]);
            frame(digest, value);
        }
        None => frame(digest, &[0]),
    }
}

fn render_bytes(value: &[u8]) -> String {
    value
        .iter()
        .map(|byte| match byte {
            b' '..=b'~' => (*byte as char).to_string(),
            _ => format!("\\x{byte:02x}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checked_artifact::capability::{
        GitPathBytes, IndexTimestampV1, LosslessIndexMetadataV1,
    };

    fn entry(path: &[u8]) -> LosslessIndexEntry {
        LosslessIndexEntry::new(
            GitPathBytes::new(path.to_vec()).unwrap(),
            0,
            0o100644,
            0,
            0,
            LosslessIndexMetadataV1::new(
                IndexTimestampV1::new(0, 0).unwrap(),
                IndexTimestampV1::new(0, 0).unwrap(),
                [0; 5],
                vec![1; 20],
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn collision_relation_covers_exact_prefix_and_ascii_equivalence() {
        let domain = PrivateControlDomain::for_root(
            crate::checked_artifact::catalog_names::CatalogPrivateRootV1::Workspace,
        );
        for value in [
            b".gwz".as_slice(),
            b".gwz/checked-artifacts".as_slice(),
            b".gwz/checked-artifacts/child".as_slice(),
        ] {
            assert!(
                reject_private_collisions(
                    &[entry(value)],
                    &domain,
                    CollisionModes {
                        root: PathComponentMode::Sensitive,
                        private_parent: Some(PathComponentMode::Sensitive),
                    }
                )
                .is_err()
            );
        }
        assert!(
            reject_private_collisions(
                &[entry(b".GWZ/CHECKED-ARTIFACTS")],
                &domain,
                CollisionModes {
                    root: PathComponentMode::AsciiCaseFold,
                    private_parent: Some(PathComponentMode::AsciiCaseFold),
                }
            )
            .is_err()
        );
        assert!(
            reject_private_collisions(
                &[entry(b"src/lib.rs")],
                &domain,
                CollisionModes {
                    root: PathComponentMode::AsciiCaseFold,
                    private_parent: Some(PathComponentMode::AsciiCaseFold),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn collision_relation_reserves_the_complete_dynamic_scratch_family() {
        let domain = PrivateControlDomain::for_root(
            crate::checked_artifact::catalog_names::CatalogPrivateRootV1::Workspace,
        );
        let suffix = b".0000000000000000000000000000000000000000000000000000000000000000.\
0000000000000000000000000000000000000000000000000000000000000000.\
0101010101010101010101010101010101010101010101010101010101010101";
        let mut canonical = b".gwz/checked-artifacts-catalog-bootstrap-v1.scratch".to_vec();
        canonical.extend_from_slice(suffix);
        let mut malformed = b".gwz/checked-artifacts-catalog-bootstrap-v1.scratch".to_vec();
        malformed.extend_from_slice(b".malformed");

        for path in [canonical.as_slice(), malformed.as_slice()] {
            assert!(
                reject_private_collisions(
                    &[entry(path)],
                    &domain,
                    CollisionModes {
                        root: PathComponentMode::Sensitive,
                        private_parent: Some(PathComponentMode::Sensitive),
                    },
                )
                .is_err(),
                "dynamic scratch-family path must be reserved: {path:?}"
            );
        }
        assert!(
            reject_private_collisions(
                &[entry(
                    b".GWZ/CHECKED-ARTIFACTS-CATALOG-BOOTSTRAP-V1.SCRATCH.alias"
                )],
                &domain,
                CollisionModes {
                    root: PathComponentMode::AsciiCaseFold,
                    private_parent: Some(PathComponentMode::AsciiCaseFold),
                },
            )
            .is_err()
        );
    }
}
