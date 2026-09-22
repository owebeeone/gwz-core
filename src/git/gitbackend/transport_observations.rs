use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::transport_support::identity::{SelectedIdentity, Source};

/// Observations belong to one operation; snapshots contain public metadata only.
#[derive(Clone, Debug, Default)]
pub struct TransportObservations {
    rows: Arc<Mutex<Vec<TransportAttempt>>>,
}

impl PartialEq for TransportObservations {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot() == other.snapshot()
    }
}
impl Eq for TransportObservations {}

#[derive(Clone, Debug)]
pub(crate) struct TransportAttempt(Arc<Mutex<crate::TransportObservation>>, Arc<AtomicBool>);

impl TransportObservations {
    pub fn snapshot(&self) -> Vec<crate::TransportObservation> {
        self.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|attempt| {
                attempt
                    .0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
            })
            .collect()
    }

    /// An explicitly private member's refused fresh clone is intentionally quiet.
    pub(crate) fn forget_private_clone(&self, path: &Path) {
        let path = path.to_string_lossy();
        self.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|attempt| {
                let row = attempt.0.lock().unwrap_or_else(|error| error.into_inner());
                row.operation != crate::TransportOperation::Clone || row.repository_path != path
            });
    }

    pub(crate) fn begin(
        &self,
        path: &Path,
        remote: &str,
        operation: crate::TransportOperation,
        identity: Option<&SelectedIdentity>,
    ) -> TransportAttempt {
        let selection_source = match identity.map(|identity| identity.source) {
            Some(Source::InvocationRemote) => crate::TransportSelectionSource::InvocationRemote,
            Some(Source::InvocationDefault) => crate::TransportSelectionSource::InvocationDefault,
            Some(Source::LocalConfiguration) => crate::TransportSelectionSource::LocalConfiguration,
            None => crate::TransportSelectionSource::Ambient,
        };
        #[allow(unused_mut)]
        let row = crate::TransportObservation {
            repository_path: path.to_string_lossy().into_owned(),
            remote: remote.into(),
            operation,
            credential_method: if identity.is_some() {
                crate::TransportCredentialMethod::File
            } else {
                crate::TransportCredentialMethod::Unknown
            },
            selection_source,
            credential_offered: false,
            authenticated: None,
            public_key_fingerprint: None,
            ..Default::default()
        };
        let attempt = TransportAttempt(Arc::new(Mutex::new(row)), Arc::new(AtomicBool::new(false)));
        self.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(attempt.clone());
        attempt
    }
}

impl TransportAttempt {
    cfg_if::cfg_if! {
        if #[cfg(all(unix, gwz_transport_candidate))] {
            pub(crate) fn opened(&self, stream_id: i64, opened: &gwz_transport::protocol::Opened) {
                let mut row = self.0.lock().unwrap_or_else(|e| e.into_inner());
                row.endpoint_id = Some(opened.endpoint_id.clone());
                row.connection_id = Some(opened.connection_id.clone());
                row.stream_id = Some(stream_id);
                row.reused = Some(opened.reused);
            }
            pub(crate) fn facts(&self, facts: &gwz_transport::protocol::Facts) {
                self.1.store(true, Ordering::Release);
                let mut row = self.0.lock().unwrap_or_else(|e| e.into_inner());
                row.credential_method = match facts.method {
                    gwz_transport::protocol::AuthMethod::SshKey => crate::TransportCredentialMethod::File,
                    gwz_transport::protocol::AuthMethod::SshAgent => crate::TransportCredentialMethod::Agent,
                    gwz_transport::protocol::AuthMethod::Gh => crate::TransportCredentialMethod::Helper,
                    _ => row.credential_method,
                };
                row.credential_offered |= facts.credential_offered;
                if facts.authenticated.is_some() { row.authenticated = facts.authenticated; }
                if facts.key_fingerprint.is_some() { row.public_key_fingerprint = facts.key_fingerprint.clone(); }
            }
        }
    }
    pub(crate) fn offered(&self, method: crate::TransportCredentialMethod) {
        let mut row = self.0.lock().unwrap_or_else(|error| error.into_inner());
        row.credential_method = method;
        row.credential_offered = true;
    }
    pub(crate) fn rejected(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .authenticated = Some(false);
    }
    pub(crate) fn succeeded(&self) {
        let mut row = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if row.credential_offered && !self.1.load(Ordering::Acquire) {
            row.authenticated = Some(true);
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            #[test]
            fn selecting_or_offering_a_key_is_not_authentication_proof() {
                let observations = TransportObservations::default();
                let identity = SelectedIdentity {
                    path: "private-key-must-not-be-reported".into(),
                    source: Source::InvocationDefault,
                };
                let attempt = observations.begin(
                    Path::new("repo"),
                    "origin",
                    crate::TransportOperation::Push,
                    Some(&identity),
                );
                assert_eq!(observations.snapshot()[0].authenticated, None);
                attempt.offered(crate::TransportCredentialMethod::File);
                assert_eq!(observations.snapshot()[0].authenticated, None);
                attempt.succeeded();
                assert_eq!(observations.snapshot()[0].authenticated, Some(true));
                let refused = observations.begin(
                    Path::new("repo"),
                    "other",
                    crate::TransportOperation::Fetch,
                    Some(&identity),
                );
                refused.offered(crate::TransportCredentialMethod::File);
                refused.rejected();
                assert_eq!(observations.snapshot()[1].authenticated, Some(false));
                assert!(
                    !format!("{:?}", observations.snapshot())
                        .contains("private-key-must-not-be-reported")
                );
                assert!(TransportObservations::default().snapshot().is_empty());
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(all(test, unix, gwz_transport_candidate))] {
        mod https_tests {
            use super::*;

            fn opened() -> gwz_transport::protocol::Opened {
                gwz_transport::protocol::Opened {
                    endpoint_id: "placement-123".into(),
                    ..Default::default()
                }
            }

            #[test]
            fn gh_facts_are_helper_metadata_without_proving_https_authentication() {
                let observations = TransportObservations::default();
                let attempt = observations.begin(
                    Path::new("repo"),
                    "https-origin",
                    crate::TransportOperation::Fetch,
                    None,
                );
                attempt.opened(7, &opened());
                attempt.facts(&gwz_transport::protocol::Facts {
                    method: gwz_transport::protocol::AuthMethod::Gh,
                    credential_offered: true,
                    ..Default::default()
                });
                attempt.succeeded();
                let row = &observations.snapshot()[0];
                assert_eq!(row.credential_method, crate::TransportCredentialMethod::Helper);
                assert!(row.credential_offered);
                assert_eq!(row.authenticated, None);
            }

            #[test]
            fn reused_gh_facts_keep_https_authentication_unknown() {
                let observations = TransportObservations::default();
                let attempt = observations.begin(
                    Path::new("repo"),
                    "https-origin",
                    crate::TransportOperation::Push,
                    None,
                );
                let opened = gwz_transport::protocol::Opened {
                    endpoint_id: "placement-123".into(),
                    reused: true,
                    ..Default::default()
                };
                attempt.opened(8, &opened);
                attempt.facts(&gwz_transport::protocol::Facts {
                    method: gwz_transport::protocol::AuthMethod::Gh,
                    credential_offered: true,
                    ..Default::default()
                });
                attempt.succeeded();
                let row = &observations.snapshot()[0];
                assert_eq!(row.credential_method, crate::TransportCredentialMethod::Helper);
                assert_eq!(row.authenticated, None);
            }
        }
    }
}
