use super::*;

impl Session {
    /// Admits a client request: it is registered first (a refused or duplicate registration changes nothing), then its
    /// resolved limits are recorded for the request alone. Nothing is installed, resized, retired or refused for
    /// limits that differ from another request's (reuse design section 5): each request's pool requests carry its own
    /// limits, its owner is held to them, and the host's physical ceilings bound the total as max(ceiling, the
    /// limits of the requests that are live).
    pub(in crate::transport_host) fn admit_client_request(
        self: &Arc<Self>,
        request: &str,
        capacity: pool::Capacity,
    ) -> ModelResult<ClientRequest> {
        let client = ClientRequest::new(self.clone(), request)?;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // A request sealed between its registration and here (the session closing, a peer's cancel) has ended
            // already: nothing is recorded for it, since nothing would end the record.
            let live = state
                .registrations
                .get(request)
                .is_some_and(|record| record.sealed.is_none());
            if live {
                if let Some(authority) = &state.authority {
                    authority.admit_operation(request, capacity.total, capacity.per_host);
                }
                if let Some(engine) = &mut state.engine {
                    engine.set_limits(request, capacity);
                }
                if let Some(endpoint) = &mut state.https {
                    endpoint.set_limits(request, capacity);
                }
            }
        }
        Ok(client)
    }
}
