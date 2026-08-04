use std::collections::BTreeSet;

use super::super::super::{MergeOperationRecord, PreservationEvidence};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArchivedCleanupWorklist {
    pub(crate) backup_refs: Vec<ArchivedBackupRefOwner>,
    pub(crate) has_stash_evidence: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ArchivedBackupRefOwner {
    pub(crate) target_id: String,
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) target_commit: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CleanupError {
    ContradictoryEvidence,
    NonCanonicalRef,
    DuplicateOwner,
}

pub(super) fn from_v0(
    record: &MergeOperationRecord,
) -> Result<ArchivedCleanupWorklist, CleanupError> {
    let mut owners = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut has_stash_evidence = false;
    for (target_id, participant) in &record.participants {
        collect_owner(
            record,
            target_id,
            &participant.path,
            owner_key(target_id),
            &participant.preservation,
            &mut owners,
            &mut seen,
            &mut has_stash_evidence,
        )?;
    }
    if let Some(publication) = record.publication.as_ref() {
        collect_owner(
            record,
            "@root",
            ".",
            "root",
            &publication.root_preservation,
            &mut owners,
            &mut seen,
            &mut has_stash_evidence,
        )?;
    }
    Ok(ArchivedCleanupWorklist {
        backup_refs: owners.into_iter().collect(),
        has_stash_evidence,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_owner(
    record: &MergeOperationRecord,
    target_id: &str,
    path: &str,
    key: &str,
    rows: &[PreservationEvidence],
    owners: &mut BTreeSet<ArchivedBackupRefOwner>,
    seen: &mut BTreeSet<(String, String)>,
    has_stash_evidence: &mut bool,
) -> Result<(), CleanupError> {
    if rows.len() > 1 {
        return Err(CleanupError::DuplicateOwner);
    }
    for row in rows {
        if row.backup_ref.is_some() != row.backup_commit.is_some()
            || row.stash_id.is_some() != row.stash_object_id.is_some()
            || row.backup_ref.is_none() && row.stash_id.is_none()
        {
            return Err(CleanupError::ContradictoryEvidence);
        }
        if let (Some(stash_id), Some(stash_object_id)) = (&row.stash_id, &row.stash_object_id)
            && (stash_id != &format!("stash_{}", record.merge_id) || !is_oid(stash_object_id))
        {
            return Err(CleanupError::ContradictoryEvidence);
        }
        *has_stash_evidence |= row.stash_id.is_some();
        let (Some(name), Some(target_commit)) = (&row.backup_ref, &row.backup_commit) else {
            continue;
        };
        if name != &format!("refs/gwz/merge/{}/{key}/head", record.merge_id) {
            return Err(CleanupError::NonCanonicalRef);
        }
        if !is_oid(target_commit) {
            return Err(CleanupError::ContradictoryEvidence);
        }
        if !seen.insert((path.to_owned(), name.clone())) {
            return Err(CleanupError::DuplicateOwner);
        }
        if !owners.insert(ArchivedBackupRefOwner {
            target_id: target_id.to_owned(),
            path: path.to_owned(),
            name: name.clone(),
            target_commit: target_commit.clone(),
        }) {
            return Err(CleanupError::DuplicateOwner);
        }
    }
    Ok(())
}

fn owner_key(target_id: &str) -> &str {
    if target_id == "@root" {
        "root"
    } else {
        target_id
    }
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
