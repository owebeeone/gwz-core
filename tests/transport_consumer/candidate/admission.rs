//! Test-only host admission guard for the placement candidate.
//!
//! Capability probing and dispatch share one synchronized receiver owner. A
//! permit rechecks that owner while holding the same lock used by replacement
//! and invalidation, and runs the send callback while the receiver is pinned.

use crate::candidate_generated::{TransportCapabilitiesResponse, TransportPlacement};
use gwz_transport::binding::EndpointConfig;
use gwz_transport::protocol::{
    AuthPolicy, Bind, EndpointRole, Envelope, Limits, MessageKind, Scheme,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverGeneration {
    pub id: u64,
    pub receiver_id: String,
    pub backend_family: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    UnsupportedCapabilities,
    InvalidReceiver,
    StaleGeneration,
    ReceiverMismatch,
    ReceiverClosed,
}

#[derive(Debug)]
struct AdmissionState {
    generation: ReceiverGeneration,
    policy: AuthPolicy,
    limits: Limits,
    live: bool,
}

#[derive(Debug)]
pub struct DispatchPermit {
    state: Arc<Mutex<AdmissionState>>,
    generation: ReceiverGeneration,
}

impl DispatchPermit {
    pub fn generation(&self) -> &ReceiverGeneration {
        &self.generation
    }

    /// Revalidate and send while replacement/invalidation is excluded. The
    /// callback receives the pinned generation so it cannot choose another
    /// receiver after admission.
    pub fn dispatch<T>(
        &self,
        send: impl FnOnce(&ReceiverGeneration) -> T,
    ) -> Result<T, AdmissionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AdmissionError::ReceiverClosed)?;
        validate_generation(&state, &self.generation)?;
        Ok(send(&self.generation))
    }
}

#[derive(Clone, Debug)]
pub struct ReceiverAdmission {
    state: Arc<Mutex<AdmissionState>>,
}

impl ReceiverAdmission {
    pub fn establish(
        generation: ReceiverGeneration,
        capabilities: &TransportCapabilitiesResponse,
        policy: AuthPolicy,
    ) -> Result<Self, AdmissionError> {
        let limits = validate_capabilities(&generation, capabilities, policy)?;
        Ok(Self {
            state: Arc::new(Mutex::new(AdmissionState {
                generation,
                policy,
                limits,
                live: true,
            })),
        })
    }

    pub fn placement_or_local(placement: Option<TransportPlacement>) -> TransportPlacement {
        placement.unwrap_or(TransportPlacement::Local)
    }

    pub fn admit_explicit_cli(
        &self,
        generation: &ReceiverGeneration,
    ) -> Result<DispatchPermit, AdmissionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AdmissionError::ReceiverClosed)?;
        validate_generation(&state, generation)?;
        Ok(DispatchPermit {
            state: Arc::clone(&self.state),
            generation: generation.clone(),
        })
    }

    /// Install a probed replacement before any operation is admitted on it.
    /// Existing permits retain the shared owner and fail their dispatch check.
    pub fn replace_receiver(
        &self,
        generation: ReceiverGeneration,
        capabilities: &TransportCapabilitiesResponse,
        policy: AuthPolicy,
    ) -> Result<(), AdmissionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AdmissionError::ReceiverClosed)?;
        state.live = false;
        if generation.id <= state.generation.id {
            return Err(AdmissionError::StaleGeneration);
        }
        let limits = validate_capabilities(&generation, capabilities, policy)?;
        state.generation = generation;
        state.policy = policy;
        state.limits = limits;
        state.live = true;
        Ok(())
    }

    pub fn invalidate(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.live = false;
        }
    }

    pub fn policy(&self) -> Result<AuthPolicy, AdmissionError> {
        self.state
            .lock()
            .map(|state| state.policy)
            .map_err(|_| AdmissionError::ReceiverClosed)
    }

    pub fn limits(&self) -> Result<Limits, AdmissionError> {
        self.state
            .lock()
            .map(|state| state.limits.clone())
            .map_err(|_| AdmissionError::ReceiverClosed)
    }
}

fn validate_capabilities(
    generation: &ReceiverGeneration,
    capabilities: &TransportCapabilitiesResponse,
    policy: AuthPolicy,
) -> Result<Limits, AdmissionError> {
    if generation.receiver_id.is_empty() || generation.backend_family.is_empty() {
        return Err(AdmissionError::InvalidReceiver);
    }
    if !matches!(policy, AuthPolicy::SshAmbient | AuthPolicy::SshExplicit) {
        return Err(AdmissionError::UnsupportedCapabilities);
    }
    let versions = capabilities
        .message_versions
        .as_ref()
        .ok_or(AdmissionError::UnsupportedCapabilities)?;
    let placements = capabilities
        .placements
        .as_ref()
        .ok_or(AdmissionError::UnsupportedCapabilities)?;
    let schemes = capabilities
        .schemes
        .as_ref()
        .ok_or(AdmissionError::UnsupportedCapabilities)?;
    let policies = capabilities
        .auth_policies
        .as_ref()
        .ok_or(AdmissionError::UnsupportedCapabilities)?;
    let limits = capabilities
        .message_limits
        .clone()
        .ok_or(AdmissionError::UnsupportedCapabilities)?;
    if !versions.contains(&2)
        || !placements.contains(&TransportPlacement::Cli)
        || !schemes.contains(&Scheme::Ssh)
        || !policies.contains(&policy)
        || !usable_limits(&limits)
    {
        return Err(AdmissionError::UnsupportedCapabilities);
    }
    Ok(limits)
}

fn usable_limits(limits: &Limits) -> bool {
    if !owner_accepts_limits(limits) {
        return false;
    }
    [
        limits.encoded_frame,
        limits.data_payload,
        limits.metadata_bytes,
        limits.nesting,
        limits.collection_entries,
        limits.decode_allocation,
        limits.queued_bytes,
        limits.queued_frames,
        limits.receive_window,
        limits.control_reserve_bytes,
        limits.control_reserve_frames,
    ]
    .into_iter()
    .all(|value| value > 0)
        && limits.encoded_frame <= 131_072
        && limits.data_payload <= 65_536
        && limits.metadata_bytes <= 16_384
        && limits.nesting <= 16
        && limits.collection_entries <= 256
        && limits.decode_allocation <= 524_288
        && limits.queued_bytes <= 4 * 1024 * 1024
        && limits.queued_frames <= 64
        && limits.receive_window <= limits.queued_bytes
        && limits.control_reserve_bytes < limits.queued_bytes
        && limits.control_reserve_frames < limits.queued_frames
        && limits.encoded_frame >= 4_096
        && limits.metadata_bytes >= 256
        && limits.nesting >= 10
        && limits.collection_entries >= 128
        && limits.decode_allocation >= 65_536
        && limits.control_reserve_bytes >= 4_096
        && limits.control_reserve_frames >= 2
        && limits.queued_bytes - limits.control_reserve_bytes
            >= limits.encoded_frame + limits.decode_allocation
        && limits.queued_frames - limits.control_reserve_frames >= 2
        && limits.receive_window <= limits.queued_bytes - limits.control_reserve_bytes
}

/// Exercise the owner's public binding path, which invokes its private codec
/// limits validator and live minimum checks, without changing production
/// visibility just for this candidate fixture.
fn owner_accepts_limits(limits: &Limits) -> bool {
    let policy = AuthPolicy::SshAmbient;
    let config = EndpointConfig {
        endpoint_id: "candidate".into(),
        role: EndpointRole::Local,
        schemes: vec![Scheme::Ssh],
        policies: vec![policy],
        limits: limits.clone(),
        trust_owner: "candidate".into(),
    };
    let offer = Envelope {
        version: 1,
        session_id: "candidate".into(),
        kind: MessageKind::Bind,
        bind: Some(Bind {
            versions: vec![2],
            role: EndpointRole::Local,
            schemes: vec![Scheme::Ssh],
            policies: vec![policy],
            receive_limits: limits.clone(),
        }),
        ..Default::default()
    };
    config.accept(&offer).is_ok()
}

fn validate_generation(
    state: &AdmissionState,
    generation: &ReceiverGeneration,
) -> Result<(), AdmissionError> {
    if !state.live {
        return Err(AdmissionError::ReceiverClosed);
    }
    if generation.id != state.generation.id {
        return Err(AdmissionError::StaleGeneration);
    }
    if generation.receiver_id != state.generation.receiver_id
        || generation.backend_family != state.generation.backend_family
    {
        return Err(AdmissionError::ReceiverMismatch);
    }
    Ok(())
}
