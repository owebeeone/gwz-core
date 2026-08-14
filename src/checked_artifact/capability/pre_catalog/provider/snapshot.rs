use sha2::{Digest, Sha256};

use super::super::*;
use crate::checked_artifact::capability::{
    LosslessIndexEntry, PathComponentMode, PrivateControlDomain, TrackedWorktreeEntry,
};

pub(super) struct CollisionModes {
    pub(super) root: PathComponentMode,
    pub(super) private_parent: Option<PathComponentMode>,
}

pub(super) struct IndexSnapshotFacts {
    pub(super) file_identity: Option<Vec<u8>>,
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
    pub(super) namespace: &'a [(Vec<u8>, Vec<u8>)],
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
    }
    Ok(())
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

    let mut namespace = parts.namespace.iter().collect::<Vec<_>>();
    namespace.sort_by(|left, right| left.0.cmp(&right.0));
    frame(&mut digest, &(namespace.len() as u64).to_be_bytes());
    for (path, fact) in namespace {
        frame(&mut digest, path);
        frame(&mut digest, fact);
    }
    digest.finalize().into()
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
}
