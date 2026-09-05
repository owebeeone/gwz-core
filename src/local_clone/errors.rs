//! Library error -> `ModelError` translation for the local clone family.
//!
//! Public contracts carry owned typed errors; core maps them onto the
//! existing `GwzErrorCode` registry at this boundary. No new code is
//! allocated at LCM1.0c (see `docs/ErrorCatalog.md`, "Local Clone Family").

use gwz_family_model::{MemberState, Refusal};
use gwz_family_store_contract::StoreError;

use crate::model::{ErrorCode, ModelError};

/// The family-only merge miss (design §6/§7; `GwzErrorCode.unknown_local`,
/// operator ruling 2026-09-05): `gwz merge --remote <token>` named no ready
/// family member. `state` is the recorded lifecycle state of a row that
/// exists but is not ready; `None` means there is no row at all (an absent
/// or reserved name such as `origin`). The state detail travels in the
/// message. Merge never falls back to a Git remote, so this is the whole
/// answer; pull/push keep `missing_remote` for their "neither" case.
pub(crate) fn unknown_local(token: &str, state: Option<MemberState>) -> ModelError {
    let detail = match state {
        Some(state) => format!(
            "`{token}` is a family member whose row is {}, not ready; only a ready \
             member is a merge source",
            state.as_str()
        ),
        None => format!(
            "no ready family member is named `{token}`; `merge --remote` resolves \
             family names only and never falls back to a Git remote"
        ),
    };
    ModelError::new(
        ErrorCode::UnknownLocal,
        format!("local family merge: {detail}"),
    )
}

/// A local-family operation or mode that this build does not implement.
pub(crate) fn unsupported(what: &str) -> ModelError {
    ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("{what} is not supported by this gwz-core build"),
    )
}

pub(crate) fn invalid(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, detail)
}

/// Map a pure-model refusal: collisions and nesting are path collisions,
/// missing rows are missing members, everything else is a request error.
pub(crate) fn refusal(refusal: &Refusal) -> ModelError {
    let code = match refusal {
        Refusal::NameCollision { .. }
        | Refusal::PathCollision { .. }
        | Refusal::NestedPath { .. } => ErrorCode::PathCollision,
        Refusal::NotFound { .. } => ErrorCode::MemberNotFound,
        Refusal::InvalidRow { .. }
        | Refusal::WrongState { .. }
        | Refusal::AllocationMismatch { .. }
        | Refusal::NotDisposing { .. } => ErrorCode::InvalidRequest,
    };
    ModelError::new(code, refusal.to_string())
}

/// [`store`] with the refusing operation named first, so a driver can tell
/// which local-family verb stopped at the family observation.
pub(crate) fn store_in(operation: &str, error: &StoreError) -> ModelError {
    let mapped = store(error);
    ModelError::new(mapped.code, format!("{operation}: {}", mapped.message))
}

/// Map a store error: a held family lock is an open operation, undecodable
/// family metadata is an invalid artifact, I/O is I/O, and an unimplemented
/// store is an unsupported operation.
pub(crate) fn store(error: &StoreError) -> ModelError {
    let code = match error {
        StoreError::Busy { .. } => ErrorCode::OpenOperation,
        StoreError::LockingUnsupported { .. } | StoreError::Unimplemented { .. } => {
            ErrorCode::UnsupportedOperation
        }
        StoreError::Malformed { .. }
        | StoreError::Oversize { .. }
        | StoreError::ConflictingMetadata { .. }
        | StoreError::PointerTargetInvalid { .. } => ErrorCode::ManifestInvalid,
        StoreError::NoFamily { .. } => ErrorCode::MemberNotFound,
        // Both are orchestration-order/shape refusals: the caller asked for a
        // write the family's own metadata makes unsafe (LCM1.0c-rem1 State
        // P2-2; LCM1.0c-fu1 State S2-P3-1).
        StoreError::PointerStillInstalled { .. } | StoreError::PathMismatch { .. } => {
            ErrorCode::InvalidRequest
        }
        StoreError::Refused(refusal) => return self::refusal(refusal),
        StoreError::Io { .. } | StoreError::Partial { .. } => ErrorCode::IoError,
    };
    ModelError::new(code, format!("local family: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_family_model::{MemberName, MemberState};
    use gwz_family_store_contract::StoreOperation;
    use std::path::PathBuf;

    #[test]
    fn refusals_and_store_errors_map_onto_existing_codes_only() {
        let name = MemberName::parse("A").unwrap();
        assert_eq!(
            refusal(&Refusal::NameCollision {
                name: name.clone(),
                holder_path: "../a".to_owned()
            })
            .code,
            ErrorCode::PathCollision
        );
        assert_eq!(
            refusal(&Refusal::NotFound { name: name.clone() }).code,
            ErrorCode::MemberNotFound
        );
        assert_eq!(
            refusal(&Refusal::WrongState {
                name,
                expected: MemberState::Ready,
                actual: MemberState::Creating
            })
            .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            store(&StoreError::Busy {
                lock_path: PathBuf::from("/root/.gwz/local-family.lock")
            })
            .code,
            ErrorCode::OpenOperation
        );
        assert_eq!(
            store(&StoreError::Unimplemented {
                operation: StoreOperation::ReadIndex
            })
            .code,
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(
            store(&StoreError::Malformed {
                path: PathBuf::from("x"),
                detail: "y".to_owned()
            })
            .code,
            ErrorCode::ManifestInvalid
        );
        assert_eq!(
            store(&StoreError::Partial {
                operation: StoreOperation::WritePointer,
                completed: Vec::new(),
                path: PathBuf::from("x"),
                detail: "y".to_owned()
            })
            .code,
            ErrorCode::IoError
        );
        assert_eq!(
            store(&StoreError::PathMismatch {
                member: "A".to_owned(),
                recorded: PathBuf::from("/root/../ws-A"),
                requested: PathBuf::from("/root/../ws-B"),
            })
            .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(unsupported("x").code, ErrorCode::UnsupportedOperation);
        assert_eq!(invalid("x").code, ErrorCode::InvalidRequest);
    }

    /// Design §6/§7 (operator ruling 2026-09-05): the family-only merge miss
    /// is `unknown_local`, and the message carries the state detail -- a
    /// row's lifecycle state when one exists, "no ready family member" when
    /// none does -- with no Git-remote fallback in either case.
    #[test]
    fn the_family_merge_miss_is_unknown_local_with_the_state_detail() {
        let absent = unknown_local("origin", None);
        assert_eq!(absent.code, ErrorCode::UnknownLocal);
        assert!(absent.message.contains("`origin`"), "{}", absent.message);
        assert!(
            absent.message.contains("never falls back to a Git remote"),
            "{}",
            absent.message
        );
        for state in [MemberState::Creating, MemberState::Disposing] {
            let not_ready = unknown_local("B", Some(state));
            assert_eq!(not_ready.code, ErrorCode::UnknownLocal);
            assert!(not_ready.message.contains("`B`"), "{}", not_ready.message);
            assert!(
                not_ready.message.contains(state.as_str()),
                "{}: {}",
                state.as_str(),
                not_ready.message
            );
            assert!(
                not_ready.message.contains("not ready"),
                "{}",
                not_ready.message
            );
        }
    }
}
