mod publication;
#[cfg(test)]
mod v1;
#[cfg(test)]
mod v1_candidate;
mod workspace;

#[cfg(test)]
pub(in crate::workspace_ops::merge) use publication::classify_candidate_publication_for_v1;
#[allow(
    unused_imports,
    reason = "retained v0 wrappers and shared view entry points form one package seam"
)]
pub(super) use publication::{
    CandidatePublicationObservation, CandidatePublicationPrefix, FinalizationNextAction,
    classify_candidate_publication, classify_candidate_publication_view, finalization_next_action,
    publication_prefix_allowed, publication_prefix_allowed_view,
};
#[cfg(test)]
pub(crate) use publication::{finalization_next_action_for_i2, finalization_next_action_for_v1};
#[cfg(test)]
pub(super) use publication::{publication_required_for_v1, validate_candidate_semantics_for_v1};
#[cfg(test)]
pub(in crate::workspace_ops::merge) use v1::{
    V1AcceptanceMetadata, V1AcceptanceRecord, build_v1_acceptance, classify_frozen_v1_publication,
};
#[cfg(test)]
pub(in crate::workspace_ops::merge) use v1_candidate::{
    V1CandidateBuildInput, build_v1_candidate, candidate_artifacts,
    candidate_files as v1_candidate_files, composition_message as v1_composition_message,
    publication_base as v1_publication_base,
};
#[cfg(test)]
pub(super) use workspace::accepted_root_checkout;
pub(super) use workspace::{
    AcceptedRootBase, CompleteLockErrorKind, accepted_root_checkout_with_observation,
    construct_complete_lock, publication_required, selected_root_participant,
};
