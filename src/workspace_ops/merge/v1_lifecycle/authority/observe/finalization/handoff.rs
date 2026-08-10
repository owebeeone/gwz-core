use super::super::super::*;
use super::publication;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    PreparedReverseEntryView, ReverseEntryKind, visit_reverse_entry,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) enum RecordEvidenceOr<T> {
    RecordEvidence(Box<VerifiedEvidenceResult>),
    Ready(T),
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_reverse_publication_handoff<
    B: GitBackend,
>(
    backend: &B,
    _context: &OperationContext,
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
) -> ModelResult<RecordEvidenceOr<VerifiedPublicationHandoff>> {
    let issuer = AuthorityIssuer::for_observer(current);
    let permit = ReverseEntryInspectionPermit::issue(&issuer)?;
    let mut visitor = PublicationHandoffVisitor { backend };
    visit_reverse_entry(permit, current, preview, &mut visitor)
}

struct PublicationHandoffVisitor<'a, B> {
    backend: &'a B,
}

impl<B> super::super::reverse_entry_visitor_seal::Visitor for PublicationHandoffVisitor<'_, B> {}

impl<B: GitBackend> SealedReverseEntryVisitor for PublicationHandoffVisitor<'_, B> {
    type SealedAuthority = RecordEvidenceOr<VerifiedPublicationHandoff>;

    fn inspect(
        &mut self,
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        request: V1LifecycleRequest,
        kind: ReverseEntryKind,
        anticipated_model_sha256: [u8; 32],
    ) -> ModelResult<Self::SealedAuthority> {
        if anticipated.publication.is_none() && anticipated.state != OperationState::Finalizing {
            return issue_handoff(
                current,
                request,
                kind,
                anticipated_model_sha256,
                PublicationHandoffFact::NoCandidate,
            );
        }
        if anticipated != current.record() {
            return Err(handoff_error(
                "publication-bearing reverse entry must use an action-free predecessor",
            ));
        }
        match publication::observe_reverse_handoff(self.backend, current)? {
            publication::ReversePublicationHandoffObservation::RecordEvidence(value) => {
                Ok(RecordEvidenceOr::RecordEvidence(value))
            }
            publication::ReversePublicationHandoffObservation::Ready(fact) => {
                issue_handoff(current, request, kind, anticipated_model_sha256, fact)
            }
        }
    }
}

fn issue_handoff(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    kind: ReverseEntryKind,
    anticipated_model_sha256: [u8; 32],
    publication: PublicationHandoffFact,
) -> ModelResult<RecordEvidenceOr<VerifiedPublicationHandoff>> {
    let proof = VerifiedPublicationHandoff::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "handoff",
        "verified",
        ReverseEntryAuthorityPayload {
            request,
            kind,
            anticipated_model_sha256,
            publication,
        },
    )?;
    Ok(RecordEvidenceOr::Ready(proof))
}

fn handoff_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail.into())
}
