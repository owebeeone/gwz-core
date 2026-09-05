//! Library error -> `ModelError` translation for the local clone family.
//!
//! Public contracts carry owned typed errors; core maps them onto the
//! existing `GwzErrorCode` registry at this boundary. No new code is
//! allocated at LCM1.0c (see `docs/ErrorCatalog.md`, "Local Clone Family").

use gwz_family_model::Refusal;
use gwz_family_store_contract::StoreError;

use crate::model::{ErrorCode, ModelError};

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
        StoreError::PointerStillInstalled { .. } => ErrorCode::InvalidRequest,
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
        assert_eq!(unsupported("x").code, ErrorCode::UnsupportedOperation);
        assert_eq!(invalid("x").code, ErrorCode::InvalidRequest);
    }
}
