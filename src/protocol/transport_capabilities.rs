use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::{TransportCapabilitiesRequest, TransportCapabilitiesResponse};

/// Capabilities of the native backend shipped with this core, without opening a repository.
#[allow(
    clippy::needless_update,
    reason = "gwz_transport_candidate adds fields"
)]
pub fn handle(request: TransportCapabilitiesRequest) -> ModelResult<TransportCapabilitiesResponse> {
    if request.schema_version != "gwz.protocol/v0" {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "unsupported protocol version for transport capabilities",
        ));
    }
    Ok(TransportCapabilitiesResponse {
        file_identity: true,
        exact_agent_identity: false,
        ..Default::default()
    })
}

pub fn configure_runtime(
    request: crate::TransportRuntimeRequest,
) -> ModelResult<crate::TransportRuntimeResponse> {
    if request.schema_version != "gwz.protocol/v0" {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "unsupported protocol version for transport runtime",
        ));
    }
    let ms = i32::try_from(request.server_timeout_ms).map_err(|_| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "transport timeout is outside the supported millisecond range",
        )
    })?;
    crate::git::configure_server_timeout_ms(ms)?;
    Ok(crate::TransportRuntimeResponse {
        server_timeout_ms: i64::from(ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_distinguish_file_from_exact_agent_support() {
        let response = handle(TransportCapabilitiesRequest {
            schema_version: "gwz.protocol/v0".to_owned(),
        })
        .unwrap();
        assert!(response.file_identity);
        assert!(!response.exact_agent_identity);
        assert_eq!(
            handle(TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/future".to_owned(),
            })
            .unwrap_err()
            .code,
            ErrorCode::UnsupportedOperation
        );
    }
    #[test]
    fn runtime_timeout_refuses_changes_after_backend_creation() {
        let _backend = crate::git::Git2Backend::without_credential_helpers();
        // Creating a backend fixed the process-wide timeout at the default:
        // no test in this binary chooses one before its first backend.
        let default = i64::from(crate::git::DEFAULT_SERVER_TIMEOUT_MS);
        let request = |milliseconds| crate::TransportRuntimeRequest {
            server_timeout_ms: milliseconds,
            schema_version: "gwz.protocol/v0".into(),
        };
        assert_eq!(
            configure_runtime(request(default))
                .unwrap()
                .server_timeout_ms,
            default
        );
        assert_eq!(
            configure_runtime(request(default + 1)).unwrap_err().code,
            ErrorCode::UnsupportedOperation
        );
        assert_eq!(
            configure_runtime(request(-1)).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            configure_runtime(request(i64::MAX)).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}
