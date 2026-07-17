use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) fn validate_merge_request(request: &crate::MergeRequest) -> ModelResult<()> {
    validate_common_meta(request)?;
    validate_optional_id(request.merge_id.as_deref())?;

    match request.op {
        crate::MergeOp::Start => {
            require_source(request.source_ref.as_deref())?;
            reject_present("merge_id", request.merge_id.is_some())?;
            reject_present("preserve", request.preserve.is_some())?;
            if matches!(
                request.mode,
                Some(crate::MergeMode::FfOnly | crate::MergeMode::NoFf)
            ) {
                return phase("the requested merge mode is reserved for M4");
            }
            if request.message.is_some() {
                return phase("custom merge messages are reserved for M4");
            }
            if selects_root(request) {
                return Err(ModelError::new(
                    ErrorCode::RootMergeNotYetSupported,
                    "explicit @root merge participation is reserved for M2c",
                ));
            }
        }
        crate::MergeOp::Resume => {
            reject_recovery_fields(request)?;
        }
        crate::MergeOp::Abort => {
            reject_present("source_ref", request.source_ref.is_some())?;
            reject_present("mode", request.mode.is_some())?;
            reject_present("message", request.message.is_some())?;
            if request.preserve == Some(true) {
                return phase("preserve-abort is reserved for M3");
            }
        }
        crate::MergeOp::Status | crate::MergeOp::Gc => {
            reject_recovery_fields(request)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_open_merge_id(requested: Option<&str>, open_id: &str) -> ModelResult<()> {
    if requested.is_some_and(|requested| requested != open_id) {
        return Err(ModelError::new(
            ErrorCode::MergeIdMismatch,
            format!("requested merge does not match the open merge '{open_id}'"),
        ));
    }
    Ok(())
}

fn validate_common_meta(request: &crate::MergeRequest) -> ModelResult<()> {
    if request.op != crate::MergeOp::Start && request.meta.dry_run == Some(true) {
        return invalid("dry_run is accepted only for merge start");
    }
    if let Some(policy) = &request.meta.policy {
        if policy.partial == Some(crate::PartialBehavior::Partial) {
            return invalid("partial merge policy is not supported");
        }
        if policy.destructive == Some(crate::DestructiveBehavior::Allow) {
            return invalid("merge does not support a force/destructive policy");
        }
        if policy.unsupported_member == Some(crate::UnsupportedMemberBehavior::Skip) {
            return invalid("merge does not support skipping selected participants");
        }
        if policy.sync.is_some()
            || policy.remote.is_some()
            || policy.concurrency.is_some()
            || policy.progress_min_interval_ms.is_some()
            || policy.max_connections_per_host.is_some()
        {
            return invalid("merge request contains an unrelated operation policy field");
        }
    }
    Ok(())
}

fn validate_optional_id(merge_id: Option<&str>) -> ModelResult<()> {
    if merge_id.is_some_and(|value| value.trim().is_empty()) {
        return invalid("merge_id must not be empty when supplied");
    }
    Ok(())
}

fn require_source(source: Option<&str>) -> ModelResult<()> {
    if source.is_none_or(|value| value.trim().is_empty()) {
        return invalid("source_ref is required for merge start");
    }
    Ok(())
}

fn reject_recovery_fields(request: &crate::MergeRequest) -> ModelResult<()> {
    reject_present("source_ref", request.source_ref.is_some())?;
    reject_present("mode", request.mode.is_some())?;
    reject_present("message", request.message.is_some())?;
    reject_present("preserve", request.preserve.is_some())
}

fn reject_present(field: &str, present: bool) -> ModelResult<()> {
    if present {
        return invalid(format!("{field} is not accepted for this merge operation"));
    }
    Ok(())
}

fn selects_root(request: &crate::MergeRequest) -> bool {
    request
        .meta
        .selection
        .as_ref()
        .is_some_and(|selection| selection.targets.iter().any(|target| target == "@root"))
}

fn invalid<T>(message: impl Into<String>) -> ModelResult<T> {
    Err(ModelError::new(ErrorCode::MergeValidationFailed, message))
}

fn phase<T>(message: impl Into<String>) -> ModelResult<T> {
    Err(ModelError::new(ErrorCode::MergePhaseUnsupported, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(op: crate::MergeOp) -> crate::MergeRequest {
        crate::MergeRequest {
            meta: crate::RequestMeta {
                request_id: "req".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                ..crate::RequestMeta::default()
            },
            op,
            source_ref: (op == crate::MergeOp::Start).then(|| "feature/x".to_owned()),
            merge_id: None,
            mode: None,
            message: None,
            preserve: None,
        }
    }

    #[test]
    fn accepted_field_matrix_covers_every_operation() {
        for op in [
            crate::MergeOp::Start,
            crate::MergeOp::Resume,
            crate::MergeOp::Abort,
            crate::MergeOp::Status,
            crate::MergeOp::Gc,
        ] {
            assert!(validate_merge_request(&request(op)).is_ok(), "{op:?}");
        }
        let mut with_id = request(crate::MergeOp::Resume);
        with_id.merge_id = Some("merge_1".to_owned());
        assert!(validate_merge_request(&with_id).is_ok());
        let mut normal = request(crate::MergeOp::Start);
        normal.mode = Some(crate::MergeMode::Normal);
        assert!(validate_merge_request(&normal).is_ok());
        let mut abort = request(crate::MergeOp::Abort);
        abort.preserve = Some(false);
        assert!(validate_merge_request(&abort).is_ok());
    }

    #[test]
    fn rejected_field_matrix_is_core_owned() {
        let cases = [
            (crate::MergeOp::Start, "merge_id"),
            (crate::MergeOp::Start, "preserve"),
            (crate::MergeOp::Resume, "source_ref"),
            (crate::MergeOp::Resume, "mode"),
            (crate::MergeOp::Resume, "message"),
            (crate::MergeOp::Resume, "preserve"),
            (crate::MergeOp::Abort, "source_ref"),
            (crate::MergeOp::Abort, "mode"),
            (crate::MergeOp::Abort, "message"),
            (crate::MergeOp::Status, "source_ref"),
            (crate::MergeOp::Status, "mode"),
            (crate::MergeOp::Status, "message"),
            (crate::MergeOp::Status, "preserve"),
            (crate::MergeOp::Gc, "source_ref"),
            (crate::MergeOp::Gc, "mode"),
            (crate::MergeOp::Gc, "message"),
            (crate::MergeOp::Gc, "preserve"),
        ];
        for (op, field) in cases {
            let mut value = request(op);
            match field {
                "merge_id" => value.merge_id = Some("merge_1".to_owned()),
                "source_ref" => value.source_ref = Some("feature/x".to_owned()),
                "mode" => value.mode = Some(crate::MergeMode::Normal),
                "message" => value.message = Some("message".to_owned()),
                "preserve" => value.preserve = Some(false),
                _ => unreachable!(),
            }
            assert_eq!(
                validate_merge_request(&value).unwrap_err().code,
                ErrorCode::MergeValidationFailed,
                "{op:?}.{field}"
            );
        }
    }

    #[test]
    fn reserved_features_and_root_return_specific_typed_errors() {
        let mut message = request(crate::MergeOp::Start);
        message.message = Some("custom".to_owned());
        assert_eq!(
            validate_merge_request(&message).unwrap_err().code,
            ErrorCode::MergePhaseUnsupported
        );

        let mut mode = request(crate::MergeOp::Start);
        mode.mode = Some(crate::MergeMode::FfOnly);
        assert_eq!(
            validate_merge_request(&mode).unwrap_err().code,
            ErrorCode::MergePhaseUnsupported
        );

        let mut preserve = request(crate::MergeOp::Abort);
        preserve.preserve = Some(true);
        assert_eq!(
            validate_merge_request(&preserve).unwrap_err().code,
            ErrorCode::MergePhaseUnsupported
        );

        let mut root = request(crate::MergeOp::Start);
        root.meta.selection = Some(crate::Selection {
            targets: vec!["@root".to_owned()],
            ..crate::Selection::default()
        });
        assert_eq!(
            validate_merge_request(&root).unwrap_err().code,
            ErrorCode::RootMergeNotYetSupported
        );

        assert!(validate_open_merge_id(None, "merge_1").is_ok());
        assert!(validate_open_merge_id(Some("merge_1"), "merge_1").is_ok());
        assert_eq!(
            validate_open_merge_id(Some("merge_old"), "merge_1")
                .unwrap_err()
                .code,
            ErrorCode::MergeIdMismatch
        );
    }
}
