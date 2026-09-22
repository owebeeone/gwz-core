//! Private H1 composition over the existing mux: real preparation happens only
//! after Open admission; its receipt crosses the mux before a stream is exposed.
use super::{
    https_connection::failure,
    https_destination::Destination as HttpsDestination,
    https_policy,
    https_worker::{Budget, Client, Input, Prepared},
};
use gwz_transport::{
    binding::{self, EndpointConfig},
    mux::{Config, Error, Mux},
    protocol::*,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct OpeningSession {
    request: String,
    session_id: String,
    stream_id: i64,
    open: Open,
    initiator: Mux,
    endpoint: Mux,
    receipts: Vec<Envelope>,
}
pub(crate) enum Outcome {
    Ready {
        prepared: Prepared,
        receipt: Envelope,
        first_failure: Option<Failure>,
    },
    Failed {
        failure: Failure,
        receipt: Envelope,
        first_failure: Option<Failure>,
    },
    /// Local mux admission/delivery failed; no peer receipt is claimed.
    Rejected(Failure),
}
pub(crate) fn open_for(client: &Client, input: &Input) -> Result<Open, Failure> {
    let destination = HttpsDestination::parse(&input.destination).map_err(failure)?;
    Ok(Open {
        endpoint_id: "https-endpoint".into(),
        operation_id: input.operation.clone(),
        destination: Destination {
            scheme: Scheme::Https,
            host: destination.host().into(),
            port: destination.port() as i64,
            path: destination.url.path().into(),
            ssh_username: None,
        },
        service: input.service,
        identity: Identity {
            mode: if input.policy == AuthPolicy::Gh {
                IdentityMode::Ambient
            } else {
                IdentityMode::CredentialsDisabled
            },
            ..Default::default()
        },
        policy: input.policy,
        deadlines: client.configured_deadlines(),
        receive_limits: binding::default_limits(),
    })
}
impl OpeningSession {
    pub(crate) fn new(
        session_id: String,
        request: String,
        open: Open,
        endpoint_id: String,
        trust_owner: String,
    ) -> Result<Self, Failure> {
        let config = Config::default();
        let mut initiator = Mux::initiator(&session_id, config.clone()).map_err(mux_failure)?;
        let mut endpoint = Mux::endpoint(
            &session_id,
            EndpointConfig {
                endpoint_id,
                role: EndpointRole::Driver,
                schemes: vec![Scheme::Https],
                policies: vec![AuthPolicy::Anonymous, AuthPolicy::Gh],
                limits: binding::default_limits(),
                trust_owner,
            },
            config,
        )
        .map_err(mux_failure)?;
        initiator
            .register(&request, Some(open.operation_id.clone()))
            .map_err(mux_failure)?;
        endpoint.register(&request, None).map_err(mux_failure)?;
        initiator.begin(&request).map_err(mux_failure)?;
        endpoint
            .receive(
                &initiator
                    .next_message()
                    .ok_or_else(|| failure(ErrorCode::Protocol))?,
            )
            .map_err(mux_failure)?;
        initiator
            .receive(
                &endpoint
                    .next_message()
                    .ok_or_else(|| failure(ErrorCode::Protocol))?,
            )
            .map_err(mux_failure)?;
        let mut session = Self {
            request,
            session_id,
            stream_id: 0,
            open: open.clone(),
            initiator,
            endpoint,
            receipts: Vec::with_capacity(2),
        };
        session.admit_open(open)?;
        Ok(session)
    }
    fn admit_open(&mut self, open: Open) -> Result<(), Failure> {
        self.stream_id = self
            .initiator
            .open(&self.request, open.clone())
            .map_err(mux_failure)?;
        self.endpoint
            .receive(
                &self
                    .initiator
                    .next_message()
                    .ok_or_else(|| failure(ErrorCode::Protocol))?,
            )
            .map_err(mux_failure)?;
        let action = self
            .endpoint
            .next_action()
            .ok_or_else(|| failure(ErrorCode::Protocol))?
            .1;
        if action.kind != MessageKind::Open || action.open.as_ref() != Some(&open) {
            return Err(failure(ErrorCode::Protocol));
        }
        self.open = open;
        Ok(())
    }
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }
    pub(crate) fn stream_id(&self) -> i64 {
        self.stream_id
    }
    pub(crate) fn receipts(&self) -> &[Envelope] {
        &self.receipts
    }
    pub(crate) async fn prepare(
        &mut self,
        client: &Client,
        input: Input,
        cancel: &CancellationToken,
    ) -> Outcome {
        let mut budget = client.budget_for_open(&self.open.deadlines);
        self.attempt(client, input, cancel, &mut budget).await
    }
    async fn attempt(
        &mut self,
        client: &Client,
        mut input: Input,
        cancel: &CancellationToken,
        budget: &mut Budget,
    ) -> Outcome {
        let expected = match open_for(client, &input) {
            Ok(open) => open,
            Err(error) => return Outcome::Rejected(error),
        };
        if expected.destination != self.open.destination
            || input.operation != self.open.operation_id
            || input.service != self.open.service
            || input.policy != self.open.policy
        {
            return Outcome::Rejected(failure(ErrorCode::InvalidRequest));
        }
        // Pool ownership uses the same session as the admitted message.
        input.session = self.session_id.clone();
        let result = client.prepare_budget(input, cancel, budget).await;
        let mut message = match self.endpoint.message(
            self.stream_id,
            if result.is_ok() {
                MessageKind::Opened
            } else {
                MessageKind::OpenFailed
            },
        ) {
            Ok(message) => message,
            Err(error) => return Outcome::Rejected(mux_failure(error)),
        };
        match &result {
            Ok(prepared) => message.opened = Some(prepared.opened.clone()),
            Err(error) => message.open_failed = Some(error.clone()),
        }
        let receipt = match self.route_endpoint(message) {
            Ok(receipt) => receipt,
            Err(error) => return Outcome::Rejected(error),
        };
        self.receipts.push(receipt.clone());
        match result {
            Ok(prepared) => Outcome::Ready {
                prepared,
                receipt,
                first_failure: None,
            },
            Err(failure) => Outcome::Failed {
                failure,
                receipt,
                first_failure: None,
            },
        }
    }
    pub(crate) async fn prepare_automatic(
        &mut self,
        client: &Client,
        first_input: Input,
        gh_open: Open,
        gh_input: Input,
        cancel: &CancellationToken,
    ) -> Outcome {
        let mut budget = client.budget_for_open(&self.open.deadlines);
        let first = self
            .attempt(client, first_input.clone(), cancel, &mut budget)
            .await;
        let first_failure = match first {
            Outcome::Failed { failure, .. }
                if first_input.policy == AuthPolicy::Anonymous
                    && https_policy::advertisement(first_input.service)
                    && matches!(
                        failure.code,
                        ErrorCode::Authentication | ErrorCode::RepositoryRefused
                    )
                    && failure
                        .facts
                        .as_ref()
                        .is_some_and(|facts| matches!(facts.http_status, Some(401 | 404))) =>
            {
                failure
            }
            other => return other,
        };
        if gh_open.policy != AuthPolicy::Gh || gh_input.policy != AuthPolicy::Gh {
            return Outcome::Rejected(failure(ErrorCode::InvalidRequest));
        }
        if let Err(error) = self.admit_open(gh_open) {
            return Outcome::Rejected(error);
        }
        match self.attempt(client, gh_input, cancel, &mut budget).await {
            Outcome::Ready {
                prepared, receipt, ..
            } => Outcome::Ready {
                prepared,
                receipt,
                first_failure: Some(first_failure),
            },
            Outcome::Failed {
                failure, receipt, ..
            } => Outcome::Failed {
                failure,
                receipt,
                first_failure: Some(first_failure),
            },
            other => other,
        }
    }
    pub(crate) fn route_endpoint(&mut self, message: Envelope) -> Result<Envelope, Failure> {
        self.endpoint
            .send(&self.request, &message)
            .map_err(mux_failure)?;
        self.initiator
            .receive(
                &self
                    .endpoint
                    .next_message()
                    .ok_or_else(|| failure(ErrorCode::Protocol))?,
            )
            .map_err(mux_failure)?;
        self.initiator
            .next_action()
            .map(|(_, message)| message)
            .ok_or_else(|| failure(ErrorCode::Protocol))
    }
    pub(crate) fn route_initiator(&mut self, message: Envelope) -> Result<Envelope, Failure> {
        self.initiator
            .send(&self.request, &message)
            .map_err(mux_failure)?;
        self.endpoint
            .receive(
                &self
                    .initiator
                    .next_message()
                    .ok_or_else(|| failure(ErrorCode::Protocol))?,
            )
            .map_err(mux_failure)?;
        self.endpoint
            .next_action()
            .map(|(_, message)| message)
            .ok_or_else(|| failure(ErrorCode::Protocol))
    }
}
fn mux_failure(error: Error) -> Failure {
    failure(match error {
        Error::Capacity | Error::WouldBlock => ErrorCode::Capacity,
        Error::InvalidRequest => ErrorCode::InvalidRequest,
        Error::Rejected | Error::Closed => ErrorCode::Unavailable,
        _ => ErrorCode::Protocol,
    })
}
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_opening_tests.rs"] mod tests; } }
