use std::collections::BTreeSet;

use super::super::super::model::v1::MergeOperationRecordV1;
use super::super::super::{MergeOperationRecord, PreservationEvidence};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArchivedCleanupWorklist {
    pub(super) backup_refs: Vec<ArchivedBackupRefOwner>,
    pub(super) has_stash_evidence: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ArchivedBackupRefOwner {
    pub(super) target_id: String,
    pub(super) path: String,
    pub(super) name: String,
    pub(super) target_commit: String,
}

impl ArchivedCleanupWorklist {
    #[allow(
        dead_code,
        reason = "P4 consumes cleanup behind the disabled lifecycle"
    )]
    pub(crate) fn backup_refs(&self) -> &[ArchivedBackupRefOwner] {
        &self.backup_refs
    }

    #[allow(
        dead_code,
        reason = "P4 consumes cleanup behind the disabled lifecycle"
    )]
    pub(crate) fn has_stash_evidence(&self) -> bool {
        self.has_stash_evidence
    }
}

#[allow(dead_code, reason = "P4 consumes the read-only cleanup owner view")]
impl ArchivedBackupRefOwner {
    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn target_commit(&self) -> &str {
        &self.target_commit
    }
}

/// Whether the record being derived knows the durable preservation-cursor
/// markers of `GwzM5-8DurableCursorAmendment.md` §2.1.
///
/// The amendment's §2.2/§5 terminal-plane text asserts the marker-aware arm is
/// "v0-inert: no v0 record carries markers, so the `from_v0` leg of the shared
/// derivation never sees the new arm". The shared row struct parses the two
/// names for v0 records too, so that inertness has to be enforced here rather
/// than assumed: v0 knows no markers, and a fabricated marker must never
/// legitimize an otherwise-empty v0 row (§2.3 — "the value is never adopted").
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CleanupMarkers {
    /// v0: markers are not row content. A marker-only row stays an empty row.
    Unknown,
    /// v1: markers are legitimate row content per §5.
    #[allow(
        dead_code,
        reason = "constructed only by the test-gated from_v1 leg until A1 enables v1"
    )]
    Known,
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
        let key = if participant.target_kind == super::super::super::MergeTargetKind::Root {
            "root"
        } else {
            target_id
        };
        collect_owner(
            &record.merge_id,
            target_id,
            &participant.path,
            key,
            &participant.preservation,
            CleanupMarkers::Unknown,
            &mut owners,
            &mut seen,
            &mut has_stash_evidence,
        )?;
    }
    if let Some(publication) = record.publication.as_ref() {
        collect_owner(
            &record.merge_id,
            "@root",
            ".",
            "root",
            &publication.root_preservation,
            CleanupMarkers::Unknown,
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

pub(super) fn from_v1(
    record: &MergeOperationRecordV1,
) -> Result<ArchivedCleanupWorklist, CleanupError> {
    let mut owners = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut has_stash_evidence = false;
    for (target_id, participant) in &record.participants {
        let key = if participant.target_kind == super::super::super::MergeTargetKind::Root {
            "root"
        } else {
            target_id
        };
        collect_owner(
            &record.merge_id,
            target_id,
            &participant.path,
            key,
            &participant.preservation,
            CleanupMarkers::Known,
            &mut owners,
            &mut seen,
            &mut has_stash_evidence,
        )?;
    }
    if let Some(publication) = record.publication.as_ref() {
        collect_owner(
            &record.merge_id,
            "@root",
            ".",
            "root",
            &publication.root_preservation,
            CleanupMarkers::Known,
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
    merge_id: &str,
    target_id: &str,
    path: &str,
    key: &str,
    rows: &[PreservationEvidence],
    markers: CleanupMarkers,
    owners: &mut BTreeSet<ArchivedBackupRefOwner>,
    seen: &mut BTreeSet<(String, String)>,
    has_stash_evidence: &mut bool,
) -> Result<(), CleanupError> {
    if rows.len() > 1 {
        return Err(CleanupError::DuplicateOwner);
    }
    for row in rows {
        // `GwzM5-8DurableCursorAmendment.md` §5: on the v1 leg, marker-only
        // rows (`N`, `N+R`) are a legitimate archived shape and must not error
        // — otherwise one fully-noop owner would fail every archived merge's
        // worklist derivation, blocking all targeted cleanup. Markers
        // contribute no worklist entries and never block backup-ref deletion.
        //
        // On the v0 leg the arm is forked off, keeping the amendment's
        // "v0-inert" assertion true of the acceptance surface and not merely of
        // legitimate writers: v0 knows no markers, so a fabricated marker never
        // legitimizes an otherwise-empty v0 row. That row stays
        // `ContradictoryEvidence` and retention refuses fail-closed, exactly as
        // before this amendment.
        let marker_content = matches!(markers, CleanupMarkers::Known)
            && (row.noop_commit.is_some() || row.reset_commit.is_some());
        if row.backup_ref.is_some() != row.backup_commit.is_some()
            || row.stash_id.is_some() != row.stash_object_id.is_some()
            || row.backup_ref.is_none() && row.stash_id.is_none() && !marker_content
        {
            return Err(CleanupError::ContradictoryEvidence);
        }
        if let (Some(stash_id), Some(stash_object_id)) = (&row.stash_id, &row.stash_object_id)
            && (stash_id != &format!("stash_{merge_id}") || !is_oid(stash_object_id))
        {
            return Err(CleanupError::ContradictoryEvidence);
        }
        *has_stash_evidence |= row.stash_id.is_some();
        let (Some(name), Some(target_commit)) = (&row.backup_ref, &row.backup_commit) else {
            continue;
        };
        if name != &format!("refs/gwz/merge/{merge_id}/{key}/head") {
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

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
