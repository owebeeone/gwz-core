use super::*;

impl Session {
    fn inbound_port(&self, attachment: &Attachment) -> ModelResult<mux::Port> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport port is closed"));
        }
        if state.owner.is_none() {
            if attachment.1.kind != MessageKind::Bind
                || !state.registrations.contains_key(&attachment.0)
                || gwz_transport::codec::admit(&attachment.1).is_err()
            {
                Self::close_state(&mut state);
                return Err(invalid("invalid endpoint bootstrap"));
            }
            let config = state
                .endpoint_config
                .clone()
                .ok_or_else(|| invalid("endpoint missing"))?;
            let mux =
                Mux::endpoint(&attachment.1.session_id, config, mux_config()).map_err(mux_error)?;
            let (owner, port) = Owner::new(mux);
            owner.advance(self.origin.elapsed().as_millis() as u64);
            for (id, record) in &state.registrations {
                owner
                    .register(id, record.operation.clone())
                    .map_err(mux_error)?;
                if record.sealed.is_some() {
                    let _ = owner.cancel(id);
                }
            }
            state.session = Some(attachment.1.session_id.clone());
            state.owner = Some(owner);
            state.port = Some(port);
        }
        Ok(state.port.as_ref().expect("initialized port").clone())
    }
}
impl Drop for PortLease {
    fn drop(&mut self) {
        self.0.close();
    }
}
impl TransportPort {
    pub async fn next_message(&self) -> ModelResult<Option<Attachment>> {
        let session = &self.0.0;
        let listener = session.listener();
        let port = poll_fn(|cx| {
            if !listener.arm(cx) {
                session.close();
                return Poll::Ready(Err(unavailable("transport waiter capacity")));
            }
            let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                Poll::Ready(Ok(None))
            } else if let Some(port) = &state.port {
                Poll::Ready(Ok(Some(port.clone())))
            } else {
                Poll::Pending
            }
        })
        .await?;
        let message = match port {
            Some(port) => port.next_message().await.map_err(mux_error)?,
            None => None,
        };
        if message.is_some() {
            // A send that found the mux's queue full can go now.
            session.wake();
        }
        Ok(message)
    }
    pub async fn deliver(&self, attachment: Attachment) -> ModelResult<()> {
        let result = self
            .0
            .0
            .inbound_port(&attachment)?
            .deliver(attachment)
            .await
            .map_err(mux_error);
        if result.is_err() {
            self.0.0.close();
        }
        self.0.0.event.signal();
        self.0.0.wake();
        result
    }
    pub fn disconnect(&self) {
        self.0.0.close();
    }
}
