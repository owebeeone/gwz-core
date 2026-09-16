//! Admission: the pre-reservation checks over the request and its source,
//! and the completion faults a finished copy may still carry.

use gwz_family_model::{
    CloneMode, PointerObservation, check_allocation_available, check_name_available,
    check_name_free_of_remotes, check_path_available,
};
use gwz_family_store_contract::FamilySession;
use gwz_repo_contract::RepoKey;

use crate::*;

/// Step 1. Every check that can be decided is decided, and the refusals are
/// reported together. A port error is not a refusal: it stops here, with no
/// effects, rather than presenting a partial aggregate as the reasons.
pub(crate) fn admit(
    request: &InstallRequest,
    session: &mut dyn FamilySession,
    ports: &mut dyn InstallPorts,
) -> Result<SourceSnapshot, (InstallStep, InstallError)> {
    let mut refusals = Vec::new();

    let snapshot = match ports.snapshot_source(&request.source) {
        Ok(snapshot) => Some(snapshot),
        Err(InstallPortError::Layout(error)) => {
            refusals.push(InstallRefusal::SourceLayout(error));
            None
        }
        Err(error) => {
            return Err((
                InstallStep::InventorySource,
                InstallError::Source(Box::new(error)),
            ));
        }
    };

    let observed = ports
        .observe_destination(&request.destination)
        .map_err(|error| {
            (
                InstallStep::ObserveDestination,
                InstallError::Port(Box::new(error)),
            )
        })?;
    if observed.nonempty {
        refusals.push(InstallRefusal::DestinationNotEmpty {
            destination: request.destination.clone(),
        });
    }
    if observed.is_workspace {
        refusals.push(InstallRefusal::DestinationIsWorkspace {
            destination: request.destination.clone(),
        });
    }

    match session.reread() {
        // No index yet: `Allocate` refuses `NoFamily` on its own, and the
        // model has nothing to compare against here.
        Ok(None) => {}
        Ok(Some(view)) => {
            for refusal in [
                check_name_available(&view, &request.name),
                check_path_available(&view, &request.path),
                check_allocation_available(&view, &request.name, &request.allocation),
            ]
            .into_iter()
            .filter_map(Result::err)
            {
                refusals.push(InstallRefusal::Family(refusal));
            }
        }
        Err(error) => return Err((InstallStep::Reserve, InstallError::Store(Box::new(error)))),
    }

    if let Some(snapshot) = &snapshot {
        refusals.extend(source_refusals(request, snapshot));
    }

    let Some(snapshot) = snapshot else {
        // Only a source-layout refusal returns no snapshot, so `refusals`
        // is non-empty here.
        return Err((
            InstallStep::InventorySource,
            InstallError::Refused(refusals),
        ));
    };
    if !refusals.is_empty() {
        return Err((InstallStep::Reserve, InstallError::Refused(refusals)));
    }
    Ok(snapshot)
}

/// The checks the captured source answers, all before the `creating` row.
pub(crate) fn source_refusals(
    request: &InstallRequest,
    snapshot: &SourceSnapshot,
) -> Vec<InstallRefusal> {
    let mut refusals = Vec::new();
    let remotes: Vec<(String, String)> = snapshot
        .repositories
        .iter()
        .flat_map(|repo| {
            repo.remotes
                .iter()
                .map(|remote| (repo.key.to_string(), remote.clone()))
        })
        .collect();
    if let Err(collision) = check_name_free_of_remotes(
        &request.name,
        remotes
            .iter()
            .map(|(member, remote)| (member.as_str(), remote.as_str())),
    ) {
        refusals.push(InstallRefusal::NameIsRemote(collision));
    }

    if request.mode == CloneMode::Verbatim
        && let Some(detail) = &snapshot.open_gwz_merge
    {
        refusals.push(InstallRefusal::SourceOpenMerge {
            detail: detail.clone(),
        });
    }

    if !request.copies_the_tree() && !snapshot.captured(&RepoKey::Root) {
        refusals.push(InstallRefusal::RootNotCaptured);
    }

    if let Some(branch) = &request.branch {
        if request.copies_the_tree() {
            refusals.push(InstallRefusal::BranchNotSupported {
                branch: branch.clone(),
                mode: request.mode,
            });
        } else {
            for repo in &snapshot.repositories {
                if repo.branches.iter().any(|existing| existing == branch) {
                    refusals.push(InstallRefusal::BranchExists {
                        branch: branch.clone(),
                        member: repo.key.clone(),
                    });
                }
            }
        }
    }
    refusals
}

/// Design §4.1's post-copy check and "at ready" column, plus §4.0's
/// dest-complete independence rule, applied to what the port observed.
pub(crate) fn completion_faults(observed: &DestinationObservation) -> Vec<CompletionFault> {
    let mut faults = Vec::new();
    match (observed.family_index, observed.pointer) {
        (true, PointerObservation::Absent) => faults.push(CompletionFault::FamilyIndexPresent),
        (true, _) => faults.push(CompletionFault::ConflictingMetadata),
        (false, PointerObservation::Matches) => {}
        (false, PointerObservation::Absent) => faults.push(CompletionFault::PointerMissing),
        (false, observed) => faults.push(CompletionFault::PointerInvalid { observed }),
    }
    if observed.merge_store {
        faults.push(CompletionFault::MergeStorePresent);
    }
    for path in &observed.residual {
        faults.push(CompletionFault::ResidualPath { path: path.clone() });
    }
    for detail in &observed.dependencies {
        faults.push(CompletionFault::NotIndependent {
            detail: detail.clone(),
        });
    }
    faults
}
