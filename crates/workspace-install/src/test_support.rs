//! Recording, scripted and order-checking fakes for orchestration tests.
//!
//! [`Journal`] is the point of the module: ports, the family session and the
//! tree copier all append to one ordered event log, and the log checks
//! design §4's ordering as it is written. A fake that is handed an
//! out-of-order call records a violation instead of quietly accepting it, so
//! a test that asserts `journal.violations().is_empty()` fails when the
//! installer reorders its steps. The journal's own teeth are tested in this
//! module.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gwz_copy_contract::{Cancellation, CopyError, CopyReport, CopyRequest, TreeCopier};
use gwz_family_model::{AllocationId, FamilyChange, FamilyId, FamilyView, MemberName};
use gwz_family_store_contract::{AppliedChange, FamilySession, StoreError};

use crate::{
    ConfigurationPlan, ConfigurationReport, ConstructionRequest, DestinationObservation,
    InstallPortError, InstallPorts, ManifestReceipt, SourceSnapshot,
};

/// One thing the installer did, whichever collaborator it did it through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallEvent {
    SnapshotSource,
    ObserveDestination,
    AllocateDestination,
    CopyTree,
    ConstructRepositories,
    InstallPointer,
    RecheckSource,
    RecaptureConfiguration,
    PublishManifest,
    /// `FamilySession::reread`.
    Reread,
    /// `FamilyChange::Allocate`: the `creating` row.
    Allocate,
    /// `FamilyChange::RecordError`: the diagnostic on a retained row.
    RecordError,
    /// `FamilyChange::MarkReady`.
    MarkReady,
    /// Any other session call; the installer makes none.
    Other,
}

impl InstallEvent {
    const fn label(self) -> &'static str {
        match self {
            Self::SnapshotSource => "snapshot_source",
            Self::ObserveDestination => "observe_destination",
            Self::AllocateDestination => "allocate_destination",
            Self::CopyTree => "copy_tree",
            Self::ConstructRepositories => "construct_repositories",
            Self::InstallPointer => "install_pointer",
            Self::RecheckSource => "recheck_source",
            Self::RecaptureConfiguration => "recapture_configuration",
            Self::PublishManifest => "publish_manifest",
            Self::Reread => "reread",
            Self::Allocate => "allocate row",
            Self::RecordError => "record error",
            Self::MarkReady => "mark ready",
            Self::Other => "other session call",
        }
    }
}

#[derive(Debug, Default)]
struct JournalState {
    events: Vec<InstallEvent>,
    violations: Vec<String>,
}

/// The shared ordered log. Cloning shares one log, so the ports, the
/// session and the copier of one test all write to it.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    state: Rc<RefCell<JournalState>>,
}

impl Journal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<InstallEvent> {
        self.state.borrow().events.clone()
    }

    /// Every ordering rule broken so far, in the order they were broken.
    pub fn violations(&self) -> Vec<String> {
        self.state.borrow().violations.clone()
    }

    pub fn seen(&self, event: InstallEvent) -> bool {
        self.state.borrow().events.contains(&event)
    }

    /// Append `event`, checking design §4's ordering first.
    pub fn record(&self, event: InstallEvent) {
        let mut state = self.state.borrow_mut();
        if let Some(violation) = violation(&state.events, event) {
            state.violations.push(violation);
        }
        state.events.push(event);
    }
}

/// The ordering property, stated once: a complete inventory precedes the
/// reservation, every destination effect follows it, the manifest is last
/// and `ready` follows the manifest.
fn violation(events: &[InstallEvent], event: InstallEvent) -> Option<String> {
    use InstallEvent::{
        Allocate, AllocateDestination, ConstructRepositories, CopyTree, InstallPointer, MarkReady,
        ObserveDestination, PublishManifest, RecaptureConfiguration, RecheckSource, RecordError,
        SnapshotSource,
    };
    let seen = |wanted: InstallEvent| events.contains(&wanted);
    let broke = |detail: String| Some(format!("{}: {detail}", event.label()));

    // A failed `mark ready` is journaled too, and the diagnostic on the
    // retained row is the one thing that legally follows it.
    if seen(MarkReady) && event != RecordError {
        return broke("ran after the row was made ready".to_owned());
    }
    if seen(PublishManifest) && !matches!(event, MarkReady | RecordError) {
        return broke("ran after the final manifest was published".to_owned());
    }
    match event {
        Allocate if !seen(SnapshotSource) || !seen(ObserveDestination) => {
            broke("the creating row precedes a complete inventory".to_owned())
        }
        AllocateDestination
        | CopyTree
        | ConstructRepositories
        | InstallPointer
        | RecheckSource
        | RecaptureConfiguration
        | PublishManifest
        | RecordError
            if !seen(Allocate) =>
        {
            broke("ran before the creating row".to_owned())
        }
        CopyTree | ConstructRepositories if !seen(AllocateDestination) => {
            broke("ran before the destination was allocated".to_owned())
        }
        InstallPointer if !seen(CopyTree) && !seen(ConstructRepositories) => {
            broke("ran before the destination was built".to_owned())
        }
        PublishManifest if !seen(RecheckSource) || !seen(RecaptureConfiguration) => {
            broke("the manifest precedes the source recheck and the lock recapture".to_owned())
        }
        MarkReady if !seen(PublishManifest) => {
            broke("ready precedes the final manifest".to_owned())
        }
        _ => None,
    }
}

/// A recording, scripted [`InstallPorts`].
///
/// Unscripted answers are the ones a passing install needs, so a test
/// scripts only what it is about: `snapshot_source` refuses typed until a
/// snapshot is set, `observe_destination` answers
/// [`DestinationObservation::absent`] first and
/// [`DestinationObservation::complete`] afterwards, recapture reports a
/// recaptured lock and no generated changes, and the manifest receipt
/// reports a regenerated marker.
#[derive(Debug, Default)]
pub struct RecordingInstallPorts {
    journal: Journal,
    snapshot: Option<SourceSnapshot>,
    observations: Vec<DestinationObservation>,
    configuration: Option<ConfigurationReport>,
    receipt: Option<ManifestReceipt>,
    failures: Vec<(InstallEvent, InstallPortError)>,
    construction: Vec<ConstructionRequest>,
    plans: Vec<ConfigurationPlan>,
}

impl RecordingInstallPorts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Share `journal` with the session and copier of the same test.
    pub fn with_journal(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            ..Self::default()
        }
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn calls(&self) -> Vec<InstallEvent> {
        self.journal.events()
    }

    /// Script the snapshot `snapshot_source` answers with.
    pub fn snapshot(&mut self, snapshot: SourceSnapshot) {
        self.snapshot = Some(snapshot);
    }

    /// Queue one `observe_destination` answer (first queued, first
    /// returned).
    pub fn observe(&mut self, observation: DestinationObservation) {
        self.observations.push(observation);
    }

    pub fn recapture(&mut self, report: ConfigurationReport) {
        self.configuration = Some(report);
    }

    pub fn receipt(&mut self, receipt: ManifestReceipt) {
        self.receipt = Some(receipt);
    }

    pub fn fail_next(&mut self, call: InstallEvent, error: InstallPortError) {
        self.failures.push((call, error));
    }

    /// The construction requests the installer made, in order.
    pub fn construction_requests(&self) -> &[ConstructionRequest] {
        &self.construction
    }

    /// The configuration plans the installer made, in order.
    pub fn plans(&self) -> &[ConfigurationPlan] {
        &self.plans
    }

    fn record(&mut self, call: InstallEvent) -> Result<(), InstallPortError> {
        self.journal.record(call);
        if let Some(index) = self.failures.iter().position(|(name, _)| *name == call) {
            return Err(self.failures.remove(index).1);
        }
        Ok(())
    }
}

impl InstallPorts for RecordingInstallPorts {
    fn snapshot_source(&mut self, _source: &Path) -> Result<SourceSnapshot, InstallPortError> {
        self.record(InstallEvent::SnapshotSource)?;
        self.snapshot
            .clone()
            .ok_or(InstallPortError::Unimplemented {
                operation: "unscripted snapshot_source",
            })
    }

    fn observe_destination(
        &mut self,
        _destination: &Path,
    ) -> Result<DestinationObservation, InstallPortError> {
        let first = !self.journal.seen(InstallEvent::ObserveDestination);
        self.record(InstallEvent::ObserveDestination)?;
        if self.observations.is_empty() {
            return Ok(if first {
                DestinationObservation::absent()
            } else {
                DestinationObservation::complete()
            });
        }
        Ok(self.observations.remove(0))
    }

    fn allocate_destination(&mut self, _destination: &Path) -> Result<(), InstallPortError> {
        self.record(InstallEvent::AllocateDestination)
    }

    fn construct_repositories(
        &mut self,
        request: &ConstructionRequest,
    ) -> Result<(), InstallPortError> {
        self.construction.push(request.clone());
        self.record(InstallEvent::ConstructRepositories)
    }

    fn recheck_source(&mut self, _snapshot: &SourceSnapshot) -> Result<(), InstallPortError> {
        self.record(InstallEvent::RecheckSource)
    }

    fn recapture_configuration(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ConfigurationReport, InstallPortError> {
        self.plans.push(plan.clone());
        self.record(InstallEvent::RecaptureConfiguration)?;
        Ok(self.configuration.clone().unwrap_or(ConfigurationReport {
            lock_recaptured: true,
            generated_changes: Vec::new(),
        }))
    }

    fn publish_manifest(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ManifestReceipt, InstallPortError> {
        self.plans.push(plan.clone());
        self.record(InstallEvent::PublishManifest)?;
        Ok(self.receipt.clone().unwrap_or(ManifestReceipt {
            marker_regenerated: true,
        }))
    }
}

/// A [`FamilySession`] that journals every call before delegating, and can
/// fail one named change without disturbing the store beneath it.
pub struct JournalSession<'a> {
    inner: &'a mut dyn FamilySession,
    journal: Journal,
    failures: Vec<(InstallEvent, StoreError)>,
}

impl<'a> JournalSession<'a> {
    pub fn new(inner: &'a mut dyn FamilySession, journal: &Journal) -> Self {
        Self {
            inner,
            journal: journal.clone(),
            failures: Vec::new(),
        }
    }

    /// Fail the next occurrence of `change` with `error`, writing nothing.
    pub fn fail_next(&mut self, change: InstallEvent, error: StoreError) {
        self.failures.push((change, error));
    }

    fn record(&mut self, event: InstallEvent) -> Result<(), StoreError> {
        self.journal.record(event);
        if let Some(index) = self.failures.iter().position(|(name, _)| *name == event) {
            return Err(self.failures.remove(index).1);
        }
        Ok(())
    }
}

fn change_event(change: &FamilyChange) -> InstallEvent {
    match change {
        FamilyChange::Allocate { .. } => InstallEvent::Allocate,
        FamilyChange::RecordError { .. } => InstallEvent::RecordError,
        FamilyChange::MarkReady { .. } => InstallEvent::MarkReady,
        _ => InstallEvent::Other,
    }
}

impl FamilySession for JournalSession<'_> {
    fn root(&self) -> &Path {
        self.inner.root()
    }

    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
        self.record(InstallEvent::Reread)?;
        self.inner.reread()
    }

    fn found(
        &mut self,
        family_id: FamilyId,
        root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError> {
        self.record(InstallEvent::Other)?;
        self.inner.found(family_id, root_allocation)
    }

    fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError> {
        self.record(change_event(change))?;
        self.inner.apply(change)
    }

    fn install_pointer(
        &mut self,
        name: &MemberName,
        destination: &Path,
    ) -> Result<AppliedChange, StoreError> {
        self.record(InstallEvent::InstallPointer)?;
        self.inner.install_pointer(name, destination)
    }

    fn remove_pointer(&mut self, name: &MemberName) -> Result<AppliedChange, StoreError> {
        self.record(InstallEvent::Other)?;
        self.inner.remove_pointer(name)
    }
}

/// A [`TreeCopier`] that journals the copy before delegating.
pub struct JournalCopier<'a> {
    inner: &'a dyn TreeCopier,
    journal: Journal,
}

impl<'a> JournalCopier<'a> {
    pub fn new(inner: &'a dyn TreeCopier, journal: &Journal) -> Self {
        Self {
            inner,
            journal: journal.clone(),
        }
    }
}

impl TreeCopier for JournalCopier<'_> {
    fn copy_tree(
        &self,
        request: &CopyRequest,
        cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError> {
        self.journal.record(InstallEvent::CopyTree);
        self.inner.copy_tree(request, cancellation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InstallPortError;

    #[test]
    fn unscripted_snapshot_fails_typed_and_failures_are_consumed_in_order() {
        let mut ports = RecordingInstallPorts::new();
        assert!(matches!(
            ports.snapshot_source(Path::new("/src")),
            Err(InstallPortError::Unimplemented { .. })
        ));
        ports.fail_next(
            InstallEvent::RecheckSource,
            InstallPortError::Drift {
                detail: "HEAD moved".to_owned(),
            },
        );
        let snapshot = SourceSnapshot {
            repositories: Vec::new(),
            configuration_digest: "d".to_owned(),
            open_gwz_merge: None,
        };
        assert!(ports.recheck_source(&snapshot).is_err());
        assert!(ports.recheck_source(&snapshot).is_ok());
        assert_eq!(ports.calls().len(), 3);
    }

    #[test]
    fn the_destination_fake_answers_absent_then_complete() {
        let mut ports = RecordingInstallPorts::new();
        let first = ports.observe_destination(Path::new("/dest")).unwrap();
        assert_eq!(first, DestinationObservation::absent());
        assert!(!first.exists);
        let second = ports.observe_destination(Path::new("/dest")).unwrap();
        assert_eq!(second, DestinationObservation::complete());
        assert!(crate::completion_faults(&second).is_empty());
        assert!(!crate::completion_faults(&first).is_empty());
    }

    /// The journal is only evidence if an out-of-order call really is
    /// refused, so drive the violations directly.
    #[test]
    fn the_journal_catches_every_ordering_violation_it_claims_to() {
        let journal = Journal::new();
        journal.record(InstallEvent::Allocate);
        assert_eq!(journal.violations().len(), 1, "row before the inventory");
        assert!(journal.violations()[0].contains("complete inventory"));

        let journal = Journal::new();
        for event in [
            InstallEvent::SnapshotSource,
            InstallEvent::ObserveDestination,
            InstallEvent::Allocate,
            InstallEvent::AllocateDestination,
            InstallEvent::CopyTree,
            InstallEvent::InstallPointer,
            InstallEvent::RecaptureConfiguration,
        ] {
            journal.record(event);
        }
        assert!(
            journal.violations().is_empty(),
            "{:?}",
            journal.violations()
        );
        journal.record(InstallEvent::PublishManifest);
        assert_eq!(journal.violations().len(), 1, "manifest before the recheck");
        journal.record(InstallEvent::RecheckSource);
        assert_eq!(journal.violations().len(), 2, "a step after the manifest");
        journal.record(InstallEvent::MarkReady);
        journal.record(InstallEvent::RecordError);
        assert_eq!(
            journal.violations().len(),
            2,
            "a diagnostic on the retained row may follow a failed ready"
        );
        journal.record(InstallEvent::RecaptureConfiguration);
        assert_eq!(journal.violations().len(), 3, "nothing else follows ready");
    }

    #[test]
    fn the_journal_refuses_a_build_before_the_row_and_a_ready_before_the_manifest() {
        let journal = Journal::new();
        journal.record(InstallEvent::SnapshotSource);
        journal.record(InstallEvent::ObserveDestination);
        journal.record(InstallEvent::CopyTree);
        assert_eq!(journal.violations().len(), 1, "copy before the row");
        let journal = Journal::new();
        journal.record(InstallEvent::SnapshotSource);
        journal.record(InstallEvent::ObserveDestination);
        journal.record(InstallEvent::Allocate);
        journal.record(InstallEvent::AllocateDestination);
        journal.record(InstallEvent::InstallPointer);
        assert_eq!(journal.violations().len(), 1, "pointer before the build");
        journal.record(InstallEvent::MarkReady);
        assert_eq!(journal.violations().len(), 2, "ready before the manifest");
    }
}
