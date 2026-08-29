use std::path::Path;

use crate::model;

use super::super::OperationRequest;

/// Dispatch a unified commit-log request into the future Phase-2 engine.
///
/// S2.0 owns only this first-class request/action seam. The later commit-log
/// engine steps replace the intentional refusal with read execution; refusing
/// here avoids returning a misleading empty history in the interim.
pub(in crate::operation) fn handle_log(
    start: &Path,
    request: crate::LogRequest,
    operation_id: impl Into<String>,
) -> model::ModelResult<crate::LogResponse> {
    let _context = OperationRequest::Log(request).context(operation_id.into())?;
    let _ = start;
    Err(model::ModelError::new(
        model::ErrorCode::UnsupportedOperation,
        "log engine is not implemented yet",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dispatch_reaches_the_future_engine_stub() {
        let request = crate::LogRequest {
            meta: crate::RequestMeta {
                request_id: "req-log-stub".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                ..crate::RequestMeta::default()
            },
            ..crate::LogRequest::default()
        };

        let error = handle_log(Path::new("."), request, "op-log-stub")
            .expect_err("S2.0 must leave execution to future log engine steps");

        assert_eq!(error.code, model::ErrorCode::UnsupportedOperation);
        assert_eq!(error.message, "log engine is not implemented yet");
    }
}
