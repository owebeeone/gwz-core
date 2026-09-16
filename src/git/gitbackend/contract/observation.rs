//! Small observation and request shapes exchanged with the Git backend.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitDirectRefObservation {
    Absent,
    Direct { target: String },
    NonDirect,
}
/// Exact paths whose live facts are proved by another aggregate observer.
/// Worktree and index ownership are intentionally separate domains.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitCheckoutOverlay {
    pub worktree_paths: Vec<String>,
    pub index_paths: Vec<String>,
}
