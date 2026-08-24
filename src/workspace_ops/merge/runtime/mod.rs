mod dispatch;
mod mutation_guard;
mod open_gate;
#[cfg(test)]
mod tests;

pub(crate) use dispatch::MergeDependencies;
pub(in crate::workspace_ops::merge) use dispatch::V1Router;
#[cfg(test)]
pub(crate) use dispatch::handle_merge_with_dependencies;
pub use dispatch::{handle_merge, handle_merge_with_events};
pub(crate) use mutation_guard::guarded_workspace_root;
pub use mutation_guard::{WorkspaceMutationGuard, acquire_workspace_mutation_guard};
pub(crate) use open_gate::enforce_open_merge_stage_targets;
pub use open_gate::enforce_workspace_open_merge_gate;
