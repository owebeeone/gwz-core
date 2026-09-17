//! The ordered installation sequence of design §4.

use gwz_copy_contract::{Cancellation, CopyRequest, TreeCopier};
use gwz_family_model::{CloneMode, FamilyChange};
use gwz_family_store_contract::FamilySession;

use crate::*;

/// Install one local clone destination.
pub fn install(
    request: &InstallRequest,
    session: &mut dyn FamilySession,
    copier: &dyn TreeCopier,
    ports: &mut dyn InstallPorts,
    cancellation: &dyn Cancellation,
) -> Result<InstallReport, InstallFailure> {
    let mut progress = Progress::default();

    // Step 1: admit or refuse, before anything is written.
    let snapshot = match admit(request, session, ports) {
        Ok(snapshot) => snapshot,
        Err((step, error)) => return Err(progress.stop(session, &request.name, step, error)),
    };

    // Step 2: reserve the row, then allocate the destination directory.
    macro_rules! attempt {
        ($step:expr, $result:expr) => {
            match $result {
                Ok(value) => value,
                Err(error) => return Err(progress.stop(session, &request.name, $step, error)),
            }
        };
    }
    macro_rules! checkpoint {
        ($step:expr) => {
            if cancellation.is_cancelled() {
                return Err(progress.stop(session, &request.name, $step, InstallError::Cancelled));
            }
        };
    }

    checkpoint!(InstallStep::Reserve);
    attempt!(
        InstallStep::Reserve,
        session
            .apply(&FamilyChange::Allocate {
                name: request.name.clone(),
                row: request.row(),
            })
            .map_err(|error| InstallError::Store(Box::new(error)))
    );
    progress.did(InstallEffect::RowAllocated);

    attempt!(
        InstallStep::AllocateDestination,
        ports
            .allocate_destination(&request.destination)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    progress.did(InstallEffect::DestinationAllocated);

    // Step 3: build the destination, install its metadata, check it, and
    // recheck the source before anything is published.
    if request.copies_the_tree() {
        checkpoint!(InstallStep::CopyTree);
        let report = attempt!(
            InstallStep::CopyTree,
            copier
                .copy_tree(&copy_request(request), cancellation)
                .map_err(|error| InstallError::Copy(Box::new(error)))
        );
        progress.copy = Some(report);
        progress.did(InstallEffect::TreeCopied);
    } else {
        checkpoint!(InstallStep::ConstructRepositories);
        attempt!(
            InstallStep::ConstructRepositories,
            ports
                .construct_repositories(&ConstructionRequest {
                    mode: request.mode,
                    branch: request.branch.clone(),
                    snapshot: snapshot.clone(),
                    destination: request.destination.clone(),
                })
                .map_err(|error| InstallError::Port(Box::new(error)))
        );
        progress.did(InstallEffect::RepositoriesConstructed);
    }

    let git = attempt!(
        InstallStep::InstallDestinationGit,
        ports
            .install_destination_git(&request.destination)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    progress.git = Some(git);
    progress.did(InstallEffect::DestinationGitInstalled);

    // R1: what the destination copied is written down before the pointer,
    // so a published lane always carries its own baseline.
    let record = attempt!(
        InstallStep::RecordCopy,
        ports
            .record_copy(&request.destination)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    progress.record = Some(record);
    progress.did(InstallEffect::CopyRecorded);

    attempt!(
        InstallStep::InstallPointer,
        session
            .install_pointer(&request.name, &request.destination)
            .map_err(|error| InstallError::Store(Box::new(error)))
    );
    progress.did(InstallEffect::PointerInstalled);

    let observed = attempt!(
        InstallStep::CheckDestination,
        ports
            .observe_destination(&request.destination)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    let faults = completion_faults(&observed);
    if !faults.is_empty() {
        return Err(progress.stop(
            session,
            &request.name,
            InstallStep::CheckDestination,
            InstallError::Incomplete(faults),
        ));
    }

    attempt!(
        InstallStep::RecheckSource,
        ports
            .recheck_source(&snapshot)
            .map_err(|error| InstallError::Source(Box::new(error)))
    );

    let plan = ConfigurationPlan {
        destination: request.destination.clone(),
        mode: request.mode,
        branch: request.branch.clone(),
        snapshot,
    };
    let configuration = attempt!(
        InstallStep::RecaptureConfiguration,
        ports
            .recapture_configuration(&plan)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    let recapture_required = request.mode != CloneMode::Verbatim;
    let recaptured = configuration.lock_recaptured;
    progress.configuration = Some(configuration);
    progress.did(InstallEffect::ConfigurationInstalled);
    if recapture_required && !recaptured {
        return Err(progress.stop(
            session,
            &request.name,
            InstallStep::RecaptureConfiguration,
            InstallError::Incomplete(vec![CompletionFault::LockNotRecaptured]),
        ));
    }

    // Step 4: the final manifest last, then ready.
    checkpoint!(InstallStep::PublishManifest);
    let receipt = attempt!(
        InstallStep::PublishManifest,
        ports
            .publish_manifest(&plan)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    progress.did(InstallEffect::ManifestPublished);
    if !receipt.marker_regenerated {
        return Err(progress.stop(
            session,
            &request.name,
            InstallStep::PublishManifest,
            InstallError::Incomplete(vec![CompletionFault::MarkerNotRegenerated]),
        ));
    }

    attempt!(
        InstallStep::MarkReady,
        session
            .apply(&FamilyChange::MarkReady {
                name: request.name.clone(),
                expected_allocation: request.allocation.clone(),
            })
            .map_err(|error| InstallError::Store(Box::new(error)))
    );
    progress.did(InstallEffect::RowReady);

    Ok(InstallReport {
        copy: progress.copy,
        git: progress.git,
        record: progress.record,
        configuration: progress.configuration,
        effects: progress.effects,
    })
}

pub(crate) fn copy_request(request: &InstallRequest) -> CopyRequest {
    CopyRequest {
        source: request.source.clone(),
        destination: request.destination.clone(),
        exclusions: request.exclusions.clone(),
        mode: request.copy_mode,
    }
}
