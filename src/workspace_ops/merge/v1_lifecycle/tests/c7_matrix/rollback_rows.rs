use super::matrix_spec::*;

#[test]
fn rollback_non_root_checked_artifact_and_action_free_domains_are_closed() {
    assert_eq!(ROLLBACK_ROWS.len(), 12);
    assert_eq!(
        ROLLBACK_ROWS
            .iter()
            .enumerate()
            .filter(|(index, (row, _))| ROLLBACK_ROWS[..*index].iter().all(|(seen, _)| seen != row))
            .count(),
        ROLLBACK_ROWS.len()
    );
    assert_eq!(
        ROLLBACK_ROWS
            .iter()
            .filter(|(_, class)| *class == RowClass::Physical)
            .count(),
        9
    );
    assert_eq!(NON_ROOT_ROWS.len(), 6);
    assert_eq!(CHECKED_ARTIFACT_ROWS.len(), 7);
    assert_eq!(ACTION_FREE_POSITIONS.len(), 9);
    assert_eq!(
        ACTION_FREE_POSITIONS
            .iter()
            .enumerate()
            .filter(|(index, row)| !ACTION_FREE_POSITIONS[..*index].contains(row))
            .count(),
        ACTION_FREE_POSITIONS.len()
    );
    assert_eq!(
        CHECKED_ARTIFACT_ROWS[0],
        (CheckedArtifactRow::SourceEqualsGoal, RowClass::ProofOnly)
    );
}
