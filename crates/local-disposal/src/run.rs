//! The ordered disposal sequence itself: `run` drives design §5.2 steps 1-5,
//! `detach` performs the pointer-then-row removal both exits share.

use gwz_family_model::{FamilyChange, ListState, RemovalReason, TargetObservation};
use gwz_family_store_contract::{FamilySession, MetadataEffect};

use crate::*;

pub(crate) fn run(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
    ports: &mut dyn DisposalPorts,
    effects: &mut Vec<DisposeEffect>,
) -> Result<Vec<HazardFinding>, DisposeError> {
    let waivers = match &request.policy {
        DisposePolicy::Keep => None,
        DisposePolicy::Delete { waivers } => {
            let unique: std::collections::BTreeSet<_> = waivers.iter().collect();
            if unique.len() != waivers.len() {
                return Err(DisposeError::Refused(Refusal::InvalidRow {
                    name: request.name.clone(),
                    detail: "repeated hazard waiver".to_owned(),
                }));
            }
            Some(waivers.as_slice())
        }
    };

    // Step 1.
    let plan = validate(request, session)?;

    // Step 2: keep detaches metadata only, whatever state the target is in.
    let Some(waivers) = waivers else {
        detach(request, session, effects, RemovalReason::Keep)?;
        return Ok(Vec::new());
    };

    // Step 3: fresh evidence, then the work and history checks.
    let evidence = ports
        .observe_target(&plan.target)
        .map_err(DisposeError::Port)?;
    if !evidence.unknown.is_empty() {
        return Err(DisposeError::Unknown(evidence.unknown));
    }
    match classify_target(&plan.row, Some(&evidence.target)) {
        ListState::Ready => {}
        // Step 5's second half: the contents are already gone, so an
        // explicit dispose may remove the stale row. No file is touched, so
        // no work or history check applies.
        ListState::Missing => {
            // An observer that reports the target absent and repositories
            // inside it contradicts itself. This is the one path that
            // removes a row with no work or history check, so an
            // inconsistent observation refuses rather than being reconciled.
            if !evidence.repositories.is_empty() {
                return Err(DisposeError::PathMismatch {
                    expected: plan.target,
                    observed: format!(
                        "the target is absent, yet {} repositor(y|ies) were observed in it",
                        evidence.repositories.len()
                    ),
                });
            }
            detach(request, session, effects, RemovalReason::Stale)?;
            return Ok(Vec::new());
        }
        state @ (ListState::Incomplete | ListState::InterruptedDisposal) => {
            debug_assert_ne!(plan.row.state, MemberState::Ready, "{state:?}");
            return Err(DisposeError::Refused(Refusal::WrongState {
                name: request.name.clone(),
                expected: MemberState::Ready,
                actual: plan.row.state,
            }));
        }
        state => {
            return Err(DisposeError::PathMismatch {
                expected: plan.target,
                observed: describe(state, &evidence.target),
            });
        }
    }
    let waived = inspect(&plan, &evidence, waivers, ports)?;

    // Step 4. The write result is checked before anything is removed: the
    // session revalidates the row's state and allocation under the lock.
    session
        .apply(&FamilyChange::MarkDisposing {
            name: request.name.clone(),
            expected_allocation: plan.row.allocation_id.clone(),
        })
        .map_err(DisposeError::Store)?;
    effects.push(DisposeEffect::RowDisposing);

    ports
        .remove_directory(&plan.target)
        .map_err(|failure| DisposeError::RemovalStopped {
            remaining: failure.remaining,
            detail: failure.error.to_string(),
        })?;
    effects.push(DisposeEffect::DirectoryRemoved);

    // Step 5.
    detach(request, session, effects, RemovalReason::Disposed)?;
    Ok(waived)
}

/// Remove the matching pointer, then the row. The order is the store
/// contract's only recoverable one, and a pointer the store cannot
/// physically remove stops here — the report names the pointer, not the row
/// (checkpoint §11, lane D).
pub(crate) fn detach(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
    effects: &mut Vec<DisposeEffect>,
    reason: RemovalReason,
) -> Result<(), DisposeError> {
    let applied = session
        .remove_pointer(&request.name)
        .map_err(DisposeError::Store)?;
    if applied
        .effects
        .iter()
        .any(|effect| matches!(effect, MetadataEffect::PointerRemoved { .. }))
    {
        effects.push(DisposeEffect::PointerRemoved);
    }
    session
        .apply(&FamilyChange::RemoveRow {
            name: request.name.clone(),
            reason,
        })
        .map_err(DisposeError::Store)?;
    effects.push(match reason {
        RemovalReason::Keep => DisposeEffect::RowDetached,
        RemovalReason::Disposed | RemovalReason::Stale => DisposeEffect::RowRemoved,
    });
    Ok(())
}

pub(crate) fn describe(state: ListState, observation: &TargetObservation) -> String {
    match (state, observation) {
        (_, TargetObservation::Malformed { detail }) => {
            format!("its family metadata could not be decoded: {detail}")
        }
        (ListState::PointerRemoved, _) => {
            "its family pointer is gone; an interrupted detach left the tree in place".to_owned()
        }
        (_, TargetObservation::Present { pointer, marker }) => format!(
            "its family pointer or allocation marker does not belong to this row \
             (pointer {pointer:?}, marker {marker:?})"
        ),
        (state, observation) => format!("{state:?} for {observation:?}"),
    }
}
