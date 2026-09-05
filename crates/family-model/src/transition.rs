//! Index transitions: pure validation that produces the next view.
//!
//! The store rereads the index under the family lock, validates a change
//! here, and writes `next`. Nothing here touches a file; refusals are typed
//! and name the row and the state that refused.

use crate::{AllocationId, FamilyView, MemberName, MemberRow, MemberState, ROOT_PATH};

/// One index change. Pointer and marker files are separate store session
/// operations; this enum only ever changes the root index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyChange {
    /// Reserve a name and path with a `creating` row.
    Allocate { name: MemberName, row: MemberRow },
    /// Record why a `creating` row stopped, keeping it incomplete.
    RecordError {
        name: MemberName,
        last_error: String,
    },
    /// `creating` -> `ready`, after the final manifest is installed.
    MarkReady {
        name: MemberName,
        expected_allocation: AllocationId,
    },
    /// `ready` -> `disposing`, before one-shot removal.
    MarkDisposing {
        name: MemberName,
        expected_allocation: AllocationId,
    },
    /// Remove a row: after deletion, for `--keep`, or for a stale row whose
    /// target is gone.
    RemoveRow {
        name: MemberName,
        reason: RemovalReason,
    },
    /// Remove every row; the store then removes the index. Repeatable.
    Disband,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalReason {
    /// The validated directory was removed.
    Disposed,
    /// `dispose --keep`: detach only; every file stays.
    Keep,
    /// Explicit dispose of a row whose target is absent.
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NameCollision {
        name: MemberName,
        holder_path: String,
    },
    PathCollision {
        path: String,
        holder: String,
    },
    /// The path is inside the root or another member (or contains one).
    NestedPath {
        path: String,
        other: String,
    },
    /// An allocation row that is not a valid reservation (wrong state,
    /// empty or absolute path).
    InvalidRow {
        name: MemberName,
        detail: String,
    },
    NotFound {
        name: MemberName,
    },
    WrongState {
        name: MemberName,
        expected: MemberState,
        actual: MemberState,
    },
    AllocationMismatch {
        name: MemberName,
        expected: AllocationId,
        actual: AllocationId,
    },
    /// `dispose --keep` and stale removal refuse a `disposing` row's
    /// directory contents only through explicit dispose; `RemoveRow` with
    /// `Disposed` requires the row to be `disposing`.
    NotDisposing {
        name: MemberName,
        actual: MemberState,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameCollision { name, holder_path } => {
                write!(f, "name `{name}` already holds {holder_path}")
            }
            Self::PathCollision { path, holder } => {
                write!(f, "path `{path}` is already a family member ({holder})")
            }
            Self::NestedPath { path, other } => {
                write!(
                    f,
                    "path `{path}` is nested with family member path `{other}`"
                )
            }
            Self::InvalidRow { name, detail } => write!(f, "row `{name}`: {detail}"),
            Self::NotFound { name } => write!(f, "no family member named `{name}`"),
            Self::WrongState {
                name,
                expected,
                actual,
            } => write!(
                f,
                "member `{name}` is {} (expected {})",
                actual.as_str(),
                expected.as_str()
            ),
            Self::AllocationMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "member `{name}` allocation {actual} does not match {expected}"
            ),
            Self::NotDisposing { name, actual } => {
                write!(
                    f,
                    "member `{name}` is {} and was not being disposed",
                    actual.as_str()
                )
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// A change the model accepted, with the index it produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedChange {
    pub change: FamilyChange,
    pub next: FamilyView,
}

/// Validate `change` against `view` and produce the next view.
pub fn validate_transition(
    view: &FamilyView,
    change: &FamilyChange,
) -> Result<ValidatedChange, Refusal> {
    let mut next = view.clone();
    match change {
        FamilyChange::Allocate { name, row } => {
            if row.state != MemberState::Creating {
                return Err(Refusal::InvalidRow {
                    name: name.clone(),
                    detail: format!("an allocation must be creating, not {}", row.state.as_str()),
                });
            }
            validate_path(name, &row.path)?;
            if let Some(holder) = view.members.get(name) {
                return Err(Refusal::NameCollision {
                    name: name.clone(),
                    holder_path: holder.path.clone(),
                });
            }
            for (other_name, other) in &view.members {
                if other.path == row.path {
                    return Err(Refusal::PathCollision {
                        path: row.path.clone(),
                        holder: other_name.as_str().to_owned(),
                    });
                }
                if nested(&row.path, &other.path) {
                    return Err(Refusal::NestedPath {
                        path: row.path.clone(),
                        other: other.path.clone(),
                    });
                }
            }
            next.members.insert(name.clone(), row.clone());
        }
        FamilyChange::RecordError { name, last_error } => {
            let row = require(&mut next, name)?;
            expect_state(name, row, MemberState::Creating)?;
            row.last_error = Some(last_error.clone());
        }
        FamilyChange::MarkReady {
            name,
            expected_allocation,
        } => {
            let row = require(&mut next, name)?;
            expect_state(name, row, MemberState::Creating)?;
            expect_allocation(name, row, expected_allocation)?;
            row.state = MemberState::Ready;
            row.last_error = None;
        }
        FamilyChange::MarkDisposing {
            name,
            expected_allocation,
        } => {
            let row = require(&mut next, name)?;
            expect_state(name, row, MemberState::Ready)?;
            expect_allocation(name, row, expected_allocation)?;
            row.state = MemberState::Disposing;
        }
        FamilyChange::RemoveRow { name, reason } => {
            let row = require(&mut next, name)?;
            if *reason == RemovalReason::Disposed && row.state != MemberState::Disposing {
                return Err(Refusal::NotDisposing {
                    name: name.clone(),
                    actual: row.state,
                });
            }
            next.members.remove(name);
        }
        FamilyChange::Disband => {
            next.members.clear();
        }
    }
    Ok(ValidatedChange {
        change: change.clone(),
        next,
    })
}

fn require<'a>(view: &'a mut FamilyView, name: &MemberName) -> Result<&'a mut MemberRow, Refusal> {
    view.members
        .get_mut(name)
        .ok_or_else(|| Refusal::NotFound { name: name.clone() })
}

fn expect_state(name: &MemberName, row: &MemberRow, expected: MemberState) -> Result<(), Refusal> {
    if row.state != expected {
        return Err(Refusal::WrongState {
            name: name.clone(),
            expected,
            actual: row.state,
        });
    }
    Ok(())
}

fn expect_allocation(
    name: &MemberName,
    row: &MemberRow,
    expected: &AllocationId,
) -> Result<(), Refusal> {
    if &row.allocation_id != expected {
        return Err(Refusal::AllocationMismatch {
            name: name.clone(),
            expected: expected.clone(),
            actual: row.allocation_id.clone(),
        });
    }
    Ok(())
}

/// A member path is root-relative, non-empty, not the root itself, not an
/// absolute path, and outside the root directory.
fn validate_path(name: &MemberName, path: &str) -> Result<(), Refusal> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty() || normalized == ROOT_PATH || normalized == "./" {
        return Err(Refusal::InvalidRow {
            name: name.clone(),
            detail: "a member path must not be empty or the root".to_owned(),
        });
    }
    if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
        return Err(Refusal::InvalidRow {
            name: name.clone(),
            detail: "a member path must be root-relative".to_owned(),
        });
    }
    if !(normalized == ".." || normalized.starts_with("../")) {
        return Err(Refusal::NestedPath {
            path: path.to_owned(),
            other: ROOT_PATH.to_owned(),
        });
    }
    Ok(())
}

fn nested(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('/');
    let b = b.trim_end_matches('/');
    a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{row, view};
    use crate::{CloneMode, MemberKind};

    fn name(value: &str) -> MemberName {
        MemberName::parse(value).unwrap()
    }

    #[test]
    fn allocate_reserves_a_creating_row_and_refuses_collisions() {
        let view = view();
        let ok = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("D"),
                row: row("../ws-D", MemberState::Creating),
            },
        )
        .unwrap();
        assert_eq!(ok.next.members.len(), 4);
        assert_eq!(ok.next.members[&name("D")].state, MemberState::Creating);
        assert_eq!(view.members.len(), 3, "the input view is untouched");

        let collision = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("A"),
                row: row("../elsewhere", MemberState::Creating),
            },
        )
        .unwrap_err();
        assert_eq!(
            collision,
            Refusal::NameCollision {
                name: name("A"),
                holder_path: "../ws-A".to_owned()
            }
        );
        let path = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("A2"),
                row: row("../ws-A", MemberState::Creating),
            },
        )
        .unwrap_err();
        assert_eq!(
            path,
            Refusal::PathCollision {
                path: "../ws-A".to_owned(),
                holder: "A".to_owned()
            }
        );
        let nested = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("A3"),
                row: row("../ws-A/inner", MemberState::Creating),
            },
        )
        .unwrap_err();
        assert!(matches!(nested, Refusal::NestedPath { .. }));
        let inside_root = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("In"),
                row: row("sub/dir", MemberState::Creating),
            },
        )
        .unwrap_err();
        assert_eq!(
            inside_root,
            Refusal::NestedPath {
                path: "sub/dir".to_owned(),
                other: ".".to_owned()
            }
        );
        let absolute = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("Abs"),
                row: row("/tmp/x", MemberState::Creating),
            },
        )
        .unwrap_err();
        assert!(matches!(absolute, Refusal::InvalidRow { .. }));
        let ready = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: name("R"),
                row: row("../ws-R", MemberState::Ready),
            },
        )
        .unwrap_err();
        assert!(
            matches!(ready, Refusal::InvalidRow { .. }),
            "an allocation is never ready"
        );
    }

    #[test]
    fn lifecycle_transitions_check_state_and_allocation() {
        let view = view();
        let alloc_b = view.members[&name("B")].allocation_id.clone();
        let ready = validate_transition(
            &view,
            &FamilyChange::MarkReady {
                name: name("B"),
                expected_allocation: alloc_b.clone(),
            },
        )
        .unwrap();
        assert_eq!(ready.next.members[&name("B")].state, MemberState::Ready);

        let wrong_alloc = validate_transition(
            &view,
            &FamilyChange::MarkReady {
                name: name("B"),
                expected_allocation: AllocationId::new("other").unwrap(),
            },
        )
        .unwrap_err();
        assert!(matches!(wrong_alloc, Refusal::AllocationMismatch { .. }));

        let already_ready = validate_transition(
            &view,
            &FamilyChange::MarkReady {
                name: name("A"),
                expected_allocation: view.members[&name("A")].allocation_id.clone(),
            },
        )
        .unwrap_err();
        assert_eq!(
            already_ready,
            Refusal::WrongState {
                name: name("A"),
                expected: MemberState::Creating,
                actual: MemberState::Ready
            }
        );

        let disposing = validate_transition(
            &view,
            &FamilyChange::MarkDisposing {
                name: name("A"),
                expected_allocation: view.members[&name("A")].allocation_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            disposing.next.members[&name("A")].state,
            MemberState::Disposing
        );
        let dispose_creating = validate_transition(
            &view,
            &FamilyChange::MarkDisposing {
                name: name("B"),
                expected_allocation: alloc_b,
            },
        )
        .unwrap_err();
        assert!(matches!(dispose_creating, Refusal::WrongState { .. }));

        let missing = validate_transition(
            &view,
            &FamilyChange::RecordError {
                name: name("Z"),
                last_error: "copy failed".to_owned(),
            },
        )
        .unwrap_err();
        assert_eq!(missing, Refusal::NotFound { name: name("Z") });
        let recorded = validate_transition(
            &view,
            &FamilyChange::RecordError {
                name: name("B"),
                last_error: "copy failed".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(
            recorded.next.members[&name("B")].last_error.as_deref(),
            Some("copy failed")
        );
        assert_eq!(
            recorded.next.members[&name("B")].state,
            MemberState::Creating
        );
    }

    #[test]
    fn removal_and_disband_are_explicit_and_repeatable() {
        let view = view();
        let keep = validate_transition(
            &view,
            &FamilyChange::RemoveRow {
                name: name("B"),
                reason: RemovalReason::Keep,
            },
        )
        .unwrap();
        assert!(
            !keep.next.members.contains_key(&name("B")),
            "keep detaches an incomplete row"
        );
        let disposed_not_disposing = validate_transition(
            &view,
            &FamilyChange::RemoveRow {
                name: name("A"),
                reason: RemovalReason::Disposed,
            },
        )
        .unwrap_err();
        assert_eq!(
            disposed_not_disposing,
            Refusal::NotDisposing {
                name: name("A"),
                actual: MemberState::Ready
            }
        );
        let disposed = validate_transition(
            &view,
            &FamilyChange::RemoveRow {
                name: name("C"),
                reason: RemovalReason::Disposed,
            },
        )
        .unwrap();
        assert!(!disposed.next.members.contains_key(&name("C")));
        let disband = validate_transition(&view, &FamilyChange::Disband).unwrap();
        assert!(disband.next.members.is_empty());
        let again = validate_transition(&disband.next, &FamilyChange::Disband).unwrap();
        assert!(
            again.next.members.is_empty(),
            "disband repeats without refusing"
        );
        assert_eq!(disband.next.family_id, view.family_id);
    }

    #[test]
    fn refusals_render_their_row_and_state() {
        let refusal = Refusal::WrongState {
            name: name("A"),
            expected: MemberState::Creating,
            actual: MemberState::Ready,
        };
        assert_eq!(
            refusal.to_string(),
            "member `A` is ready (expected creating)"
        );
        let row = MemberRow {
            path: "../x".to_owned(),
            kind: MemberKind::Bare,
            state: MemberState::Creating,
            allocation_id: AllocationId::new("a").unwrap(),
            source_path: "gwz-core".to_owned(),
            mode: CloneMode::Bare,
            last_error: None,
        };
        assert_eq!(row.mode, CloneMode::Bare);
    }
}
