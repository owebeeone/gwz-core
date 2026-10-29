use super::*;

impl Session {
    /// Admits a client request: its registration is checked before the mux's lifetime request ID is consumed, and its
    /// resolved limits are recorded for the request alone. Nothing is installed, resized, retired or refused for
    /// limits that differ from another request's (reuse design section 5): each request's pool requests carry its own
    /// limits, its owner is held to them, and the host's physical ceilings (the pools' `Config` and the shared
    /// authority's) bound the total as max(ceiling, the request's limit).
    pub(in crate::transport_host) fn admit_client_request(
        self: &Arc<Self>,
        request: &str,
        capacity: pool::Capacity,
    ) -> ModelResult<ClientRequest> {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("transport session is closed"));
            }
            if !request::identifier(request)
                || state.used.contains(request)
                || state.used.len() >= 256
            {
                return Err(invalid("invalid or exhausted request registration"));
            }
            if let Some(authority) = &state.authority {
                authority.admit_operation(capacity.total, capacity.per_host);
            }
            if let Some(engine) = &mut state.engine {
                engine.set_limits(request, capacity);
            }
            if let Some(endpoint) = &mut state.https {
                endpoint.set_limits(request, capacity);
            }
        }
        ClientRequest::new(self.clone(), request)
    }
}
