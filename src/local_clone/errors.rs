//! Library error -> `ModelError` translation for the local clone family.
//!
//! Public contracts carry owned typed errors; core maps them onto the
//! `GwzErrorCode` registry at this boundary (`docs/ErrorCatalog.md`, "Local
//! Clone Family"). LCM1.0c allocated nothing; follow-up 2 allocated
//! `unknown_local` (62); LCM1.1 fix 1 (2026-09-06) allocated the four
//! local-create outcomes the wiring had folded into `unsupported_operation`
//! and `io_error` -- `unsupported_source_layout` (63), `copy_failed` (64),
//! `source_drift` (65) and `destination_incomplete` (66) -- so that
//! `unsupported_operation` means exactly "not built yet" and `io_error`
//! exactly an I/O failure. The install mappings live here, in one table,
//! so a driver-facing code is decided in one place ([`install_error_code`],
//! [`install_port_code`], [`install_refusal_code`]). LCM1.2 (2026-09-06)
//! allocated the two family-merge import outcomes that would otherwise have
//! folded into `invalid_request`/`member_not_found` and
//! `git_command_failed` -- `pairing_mismatch` (67) and `import_incomplete`
//! (68) -- and maps the rest of `gwz_local_import::ImportError` onto codes
//! that already mean the same thing ([`import_error_code`]).

use gwz_copy_contract::CopyErrorCategory;
use gwz_family_model::{MemberState, Refusal};
use gwz_family_store_contract::StoreError;
use gwz_local_import::ImportError;
use gwz_repo_contract::LayoutError;
use gwz_workspace_install::{InstallError, InstallPortError, InstallRefusal};

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
        | Refusal::NestedPath { .. }
        | Refusal::AllocationCollision { .. } => ErrorCode::PathCollision,
        Refusal::NotFound { .. } => ErrorCode::MemberNotFound,
        Refusal::InvalidRow { .. }
        | Refusal::PathNotNormalised { .. }
        | Refusal::WrongState { .. }
        | Refusal::AllocationMismatch { .. }
        | Refusal::NotDisposing { .. } => ErrorCode::InvalidRequest,
        // `Refusal` is `#[non_exhaustive]` (F1): a refusal the model adds
        // later is a request the family's own metadata refuses, until this
        // table names it.
        _ => ErrorCode::InvalidRequest,
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

/// The code behind a source-layout verdict (design §4.0). A hazard list is
/// the §4.0 refusal itself; a `.git` entry that is not a repository is a
/// layout the copy cannot reproduce either; an inspector that could not
/// read far enough to decide has an I/O failure, not a verdict (lane I
/// proposal I-3); an inspector that does not inspect is a build gap.
pub(crate) fn layout_code(error: &LayoutError) -> ErrorCode {
    match error {
        LayoutError::Unsupported { .. } | LayoutError::NotARepository { .. } => {
            ErrorCode::UnsupportedSourceLayout
        }
        LayoutError::ReadFailed { .. } => ErrorCode::IoError,
        LayoutError::Unimplemented { .. } => ErrorCode::UnsupportedOperation,
    }
}

/// The code behind an install port error, whether it stopped the capture
/// before the family lock or a port during installation (LCM1.1 fix 1).
pub(crate) fn install_port_code(error: &InstallPortError) -> ErrorCode {
    match error {
        InstallPortError::Layout(error) => layout_code(error),
        // Design §4 step 3, §12: the source moved between the snapshot and
        // publication; the destination is a copy of a moving source.
        InstallPortError::Drift { .. } => ErrorCode::SourceDrift,
        InstallPortError::Unimplemented { .. } => ErrorCode::UnsupportedOperation,
        // A destination that could not be allocated or observed, a
        // configuration that could not be read or written, a construction
        // that failed: I/O-class failures, reported with their detail.
        InstallPortError::Construction { .. }
        | InstallPortError::Configuration { .. }
        | InstallPortError::Destination { .. } => ErrorCode::IoError,
    }
}

/// The code behind an admission refusal (design §4 step 1): refused before
/// reservation, so nothing was written.
pub(crate) fn install_refusal_code(refusal: &InstallRefusal) -> ErrorCode {
    match refusal {
        InstallRefusal::Family(refusal) => self::refusal(refusal).code,
        InstallRefusal::SourceLayout(error) => layout_code(error),
        InstallRefusal::NameIsRemote(_)
        | InstallRefusal::RootNotCaptured
        | InstallRefusal::BranchExists { .. }
        | InstallRefusal::BranchNotSupported { .. } => ErrorCode::InvalidRequest,
        InstallRefusal::DestinationNotEmpty { .. }
        | InstallRefusal::DestinationIsWorkspace { .. } => ErrorCode::PathCollision,
        InstallRefusal::SourceOpenMerge { .. } => ErrorCode::OpenOperation,
    }
}

/// The code behind an install failure: the typed cause decides, and the
/// message (built by the caller) names the step, the cause, every
/// completed effect and what is retained.
pub(crate) fn install_error_code(error: &InstallError) -> ErrorCode {
    match error {
        InstallError::Refused(refusals) => refusals
            .first()
            .map_or(ErrorCode::InvalidRequest, install_refusal_code),
        InstallError::Source(error) | InstallError::Port(error) => install_port_code(error),
        InstallError::Copy(error) => match error.category {
            CopyErrorCategory::DestinationNotEmpty => ErrorCode::PathCollision,
            CopyErrorCategory::Unimplemented => ErrorCode::UnsupportedOperation,
            // A copy cancelled between entries leaves exactly the shape a
            // cancelled install leaves: a partial destination and the
            // `creating` row, both retained.
            CopyErrorCategory::Cancelled => ErrorCode::DestinationIncomplete,
            // Design §4: permission, space, I/O and metadata failures are
            // errors, not "unsupported"; §12: the partial destination is
            // retained and the source unchanged.
            CopyErrorCategory::SourceMissing
            | CopyErrorCategory::SourceUnreadable
            | CopyErrorCategory::DestinationUnwritable
            | CopyErrorCategory::UnsupportedEntry
            | CopyErrorCategory::ShortWrite
            | CopyErrorCategory::MetadataFailed
            | CopyErrorCategory::Io => ErrorCode::CopyFailed,
        },
        InstallError::Store(error) => store(error).code,
        // A failed completion rule and a cancelled install leave the same
        // retained shape (design §4 step 4: "errors or interruption leave
        // the directory and diagnostic row for inspection"), which `gwz
        // local list` shows as `creating/incomplete`; the message says
        // which it was.
        InstallError::Incomplete(_) | InstallError::Cancelled => ErrorCode::DestinationIncomplete,
    }
}

/// The code behind a family-merge import failure (design §6, §6.2; LCM1.2).
/// Every arm is decided by the typed cause; the message (built by the
/// caller) names the step, the cause and every retained import ref.
///
/// - a request shape the library refuses is `invalid_request`;
/// - the two workspaces no longer being the same shape is
///   `pairing_mismatch` (67): refused before any fetch, nothing written;
/// - a source ref that does not resolve in a paired source is the merge
///   engine's own start-validation code, `merge_validation_failed` -- the
///   family merge validates its source before transfer (design §6.1),
///   where the ordinary merge lets libgit2 answer at planning time;
/// - the fresh, collision-checked import name already existing in a
///   receiver is `path_collision`: a namespace collision at a target that
///   exists, nothing written, the next invocation mints another id;
/// - a received id that differs from the captured one is `source_drift`
///   (65): the source moved between capture and fetch, the cure is the
///   quiescence design §2 asks for, and the refs created so far are
///   retained;
/// - a transfer that stopped, a receiver that could not be read, or a
///   cancellation is `import_incomplete` (68): the refs created so far are
///   retained and the engine was not entered.
pub(crate) fn import_error_code(error: &ImportError) -> ErrorCode {
    match error {
        ImportError::InvalidRequest { .. } => ErrorCode::InvalidRequest,
        ImportError::PairingIncomplete { .. } => ErrorCode::PairingMismatch,
        ImportError::SourceMissing { .. } => ErrorCode::MergeValidationFailed,
        ImportError::RefCollision { .. } => ErrorCode::PathCollision,
        ImportError::VectorMismatch { .. } => ErrorCode::SourceDrift,
        ImportError::TransferFailed { .. } | ImportError::Cancelled { .. } => {
            ErrorCode::ImportIncomplete
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_copy_contract::CopyError;
    use gwz_family_model::{MemberName, MemberState, RemoteNameCollision};
    use gwz_family_store_contract::StoreOperation;
    use gwz_repo_contract::LayoutHazard;
    use gwz_workspace_install::CompletionFault;
    use std::path::PathBuf;

    #[test]
    fn refusals_and_store_errors_map_onto_the_registry() {
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
        // F1 (LCM1.0c follow-up 2): the two refusals that were message folds.
        assert_eq!(
            refusal(&Refusal::AllocationCollision {
                name: name.clone(),
                holder: "root".to_owned()
            })
            .code,
            ErrorCode::PathCollision
        );
        assert_eq!(
            refusal(&Refusal::PathNotNormalised {
                name: name.clone(),
                path: "../ws-A/".to_owned(),
                normalised: "../ws-A".to_owned()
            })
            .code,
            ErrorCode::InvalidRequest
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

    /// LCM1.2 (2026-09-06): every `ImportError` variant has one code, the two
    /// allocated ones are distinct from each other, from the create codes and
    /// from the codes they would have folded into, and the reused codes are
    /// the ones whose meaning already fits (see `import_error_code`).
    #[test]
    fn import_failures_map_onto_the_family_merge_codes() {
        use gwz_local_import::{ImportEffect, MovedMember, SourceProblem};
        use gwz_repo_contract::{ObjectFormat, ObjectId, RepoKey};
        let app = RepoKey::Member {
            id: "mem_app".to_owned(),
        };
        let oid = ObjectId::parse_hex(ObjectFormat::Sha1, &"ab".repeat(20)).unwrap();
        let effects = vec![ImportEffect::RefCreated {
            key: app.clone(),
            import_ref: "refs/gwz/local-imports/xfer_1".to_owned(),
            oid: oid.clone(),
        }];
        assert_eq!(
            import_error_code(&ImportError::InvalidRequest {
                detail: "x".to_owned()
            }),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            import_error_code(&ImportError::PairingIncomplete {
                missing: vec![app.clone()],
                moved: vec![MovedMember {
                    key: app.clone(),
                    receiver_path: "app".to_owned(),
                    source_path: "lib/app".to_owned(),
                }],
            }),
            ErrorCode::PairingMismatch
        );
        assert_eq!(
            import_error_code(&ImportError::SourceMissing {
                missing: vec![SourceProblem {
                    key: app.clone(),
                    detail: "refs/heads/lane/x does not resolve".to_owned(),
                }],
            }),
            ErrorCode::MergeValidationFailed
        );
        assert_eq!(
            import_error_code(&ImportError::RefCollision {
                key: app.clone(),
                import_ref: "refs/gwz/local-imports/xfer_1".to_owned(),
            }),
            ErrorCode::PathCollision
        );
        assert_eq!(
            import_error_code(&ImportError::VectorMismatch {
                key: app.clone(),
                expected: oid.clone(),
                received: None,
                effects: effects.clone(),
            }),
            ErrorCode::SourceDrift
        );
        assert_eq!(
            import_error_code(&ImportError::TransferFailed {
                key: app,
                detail: "x".to_owned(),
                effects: effects.clone(),
            }),
            ErrorCode::ImportIncomplete
        );
        assert_eq!(
            import_error_code(&ImportError::Cancelled { effects }),
            ErrorCode::ImportIncomplete
        );
        // The two allocated codes overload nothing they replace.
        for code in [ErrorCode::PairingMismatch, ErrorCode::ImportIncomplete] {
            for other in [
                ErrorCode::InvalidRequest,
                ErrorCode::MemberNotFound,
                ErrorCode::GitCommandFailed,
                ErrorCode::IoError,
                ErrorCode::UnsupportedOperation,
                ErrorCode::SourceDrift,
                ErrorCode::DestinationIncomplete,
            ] {
                assert_ne!(code, other);
            }
        }
        assert_ne!(ErrorCode::PairingMismatch, ErrorCode::ImportIncomplete);
    }

    fn copy_error(category: CopyErrorCategory) -> InstallError {
        InstallError::Copy(Box::new(CopyError {
            failed_path: PathBuf::from("app/locked.txt"),
            category,
            detail: "detail".to_owned(),
            partial: gwz_copy_contract::CopyReport::default(),
        }))
    }

    /// LCM1.1 fix 1 (2026-09-06): the four local-create outcomes are their
    /// own codes, `unsupported_operation` is exactly "not built yet" and
    /// `io_error` exactly an I/O failure -- at every call site that decides
    /// a code: the capture before the lock and the ports (`install_port_code`),
    /// admission (`install_refusal_code`) and the failure (`install_error_code`).
    #[test]
    fn install_failures_map_onto_the_four_local_create_codes() {
        let hazard = LayoutError::Unsupported {
            path: PathBuf::from("/src/app"),
            hazards: vec![LayoutHazard::Alternates {
                path: PathBuf::from("/src/app/.git/objects/info/alternates"),
            }],
        };
        // A §4.0 hazard, before the lock and at admission alike.
        assert_eq!(
            install_port_code(&InstallPortError::Layout(hazard.clone())),
            ErrorCode::UnsupportedSourceLayout
        );
        assert_eq!(
            install_refusal_code(&InstallRefusal::SourceLayout(hazard.clone())),
            ErrorCode::UnsupportedSourceLayout
        );
        assert_eq!(
            install_error_code(&InstallError::Refused(vec![InstallRefusal::SourceLayout(
                hazard
            )])),
            ErrorCode::UnsupportedSourceLayout
        );
        assert_eq!(
            layout_code(&LayoutError::NotARepository {
                path: PathBuf::from("/src/app")
            }),
            ErrorCode::UnsupportedSourceLayout
        );
        // An inspector that could not read is I/O; one that does not inspect
        // is a build gap -- neither is a layout verdict.
        assert_eq!(
            layout_code(&LayoutError::ReadFailed {
                path: PathBuf::from("/src/app"),
                detail: "EIO".to_owned()
            }),
            ErrorCode::IoError
        );
        assert_eq!(
            layout_code(&LayoutError::Unimplemented { operation: "x" }),
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(
            install_port_code(&InstallPortError::Unimplemented { operation: "x" }),
            ErrorCode::UnsupportedOperation
        );
        // Drift.
        let drift = InstallPortError::Drift {
            detail: "@root: branches differ".to_owned(),
        };
        assert_eq!(install_port_code(&drift), ErrorCode::SourceDrift);
        assert_eq!(
            install_error_code(&InstallError::Source(Box::new(drift.clone()))),
            ErrorCode::SourceDrift
        );
        assert_eq!(
            install_error_code(&InstallError::Port(Box::new(drift))),
            ErrorCode::SourceDrift
        );
        // The I/O-class port errors stay `io_error`.
        for error in [
            InstallPortError::Construction {
                detail: "x".to_owned(),
            },
            InstallPortError::Configuration {
                detail: "x".to_owned(),
            },
            InstallPortError::Destination {
                path: PathBuf::from("/dest"),
                detail: "x".to_owned(),
            },
        ] {
            assert_eq!(install_port_code(&error), ErrorCode::IoError, "{error}");
        }
        // The copy: every failure category is `copy_failed`; an occupied
        // destination stays the collision it always was; a copier that does
        // not copy is a build gap; a cancelled copy is the retained shape.
        for category in [
            CopyErrorCategory::SourceMissing,
            CopyErrorCategory::SourceUnreadable,
            CopyErrorCategory::DestinationUnwritable,
            CopyErrorCategory::UnsupportedEntry,
            CopyErrorCategory::ShortWrite,
            CopyErrorCategory::MetadataFailed,
            CopyErrorCategory::Io,
        ] {
            assert_eq!(
                install_error_code(&copy_error(category)),
                ErrorCode::CopyFailed,
                "{category:?}"
            );
        }
        assert_eq!(
            install_error_code(&copy_error(CopyErrorCategory::DestinationNotEmpty)),
            ErrorCode::PathCollision
        );
        assert_eq!(
            install_error_code(&copy_error(CopyErrorCategory::Unimplemented)),
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(
            install_error_code(&copy_error(CopyErrorCategory::Cancelled)),
            ErrorCode::DestinationIncomplete
        );
        // The completion rules and a cancelled install.
        assert_eq!(
            install_error_code(&InstallError::Incomplete(vec![
                CompletionFault::NotIndependent {
                    detail: "app: objects missing".to_owned()
                }
            ])),
            ErrorCode::DestinationIncomplete
        );
        assert_eq!(
            install_error_code(&InstallError::Cancelled),
            ErrorCode::DestinationIncomplete
        );
        // The admission refusals that keep their existing codes.
        assert_eq!(
            install_refusal_code(&InstallRefusal::NameIsRemote(RemoteNameCollision {
                name: "origin".to_owned(),
                member: "@root".to_owned(),
            })),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            install_refusal_code(&InstallRefusal::DestinationNotEmpty {
                destination: PathBuf::from("/dest")
            }),
            ErrorCode::PathCollision
        );
        assert_eq!(
            install_refusal_code(&InstallRefusal::SourceOpenMerge {
                detail: "merge_1".to_owned()
            }),
            ErrorCode::OpenOperation
        );
        assert_eq!(
            install_error_code(&InstallError::Store(Box::new(StoreError::Busy {
                lock_path: PathBuf::from("/root/.gwz/local-family.lock")
            }))),
            ErrorCode::OpenOperation
        );
        // Nothing above is an overload: the four are distinct from each
        // other and from the two codes they were folded into.
        let four = [
            ErrorCode::UnsupportedSourceLayout,
            ErrorCode::CopyFailed,
            ErrorCode::SourceDrift,
            ErrorCode::DestinationIncomplete,
        ];
        for (index, code) in four.iter().enumerate() {
            assert_ne!(*code, ErrorCode::UnsupportedOperation);
            assert_ne!(*code, ErrorCode::IoError);
            assert!(four[index + 1..].iter().all(|other| other != code));
        }
    }
}
