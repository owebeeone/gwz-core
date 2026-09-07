//! Request-shape validation for the local clone family dispatch slots.
//!
//! Every check here runs before any family observation, lock file, copy or
//! import (design §6.2). The functions are pure over the request and return
//! validated plain values that the adapters consume.

use gwz_family_model::{
    CloneMode, DisposeTarget, MemberName, RemoteToken, classify_dispose_target,
};
use gwz_local_disposal::HazardWaiver;
use gwz_local_import::IMPORT_REF_NAMESPACE;

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
    reject_selection(&request.meta)?;
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
    if request
        .copy_source
        .as_deref()
        .is_some_and(|source| source.trim().is_empty())
    {
        return Err(invalid(
            "copy_source (--from <name|path>) must not be empty when supplied",
        ));
    }
    if request.meta.dry_run == Some(true) {
        return Err(unsupported("local create with dry_run"));
    }
    // Tag 6 (`copy_source`, the `--from` selector; design §7, §11 item 11)
    // is decoded and shape-checked above; choosing another family member or
    // path as the copy source is LCM3.2, so a present value is refused here
    // rather than silently copying the addressed workspace instead.
    if request.copy_source.is_some() {
        return Err(unsupported(
            "local create from an explicit copy source (--from <name|path>)",
        ));
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
    reject_selection(&request.meta)?;
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
            // The model decides what a dispose token addresses (F2): the
            // root is a legal token and never disposed, so its refusal says
            // why rather than reporting a reserved word.
            let name = match classify_dispose_target(raw) {
                DisposeTarget::Root => {
                    return Err(invalid(
                        "root is never disposed; `gwz local disband` retires the family",
                    ));
                }
                DisposeTarget::Invalid(error) => {
                    return Err(invalid(format!("invalid member name: {error}")));
                }
                DisposeTarget::Member(name) => name,
            };
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

fn reject_selection(meta: &crate::RequestMeta) -> ModelResult<()> {
    if crate::workspace_ops::has_explicit_target_selection(meta.selection.as_ref()) {
        return Err(invalid(
            "local operations address a whole workspace and do not accept target selection",
        ));
    }
    Ok(())
}

/// The family selector of a `MergeRequest` whose shape core accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyMergeSelector {
    pub token: RemoteToken,
    /// The ref resolved inside the source; `None` means the source's HEAD.
    pub source_ref: Option<String>,
}

/// Validate the family half of a merge start, then the engine's own start
/// gate on the projected request (selector cleared, a placeholder import ref
/// as `source_ref`), so a start the engine would refuse after the import is
/// refused before it (LCM1.0c-rem1, Code P3-1; design §6.2). The engine
/// validates the delegated request again later; the two gates are one
/// function (`MergeRequest::validate_merge_start_shape`).
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
    // The engine's start gate, on the request as the wrapper will delegate
    // it: the selector is cleared and the import ref (minted at import time)
    // stands in as `source_ref`. Shape stays ahead of the dry-run refusal.
    let projected = crate::MergeRequest {
        local_source_name: None,
        source_ref: Some(format!("{IMPORT_REF_NAMESPACE}pending")),
        ..request.clone()
    };
    projected.validate_merge_start_shape()?;
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
            copy_source: None,
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

    /// Design §7 / §11 item 11 (operator ruling 2026-09-05): tag 6 is
    /// `copy_source`, the `--from <name|path>` selector. Core decodes it and
    /// checks its shape (an empty value is malformed, ahead of every
    /// unsupported refusal); a present value is refused as unsupported until
    /// LCM3.2 implements the selector, and never silently ignored.
    #[test]
    fn clone_shape_decodes_copy_source_and_refuses_it_until_lcm3_2() {
        let mut from_member = clone_request("A", crate::LocalCloneMode::Verbatim);
        from_member.copy_source = Some("B".to_owned());
        let error = validate_clone_local(&from_member).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedOperation);
        assert!(error.message.contains("--from"), "{}", error.message);

        let mut from_path = clone_request("A", crate::LocalCloneMode::Clean);
        from_path.copy_source = Some("../gwz-dev-B".to_owned());
        assert_eq!(
            validate_clone_local(&from_path).unwrap_err().code,
            ErrorCode::UnsupportedOperation
        );

        let mut empty = clone_request("A", crate::LocalCloneMode::Verbatim);
        empty.copy_source = Some("  ".to_owned());
        let error = validate_clone_local(&empty).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(error.message.contains("copy_source"), "{}", error.message);

        let mut empty_and_dry = clone_request("A", crate::LocalCloneMode::Verbatim);
        empty_and_dry.copy_source = Some(String::new());
        empty_and_dry.meta.dry_run = Some(true);
        assert_eq!(
            validate_clone_local(&empty_and_dry).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "shape stays ahead of the dry-run refusal"
        );

        let mut dry_from = clone_request("A", crate::LocalCloneMode::Verbatim);
        dry_from.copy_source = Some("B".to_owned());
        dry_from.meta.dry_run = Some(true);
        assert!(
            validate_clone_local(&dry_from)
                .unwrap_err()
                .message
                .contains("dry_run"),
            "the pinned dry-run refusal comes before the copy-source one"
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

    /// LCM1.0c-rem1 (Code P3-1): the wrapper runs the engine's own start
    /// gate on the projected request (selector cleared, a placeholder import
    /// ref as `source_ref`), so a start the engine would refuse AFTER the
    /// import is refused before it, with the engine's code and message.
    #[test]
    fn family_merge_shape_runs_the_engine_start_gate_before_any_import() {
        let mut whitespace = merge_request(crate::MergeOp::Start);
        whitespace.message = Some("   ".to_owned());
        let error = validate_family_merge(&whitespace).unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeValidationFailed);
        assert!(
            error
                .message
                .contains("merge commit message must not be empty"),
            "{}",
            error.message
        );

        let mut partial = merge_request(crate::MergeOp::Start);
        partial.meta.policy = Some(crate::OperationPolicy {
            partial: Some(crate::PartialBehavior::Partial),
            ..crate::OperationPolicy::default()
        });
        let error = validate_family_merge(&partial).unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeValidationFailed);
        assert!(
            error
                .message
                .contains("partial merge policy is not supported"),
            "{}",
            error.message
        );

        // The gate sees the PROJECTED request: the engine's own selector
        // refusal and `source_ref` requirement do not fire on a well-formed
        // family start, with or without an explicit source ref.
        assert!(validate_family_merge(&merge_request(crate::MergeOp::Start)).is_ok());
        let mut with_ref = merge_request(crate::MergeOp::Start);
        with_ref.source_ref = Some("lane/agent-17".to_owned());
        assert!(validate_family_merge(&with_ref).is_ok());

        // Shape stays ahead of the dry-run refusal (the pinned order).
        let mut dry_and_malformed = merge_request(crate::MergeOp::Start);
        dry_and_malformed.meta.dry_run = Some(true);
        dry_and_malformed.message = Some("subject\0body".to_owned());
        assert_eq!(
            validate_family_merge(&dry_and_malformed).unwrap_err().code,
            ErrorCode::MergeValidationFailed
        );
    }
}
