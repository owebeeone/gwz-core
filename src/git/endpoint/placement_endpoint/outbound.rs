use super::*;

impl PlacementEndpoint {
    pub(crate) fn take_outbound(&mut self) -> Option<Outbound> {
        let item = self.outbound.pop_front();
        if let Some(next) = self.terminal_outbound.pop_front() {
            self.outbound.push_back(next);
        }
        if let Some(item) = &item {
            if matches!(
                item.envelope.kind,
                MessageKind::OpenFailed
                    | MessageKind::IdentityChecked
                    | MessageKind::IdentityCheckFailed
                    | MessageKind::Failed
                    | MessageKind::Closed
            ) {
                let key = (item.request.clone(), item.envelope.stream_id);
                self.requests.remove(&key);
            }
        }
        item
    }
    pub(super) fn push_outbound(&mut self, request: String, message: Envelope) {
        let item = Outbound {
            request,
            envelope: message,
        };
        if self.outbound.len() < MAX_OUTBOUND {
            self.outbound.push_back(item);
        } else if self.terminal_outbound.len() < 2 * MAX_REQUESTS {
            // One Opened plus one terminal can be retained per admitted route.
            // Bulk data uses only `outbound`; it cannot consume this reserve.
            self.terminal_outbound.push_back(item);
        } else {
            // An exhausted internal bound is a binding failure, never a silent
            // successful send or a request left waiting for a dropped result.
            self.faulted = true;
        }
    }
}

pub(super) fn envelope_for(
    state: &Request,
    kind: MessageKind,
    failure: Option<Failure>,
    opened: Option<Opened>,
) -> Envelope {
    Envelope {
        version: state.version,
        session_id: state.session_id.clone(),
        stream_id: state.stream_id,
        kind,
        open_failed: (kind == MessageKind::OpenFailed)
            .then_some(failure.clone())
            .flatten(),
        identity_check_failed: (kind == MessageKind::IdentityCheckFailed)
            .then_some(failure.clone())
            .flatten(),
        failed: (kind == MessageKind::Failed).then_some(failure).flatten(),
        opened,
        identity_checked: (kind == MessageKind::IdentityChecked)
            .then_some(gwz_transport::protocol::IdentityChecked {}),
        ..Default::default()
    }
}
