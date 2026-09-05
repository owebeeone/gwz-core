//! Request-shape validation for the local clone family dispatch slots.
//!
//! Every check here runs before any family observation, lock file, copy or
//! import (design §6.2). The functions are pure over the request and return
//! validated plain values that the adapters consume.

use gwz_family_model::{CloneMode, MemberName, ROOT_NAME, RemoteToken};
use gwz_local_disposal::HazardWaiver;

use super::errors::{invalid, unsupported};
use crate::model::ModelResult;

/// A `CloneLocalWorkspaceRequest` whose shape core accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedCloneLocal {
    pub name: MemberName,
    pub mode: CloneMode,
    pub dest: Option<String>,
    pub branch: Option<String>,
}

pub fn validate_clone_local(
    request: &crate::CloneLocalWorkspaceRequest,
) -> ModelResult<ValidatedCloneLocal> {
    let name = MemberName::parse(&request.name)
        .map_err(|error| invalid(format!("invalid clone name: {error}")))?;
    let mode = match request.mode {
        crate::LocalCloneMode::Verbatim => CloneMode::Verbatim,
        crate::LocalCloneMode::Clean => CloneMode::Clean,
        crate::LocalCloneMode::Bare => CloneMode::Bare,
    };
    if let Some(branch) = request.branch.as_deref() {
        if mode == CloneMode::Verbatim {
            return Err(invalid(
                "-b <branch> is accepted only with --clean or --bare",
            ));
        }
        if branch.trim().is_empty() {
            return Err(invalid("-b <branch> must not be empty"));
        }
    }
    if request
        .dest
        .as_deref()
        .is_some_and(|dest| dest.trim().is_empty())
    {
        return Err(invalid("dest must not be empty when supplied"));
    }
    if request.meta.dry_run == Some(true) {
        return Err(unsupported("local create with dry_run"));
    }
    Ok(ValidatedCloneLocal {
        name,
        mode,
        dest: request.dest.clone(),
        branch: request.branch.clone(),
    })
}

/// A `LocalFamilyRequest` whose shape core accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidatedLocalFamily {
    List,
    Dispose {
        name: MemberName,
        keep: bool,
        waivers: Vec<HazardWaiver>,
    },
    Disband,
}

pub fn validate_local_family(
    request: &crate::LocalFamilyRequest,
) -> ModelResult<ValidatedLocalFamily> {
    let validated = match request.op {
        crate::LocalFamilyOp::List => {
            reject_present("name", request.name.is_some())?;
            reject_present("keep", request.keep.is_some())?;
            reject_present("force_hazards", !request.force_hazards.is_empty())?;
            ValidatedLocalFamily::List
        }
        crate::LocalFamilyOp::Disband => {
            reject_present("name", request.name.is_some())?;
            reject_present("keep", request.keep.is_some())?;
            reject_present("force_hazards", !request.force_hazards.is_empty())?;
            ValidatedLocalFamily::Disband
        }
        crate::LocalFamilyOp::Dispose => {
            let raw = request
                .name
                .as_deref()
                .ok_or_else(|| invalid("dispose requires a member name"))?;
            if raw == ROOT_NAME {
                return Err(invalid(
                    "root is never disposed; `gwz local disband` retires the family",
                ));
            }
            let name = MemberName::parse(raw)
                .map_err(|error| invalid(format!("invalid member name: {error}")))?;
            let waivers = HazardWaiver::parse_all(&request.force_hazards)
                .map_err(|error| invalid(error.to_string()))?;
            let keep = request.keep.unwrap_or(false);
            if keep && !waivers.is_empty() {
                return Err(invalid(
                    "--keep and --force <hazards> are mutually exclusive",
                ));
            }
            ValidatedLocalFamily::Dispose {
                name,
                keep,
                waivers,
            }
        }
    };
    if request.meta.dry_run == Some(true) {
        return Err(unsupported("local family operations with dry_run"));
    }
    Ok(validated)
}

/// The family selector of a `MergeRequest` whose shape core accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyMergeSelector {
    pub token: RemoteToken,
    /// The ref resolved inside the source; `None` means the source's HEAD.
    pub source_ref: Option<String>,
}

/// Validate the family half of a merge start. The engine's own validation
/// runs later on the delegated request with the selector cleared.
pub fn validate_family_merge(request: &crate::MergeRequest) -> ModelResult<FamilyMergeSelector> {
    let raw = request
        .local_source_name
        .as_deref()
        .ok_or_else(|| invalid("a family merge needs local_source_name"))?;
    if request.op != crate::MergeOp::Start {
        return Err(invalid(
            "local_source_name is accepted only when starting a merge",
        ));
    }
    if raw.trim().is_empty() {
        return Err(invalid("local_source_name must not be empty"));
    }
    if request.merge_id.is_some() {
        return Err(invalid("merge_id is not accepted for merge start"));
    }
    if request.preserve.is_some() {
        return Err(invalid("preserve is not accepted for merge start"));
    }
    if request
        .source_ref
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(invalid("source_ref must not be empty when supplied"));
    }
    if request.meta.dry_run == Some(true) {
        return Err(unsupported("local family merge with dry_run"));
    }
    Ok(FamilyMergeSelector {
        token: RemoteToken::new(raw),
        source_ref: request.source_ref.clone(),
    })
}

fn reject_present(field: &str, present: bool) -> ModelResult<()> {
    if present {
        return Err(invalid(format!(
            "{field} is not accepted for this local family operation"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ErrorCode;

    fn meta(dry_run: Option<bool>) -> crate::RequestMeta {
        crate::RequestMeta {
            request_id: "req".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            dry_run,
            ..crate::RequestMeta::default()
        }
    }

    fn clone_request(name: &str, mode: crate::LocalCloneMode) -> crate::CloneLocalWorkspaceRequest {
        crate::CloneLocalWorkspaceRequest {
            meta: meta(None),
            name: name.to_owned(),
            dest: None,
            mode,
            branch: None,
        }
    }

    fn family_request(op: crate::LocalFamilyOp) -> crate::LocalFamilyRequest {
        crate::LocalFamilyRequest {
            meta: meta(None),
            op,
            name: None,
            keep: None,
            force_hazards: Vec::new(),
        }
    }

    fn merge_request(op: crate::MergeOp) -> crate::MergeRequest {
        crate::MergeRequest {
            meta: meta(None),
            op,
            source_ref: None,
            merge_id: None,
            mode: None,
            message: None,
            preserve: None,
            filesystem_strict: None,
            local_source_name: Some("A".to_owned()),
        }
    }

    #[test]
    fn clone_shape_names_modes_branches_and_dry_run() {
        let ok =
            validate_clone_local(&clone_request("A", crate::LocalCloneMode::Verbatim)).unwrap();
        assert_eq!(ok.mode, CloneMode::Verbatim);
        assert_eq!(ok.name.as_str(), "A");
        for reserved in ["root", "origin", "HEAD", "FETCH_HEAD", "", "a/b", "a:b"] {
            let error =
                validate_clone_local(&clone_request(reserved, crate::LocalCloneMode::Clean))
                    .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidRequest, "{reserved:?}");
        }
        let mut branch = clone_request("A", crate::LocalCloneMode::Verbatim);
        branch.branch = Some("lane/x".to_owned());
        assert_eq!(
            validate_clone_local(&branch).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        branch.mode = crate::LocalCloneMode::Clean;
        assert_eq!(
            validate_clone_local(&branch).unwrap().branch.as_deref(),
            Some("lane/x")
        );
        branch.mode = crate::LocalCloneMode::Bare;
        assert!(validate_clone_local(&branch).is_ok());
        let mut dry = clone_request("A", crate::LocalCloneMode::Verbatim);
        dry.meta.dry_run = Some(true);
        assert_eq!(
            validate_clone_local(&dry).unwrap_err().code,
            ErrorCode::UnsupportedOperation
        );
        let mut bad_dry = clone_request("origin", crate::LocalCloneMode::Verbatim);
        bad_dry.meta.dry_run = Some(true);
        assert_eq!(
            validate_clone_local(&bad_dry).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "shape is checked before the dry-run refusal"
        );
    }

    #[test]
    fn family_shape_per_op_hazards_keep_and_dry_run() {
        assert_eq!(
            validate_local_family(&family_request(crate::LocalFamilyOp::List)).unwrap(),
            ValidatedLocalFamily::List
        );
        assert_eq!(
            validate_local_family(&family_request(crate::LocalFamilyOp::Disband)).unwrap(),
            ValidatedLocalFamily::Disband
        );
        let mut list_with_name = family_request(crate::LocalFamilyOp::List);
        list_with_name.name = Some("A".to_owned());
        assert_eq!(
            validate_local_family(&list_with_name).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        let mut dispose = family_request(crate::LocalFamilyOp::Dispose);
        assert_eq!(
            validate_local_family(&dispose).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "dispose needs a name"
        );
        dispose.name = Some("root".to_owned());
        assert!(
            validate_local_family(&dispose)
                .unwrap_err()
                .message
                .contains("never disposed")
        );
        dispose.name = Some("C".to_owned());
        dispose.force_hazards = vec!["open-merge".to_owned(), "dirty".to_owned()];
        let ValidatedLocalFamily::Dispose {
            name,
            keep,
            waivers,
        } = validate_local_family(&dispose).unwrap()
        else {
            panic!("dispose validates");
        };
        assert_eq!(name.as_str(), "C");
        assert!(!keep);
        assert_eq!(waivers, vec![HazardWaiver::OpenMerge, HazardWaiver::Dirty]);
        dispose.force_hazards = vec!["true".to_owned()];
        assert!(
            validate_local_family(&dispose)
                .unwrap_err()
                .message
                .contains("unknown hazard")
        );
        dispose.force_hazards = vec!["dirty".to_owned()];
        dispose.keep = Some(true);
        assert!(
            validate_local_family(&dispose)
                .unwrap_err()
                .message
                .contains("mutually exclusive")
        );
        dispose.force_hazards.clear();
        assert!(matches!(
            validate_local_family(&dispose).unwrap(),
            ValidatedLocalFamily::Dispose { keep: true, .. }
        ));
        dispose.meta.dry_run = Some(true);
        assert_eq!(
            validate_local_family(&dispose).unwrap_err().code,
            ErrorCode::UnsupportedOperation
        );
    }

    #[test]
    fn family_merge_selector_is_start_only_and_never_dry_run() {
        let ok = validate_family_merge(&merge_request(crate::MergeOp::Start)).unwrap();
        assert_eq!(ok.token.as_str(), "A");
        assert_eq!(ok.source_ref, None);
        for op in [
            crate::MergeOp::Resume,
            crate::MergeOp::Abort,
            crate::MergeOp::Status,
            crate::MergeOp::Gc,
        ] {
            assert_eq!(
                validate_family_merge(&merge_request(op)).unwrap_err().code,
                ErrorCode::InvalidRequest,
                "{op:?}"
            );
        }
        let mut with_ref = merge_request(crate::MergeOp::Start);
        with_ref.source_ref = Some("lane/agent-17".to_owned());
        assert_eq!(
            validate_family_merge(&with_ref)
                .unwrap()
                .source_ref
                .as_deref(),
            Some("lane/agent-17")
        );
        let mut with_id = merge_request(crate::MergeOp::Start);
        with_id.merge_id = Some("m".to_owned());
        assert_eq!(
            validate_family_merge(&with_id).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        let mut dry = merge_request(crate::MergeOp::Start);
        dry.meta.dry_run = Some(true);
        assert_eq!(
            validate_family_merge(&dry).unwrap_err().code,
            ErrorCode::UnsupportedOperation
        );
        let mut none = merge_request(crate::MergeOp::Start);
        none.local_source_name = None;
        assert_eq!(
            validate_family_merge(&none).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}
