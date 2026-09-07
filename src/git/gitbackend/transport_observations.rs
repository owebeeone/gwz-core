use std::path::Path;
use std::sync::{Arc, Mutex};

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
pub(crate) struct TransportAttempt(Arc<Mutex<crate::TransportObservation>>);

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
        };
        let attempt = TransportAttempt(Arc::new(Mutex::new(row)));
        self.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(attempt.clone());
        attempt
    }
}

impl TransportAttempt {
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
        if row.credential_offered {
            row.authenticated = Some(true);
        }
    }
}

#[cfg(test)]
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
            !format!("{:?}", observations.snapshot()).contains("private-key-must-not-be-reported")
        );
        assert!(TransportObservations::default().snapshot().is_empty());
    }
}
