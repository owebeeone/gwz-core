use super::*;

impl PlacementEndpoint {
    pub(super) fn flush_attachments(&mut self) {
        let requests: Vec<_> = self.requests.keys().cloned().collect();
        for request in requests {
            if self.outbound.len() >= MAX_OUTBOUND {
                break;
            }
            let Some(state) = self.requests.get_mut(&request) else {
                continue;
            };
            // A terminal request has queued its last message and stays only
            // until that message is taken. Its worker drops the bridge after
            // the terminal, so reading on would queue a false Failed each pass.
            if state.terminal {
                continue;
            }
            let Some(attachment) = state.attachment.as_ref() else {
                continue;
            };
            let mut disconnect_failure = None;
            while let Some(message) = state.queued_input.pop_front() {
                match attachment.send(message.clone()) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        state.queued_input.push_front(message);
                        break;
                    }
                    Err(_) => {
                        state.terminal = true;
                        disconnect_failure = Some(envelope_for(
                            state,
                            MessageKind::Failed,
                            Some(Failure {
                                detail: None,
                                setup_cause: None,
                                code: ErrorCode::Io,
                                effect: Effect::Possible,
                                facts: None,
                            }),
                            None,
                        ));
                        break;
                    }
                }
            }
            for _ in 0..8 {
                match attachment.try_receive() {
                    Ok(Some(mut message)) => {
                        message.session_id = state.session_id.clone();
                        message.stream_id = state.stream_id;
                        message.version = state.version;
                        if matches!(message.kind, MessageKind::Closed | MessageKind::Failed) {
                            state.terminal = true;
                        }
                        self.outbound.push_back(Outbound {
                            request: request.0.clone(),
                            envelope: message,
                        });
                        if state.terminal {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        state.terminal = true;
                        disconnect_failure = Some(envelope_for(
                            state,
                            MessageKind::Failed,
                            Some(Failure {
                                detail: None,
                                setup_cause: None,
                                code: ErrorCode::Io,
                                effect: Effect::Possible,
                                facts: None,
                            }),
                            None,
                        ));
                        break;
                    }
                }
                if self.outbound.len() >= MAX_OUTBOUND {
                    break;
                }
            }
            if let Some(message) = disconnect_failure {
                self.push_outbound(request.0.clone(), message);
            }
        }
    }
}
