use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) fn validate_merge_request(request: &crate::MergeRequest) -> ModelResult<()> {
    validate_common_meta(request)?;
    validate_optional_id(request.merge_id.as_deref())?;

    match request.op {
        crate::MergeOp::Start => {
            require_source(request.source_ref.as_deref())?;
            reject_present("merge_id", request.merge_id.is_some())?;
            reject_present("preserve", request.preserve.is_some())?;
            // T-1, inverted at A1. `MergeMode::NoFf` used to return the typed
            // refusal "no_ff requires the v1 record lifecycle and is not yet
            // activated" here; the activation routes it to the v1 lifecycle
            // instead.
            //
            // COUPLED with `runtime/dispatch.rs`'s message-validation
            // exclusion (Safety review §2.2 R2). While NoFf refused here, the
            // dispatch skipped `validate_custom_commit_message` for NoFf
            // starts because they could never reach record creation. Landing
            // this refusal's fall WITHOUT that exclusion's fall would let a
            // NoFf start carry an unvalidated custom message into record
            // creation — the v1 forward path consumes `row.commit_message`
            // from the record and performs no request-message validation of
            // its own. The two are one gate and move together.
            if let Some(message) = request.message.as_deref() {
                super::integration::validate_custom_commit_message(message)?;
            }
        }
        crate::MergeOp::Resume => {
            reject_recovery_fields(request)?;
        }
        crate::MergeOp::Abort => {
            reject_present("source_ref", request.source_ref.is_some())?;
            reject_present("mode", request.mode.is_some())?;
            reject_present("message", request.message.is_some())?;
        }
        crate::MergeOp::Status => {
            reject_recovery_fields(request)?;
        }
        crate::MergeOp::Gc => {
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
        reject_policy_field("policy.sync (--sync)", policy.sync.is_some())?;
        reject_policy_field("policy.remote (--remote)", policy.remote.is_some())?;
        reject_policy_field("policy.concurrency (--jobs)", policy.concurrency.is_some())?;
        reject_policy_field(
            "policy.progress_min_interval_ms (--progress-interval)",
            policy.progress_min_interval_ms.is_some(),
        )?;
        reject_policy_field(
            "policy.max_connections_per_host (--max-per-host)",
            policy.max_connections_per_host.is_some(),
        )?;
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

fn reject_policy_field(field: &str, present: bool) -> ModelResult<()> {
    if present {
        return invalid(format!("merge does not accept {field}"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> ModelResult<T> {
    Err(ModelError::new(ErrorCode::MergeValidationFailed, message))
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
        abort.preserve = Some(true);
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

    /// T-1, inverted at A1. This is M5b's designed inversion marker: while
    /// the v1 record lifecycle was a compile boundary, `MergeMode::NoFf`
    /// returned `MergePhaseUnsupported` with "no_ff requires the v1 record
    /// lifecycle and is not yet activated", and a NoFf start's custom message
    /// went unvalidated because the start could never reach record creation.
    /// The activation landed both halves of that gate together, so no-ff now
    /// validates exactly as every other start does.
    #[test]
    fn custom_messages_and_no_ff_both_validate_after_activation() {
        let mut message = request(crate::MergeOp::Start);
        message.message = Some("custom".to_owned());
        assert!(validate_merge_request(&message).is_ok());

        for body in ["", " \t\n", "\u{2003}\r\n", "subject\0body"] {
            message.message = Some(body.to_owned());
            assert_eq!(
                validate_merge_request(&message).unwrap_err().code,
                ErrorCode::MergeValidationFailed
            );
        }

        let mut ff_only = request(crate::MergeOp::Start);
        ff_only.mode = Some(crate::MergeMode::FfOnly);
        assert!(validate_merge_request(&ff_only).is_ok());

        let mut no_ff = request(crate::MergeOp::Start);
        no_ff.mode = Some(crate::MergeMode::NoFf);
        assert!(validate_merge_request(&no_ff).is_ok());
        for body in ["", " \t\n", "\u{2003}\r\n", "subject\0body"] {
            no_ff.message = Some(body.to_owned());
            assert_eq!(
                validate_merge_request(&no_ff).unwrap_err().code,
                ErrorCode::MergeValidationFailed,
                "the coupled pair validates no-ff custom messages too"
            );
        }
        no_ff.message = None;

        let mut archived_status = request(crate::MergeOp::Status);
        archived_status.merge_id = Some("merge_1".to_owned());
        assert!(validate_merge_request(&archived_status).is_ok());

        let mut root = request(crate::MergeOp::Start);
        root.meta.selection = Some(crate::Selection {
            targets: vec!["@root".to_owned()],
            ..crate::Selection::default()
        });
        assert!(validate_merge_request(&root).is_ok());
        root.meta.selection.as_mut().unwrap().exclude_targets = vec!["@root".to_owned()];
        assert!(validate_merge_request(&root).is_ok());

        assert!(validate_open_merge_id(None, "merge_1").is_ok());
        assert!(validate_open_merge_id(Some("merge_1"), "merge_1").is_ok());
        assert_eq!(
            validate_open_merge_id(Some("merge_old"), "merge_1")
                .unwrap_err()
                .code,
            ErrorCode::MergeIdMismatch
        );
    }

    #[test]
    fn unrelated_policy_errors_name_the_field_and_cli_option() {
        let cases = [
            ("sync", "--sync"),
            ("remote", "--remote"),
            ("concurrency", "--jobs"),
            ("progress_min_interval_ms", "--progress-interval"),
            ("max_connections_per_host", "--max-per-host"),
        ];

        for (field, option) in cases {
            let mut value = request(crate::MergeOp::Start);
            let mut policy = crate::OperationPolicy::default();
            match field {
                "sync" => policy.sync = Some(crate::SyncBehavior::Merge),
                "remote" => policy.remote = Some("origin".to_owned()),
                "concurrency" => policy.concurrency = Some(4),
                "progress_min_interval_ms" => policy.progress_min_interval_ms = Some(250),
                "max_connections_per_host" => policy.max_connections_per_host = Some(8),
                _ => unreachable!(),
            }
            value.meta.policy = Some(policy);

            let error = validate_merge_request(&value).unwrap_err();
            assert_eq!(error.code, ErrorCode::MergeValidationFailed, "{field}");
            assert!(error.message.contains(field), "{field}: {}", error.message);
            assert!(error.message.contains(option), "{field}: {}", error.message);
        }
    }
}
