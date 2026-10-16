//! The driver's bookkeeping of the identity checks it has asked its endpoint to make. Only the Cli placement checks a
//! path on its endpoint (the in-process endpoint is the driver's own process), so the checks exist only in the session
//! candidate (S7.1 (1.1.0) step C3a); the 1.1.0 transport build has an empty slot that completes, cancels and counts
//! nothing.

cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows)))] {
        use super::*;

        struct Check {
            request: String,
            result: Arc<Wait<Result<(), Failure>>>,
        }

        /// The checks in flight, by the stream each one asked on.
        #[derive(Default)]
        pub(super) struct CheckSlot(BTreeMap<i64, Check>);
        impl CheckSlot {
            pub(super) fn insert(
                &mut self,
                stream_id: i64,
                request: &str,
                result: Arc<Wait<Result<(), Failure>>>,
            ) {
                self.0.insert(
                    stream_id,
                    Check {
                        request: request.into(),
                        result,
                    },
                );
            }
            /// Completes the check `message` answers, and says whether it answered one.
            pub(super) fn complete(&mut self, message: &Envelope) -> bool {
                let Some(check) = self.0.remove(&message.stream_id) else {
                    return false;
                };
                let result = match message.kind {
                    MessageKind::IdentityChecked => Ok(()),
                    MessageKind::IdentityCheckFailed => Err(message
                        .identity_check_failed
                        .clone()
                        .expect("validated check failure")),
                    _ => Err(protocol_failure(
                        gwz_transport::protocol::ErrorCode::Cancelled,
                    )),
                };
                check.result.complete(result);
                true
            }
            /// Fails every check `request` has in flight with `Cancelled`.
            pub(super) fn cancel(&self, request: &str) {
                for check in self.0.values().filter(|c| c.request == request) {
                    check.result.complete(Err(protocol_failure(
                        gwz_transport::protocol::ErrorCode::Cancelled,
                    )));
                }
            }
            /// Fails every check with `CarrierLost`, as the session closes.
            pub(super) fn fail_all(&mut self) {
                for (_, check) in std::mem::take(&mut self.0) {
                    check.result.complete(Err(protocol_failure(
                        gwz_transport::protocol::ErrorCode::CarrierLost,
                    )));
                }
            }
            pub(super) fn len(&self) -> usize {
                self.0.len()
            }
            pub(super) fn waits_on(&self, request: &str) -> bool {
                self.0.values().any(|check| check.request == request)
            }
        }
    } else {
        /// The 1.1.0 transport build asks no endpoint to check anything: its slot is always empty.
        #[derive(Default)]
        pub(super) struct CheckSlot;
        impl CheckSlot {
            pub(super) fn complete(&mut self, _message: &gwz_transport::protocol::Envelope) -> bool {
                false
            }
            pub(super) fn cancel(&self, _request: &str) {}
            pub(super) fn fail_all(&mut self) {}
            pub(super) fn len(&self) -> usize {
                0
            }
            pub(super) fn waits_on(&self, _request: &str) -> bool {
                false
            }
        }
    }
}
