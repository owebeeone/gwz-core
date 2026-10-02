// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_transport {
    () => {
    /// Bind invocation credentials to a new backend value. The default refuses
    /// explicit authority rather than silently ignoring it. No global state.
    fn with_transport(
        &self,
        _start: &Path,
        options: Option<&crate::TransportOptions>,
    ) -> ModelResult<Option<Self>>
    where
        Self: Sized,
    {
        if super::transport_support::identity::has_options(options) {
            return Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                "this Git backend does not support explicit SSH identity selection",
            ));
        }
        Ok(None)
    }

    /// Validate the request-scoped receiver/operation binding before transport
    /// work or workspace mutation. Candidate hosts provide the installed
    /// request context; the native backend keeps the no-op default.
    fn validate_transport_scope(
        &self,
        _meta: &crate::RequestMeta,
        _operation_id: &str,
    ) -> ModelResult<()> {
        Ok(())
    }

    fn transport_observations(
        &self,
    ) -> Option<super::transport_observations::TransportObservations> {
        None
    }

    fn validate_transport_remotes(&self, _names: &[String]) -> ModelResult<()> {
        Ok(())
    }
    fn remote_identity(&self, _path: &Path, _remote: &str) -> ModelResult<Option<String>> {
        unsupported_backend("remote_identity")
    }
    fn set_remote_identity(
        &self,
        _path: &Path,
        _remote: &str,
        _value: Option<&str>,
    ) -> ModelResult<()> {
        unsupported_backend("set_remote_identity")
    }

    /// Local-only credential preflight; this does not establish write access.
    fn validate_remote_identity(
        &self,
        _path: &Path,
        _remote: &str,
        _push: bool,
    ) -> ModelResult<()> {
        Ok(())
    }
    fn validate_url_identity(
        &self,
        _identity_repo: Option<&Path>,
        _remote: &str,
        _url: &str,
    ) -> ModelResult<()> {
        Ok(())
    }
    /// Inspect a committed file using temporary native storage, without creating
    /// the caller's destination checkout or persisting a remote in it.
    fn read_remote_file(
        &self,
        _url: &str,
        _remote: &str,
        _relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        unsupported_backend("read_remote_file")
    }

    };
}
