mod branch_switch;
mod handle_branch;
mod handle_commit;
mod handle_create_repo;
mod handle_init_from_sources;
mod handle_list_snapshots;
mod handle_ls;
mod handle_materialize;
mod handle_repo_lifecycle;
mod handle_stage;
mod handle_stash;
mod handle_tag;
mod historical_identity;
mod materialize_preflight;
mod merge;
mod normalize_path;
mod pathspec_routing;
mod pull_head_barrier;
mod pull_head_member_preflight;
mod pull_head_merge_preflight;
mod pull_head_plan;
mod push_member;
mod stage_routing;
mod stage_workspace_git_metadata;
mod sync_workspace_boundary;
mod target_selection;
#[cfg(test)]
mod tests;
mod workspace_bootstrap;

pub(crate) use branch_switch::*;
pub use handle_branch::*;
pub use handle_commit::*;
pub use handle_create_repo::*;
pub use handle_init_from_sources::*;
pub use handle_list_snapshots::*;
pub use handle_ls::*;
pub use handle_materialize::*;
pub use handle_repo_lifecycle::*;
pub use handle_stage::*;
pub use handle_stash::*;
pub use handle_tag::*;
pub(crate) use historical_identity::*;
pub(crate) use materialize_preflight::*;
pub(crate) use merge::guarded_workspace_root;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use merge::{
    CanonicalMergeLocations, MAX_CHECKED_OWNER_RECORD_BYTES, acquire_canonical_merge_locations,
    archived_fixture_for_test, observe_checked_archive_source_v0_leaves_for_test,
    observe_checked_archive_source_v1, observe_checked_owner_v0, observe_checked_owner_v1,
    observe_checked_owner_v1_from_canonical, test_v1_record,
};
#[allow(unused_imports)]
pub(crate) use merge::{
    CheckedArchiveSourceObservation, CheckedOwnerObservationError, CheckedOwnerRecordObservation,
    CheckedOwnerRecordVersion, observe_checked_archive_source_v0,
    observe_checked_owner_v0_from_canonical,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use merge::{OperationState, ParticipantState, RecordVersion};
pub use merge::{
    WorkspaceMutationGuard, acquire_workspace_mutation_guard, enforce_workspace_open_merge_gate,
    handle_merge, handle_merge_with_events,
};
pub(crate) use normalize_path::*;
pub(crate) use pathspec_routing::*;
pub use pull_head_member_preflight::*;
pub use push_member::*;
pub(crate) use stage_routing::*;
pub(crate) use stage_workspace_git_metadata::*;
pub(crate) use sync_workspace_boundary::*;
pub(crate) use target_selection::*;
pub use workspace_bootstrap::*;
