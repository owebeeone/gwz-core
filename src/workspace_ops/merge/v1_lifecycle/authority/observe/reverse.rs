mod entry;
mod preservation;
mod preserving_recovery;
mod rollback;
mod rolling_back_recovery;

pub(in crate::workspace_ops::merge::v1_lifecycle) use entry::{
    prepare_direct_rollback_entry, prepare_exhausted_rollback_entry, prepare_preservation_entry,
};
pub(in crate::workspace_ops::merge::v1_lifecycle) use preservation::durability_fact as preservation_durability_fact;
pub(in crate::workspace_ops::merge::v1_lifecycle::authority) use preservation::observe as observe_preservation;
pub(in crate::workspace_ops::merge::v1_lifecycle) use preservation::{
    execution_prefix_is_exact as preservation_execution_prefix_is_exact,
    reset_step as preservation_reset_step, stash_guard as preservation_stash_guard,
    stash_step as preservation_stash_step,
};
pub(in crate::workspace_ops::merge::v1_lifecycle) use preserving_recovery::verify_recovery_origin as preserving_verify_recovery_origin;
pub(in crate::workspace_ops::merge::v1_lifecycle::authority) use rollback::observe as observe_rollback;
pub(in crate::workspace_ops::merge::v1_lifecycle) use rolling_back_recovery::verify_recovery_origin as rolling_back_verify_recovery_origin;
