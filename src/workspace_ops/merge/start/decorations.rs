//! Start decorations: the plan's prediction, carried onto the response.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §4, "Responses").** "Start decorations
//! (`predicted`, `prediction_complete`, `live_commit` for a conflicted row)
//! apply to the v1 start response as `decorate_start_response` applies them to
//! v0." No record version stores a prediction — it is the plan's, and the plan
//! exists only in `handle_start_durable` — so this is applied there, above the
//! lifecycle, exactly where the v0 engine applied it.

use super::super::MergeParticipantPlan;
use crate::model::ModelResult;
use crate::MergeParticipantState as PState;

pub(crate) fn decorate_start_response(
    mut response: crate::MergeResponse,
    plan: &[MergeParticipantPlan],
) -> ModelResult<crate::MergeResponse> {
    for (repo, participant) in response.repos.iter_mut().zip(plan) {
        repo.predicted = participant.analysis;
        repo.prediction_complete = Some(participant.prediction_complete);
        repo.live_commit = match repo.state {
            PState::UpToDate | PState::FastForwarded | PState::Merged => {
                repo.resulting_commit.clone()
            }
            PState::Conflicted => Some(participant.before_commit.clone()),
            _ => None,
        };
    }
    Ok(response)
}
