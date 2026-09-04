use super::super::MergeParticipantRecord;

/// Whether any participant's outcome changed, so the workspace root must
/// publish evidence.
///
/// **M5d.** Taken over the participants map rather than a record, because the
/// v0 ARCHIVE projection asks the same question of a `done/` record this
/// binary can read but not run (charter §5).
pub(in crate::workspace_ops::merge) fn publication_required(
    participants: &std::collections::BTreeMap<String, MergeParticipantRecord>,
) -> bool {
    participants
        .values()
        .any(super::super::participant_semantics::result::has_changed_result)
}
