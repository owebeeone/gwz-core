//! Shared fixtures: the in-memory family, the scripted session and the
//! evidence builders every disposal test draws on.

use super::*;

pub(crate) const ROOT: &str = "/fam/root";
pub(crate) const WS_A: &str = "/fam/ws-A";

pub(crate) fn name(text: &str) -> MemberName {
    MemberName::parse(text).unwrap()
}

pub(crate) fn allocation() -> AllocationId {
    AllocationId::new("alloc-A").unwrap()
}

pub(crate) fn row(path: &str) -> MemberRow {
    MemberRow {
        path: path.to_owned(),
        kind: MemberKind::Checkout,
        state: MemberState::Creating,
        allocation_id: allocation(),
        source_path: ".".to_owned(),
        mode: CloneMode::Verbatim,
        last_error: None,
        owner: None,
    }
}

pub(crate) type Session = <InMemoryFamilyStore as FamilyStore>::Session;

/// A founded family holding `A` at `path` in `state`, with its pointer
/// and marker installed. `Creating` leaves the row as allocated.
pub(crate) fn family(path: &str, state: MemberState) -> (InMemoryFamilyStore, Session) {
    let store = InMemoryFamilyStore::new();
    let mut session = store.try_lock(&FamilyLocation::new(ROOT)).unwrap();
    session
        .found(
            FamilyId::new("fam").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        )
        .unwrap();
    session
        .apply(&FamilyChange::Allocate {
            name: name("A"),
            row: row(path),
        })
        .unwrap();
    session
        .install_pointer(&name("A"), &PathBuf::from(ROOT).join(path))
        .unwrap();
    if state != MemberState::Creating {
        session
            .apply(&FamilyChange::MarkReady {
                name: name("A"),
                expected_allocation: allocation(),
            })
            .unwrap();
    }
    if state == MemberState::Disposing {
        session
            .apply(&FamilyChange::MarkDisposing {
                name: name("A"),
                expected_allocation: allocation(),
            })
            .unwrap();
    }
    (store, session)
}

/// The ordinary case: `A` is ready at `../ws-A`.
pub(crate) fn ready() -> (InMemoryFamilyStore, Session) {
    family("../ws-A", MemberState::Ready)
}

pub(crate) fn delete(waivers: &[HazardWaiver]) -> DisposeRequest {
    DisposeRequest {
        name: name("A"),
        policy: DisposePolicy::Delete {
            waivers: waivers.to_vec(),
        },
        root: PathBuf::from(ROOT),
        cwd: PathBuf::from(ROOT),
    }
}

pub(crate) fn keep() -> DisposeRequest {
    DisposeRequest {
        policy: DisposePolicy::Keep,
        ..delete(&[])
    }
}

pub(crate) fn oid() -> ObjectId {
    ObjectId::parse_hex(
        ObjectFormat::Sha1,
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap()
}

pub(crate) fn present() -> TargetObservation {
    TargetObservation::Present {
        pointer: PointerObservation::Matches,
        marker: MarkerObservation::Matches,
    }
}

pub(crate) fn repository(key: RepoKey, path: &str) -> RepositoryEvidence {
    let path = PathBuf::from(path);
    RepositoryEvidence {
        key,
        info: RepositoryInfo {
            git_dir: path.join(".git"),
            common_dir: path.join(".git"),
            path,
            bare: false,
            object_format: ObjectFormat::Sha1,
            head: HeadState::Attached {
                branch: "refs/heads/main".to_owned(),
                target: oid(),
            },
        },
        work: Observation::Known(WorkObservation::default()),
        gwz: GwzEvidence::default(),
        history: Observation::Known(ProtectedRoots::default()),
    }
}

/// One clean, known repository at the target, present with matching
/// metadata: the only shape ordinary deletion accepts.
pub(crate) fn clean_evidence() -> TargetEvidence {
    TargetEvidence {
        target: present(),
        repositories: vec![repository(RepoKey::Root, WS_A)],
        unknown: Vec::new(),
    }
}

pub(crate) fn scripted(evidence: TargetEvidence, history: HistoryAnswer) -> RecordingDisposalPorts {
    let mut ports = RecordingDisposalPorts::new();
    ports.evidence(evidence);
    ports.history(history);
    ports
}

/// Every refusal must leave the filesystem untouched.
pub(crate) fn assert_no_removal(ports: &RecordingDisposalPorts) {
    assert!(
        !ports
            .calls()
            .iter()
            .any(|call| matches!(call, DisposalCall::RemoveDirectory { .. })),
        "a refusal called the remover: {:?}",
        ports.calls()
    );
}

/// A session over an index handed back exactly as written, so disposal's
/// defences against a **decoded** index can be exercised: a hand-edited
/// or foreign `local-family.yml` can carry a row `validate_transition`
/// would never have created, and those rows must still refuse. The
/// contract-faithful `InMemoryFamilyStore` is used everywhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionCall {
    Reread,
    Apply(FamilyChange),
    RemovePointer(MemberName),
}

pub(crate) struct ScriptedSession {
    root: PathBuf,
    view: Option<FamilyView>,
    /// Members whose clone pointer stands at their recorded path.
    pointers: Vec<MemberName>,
    calls: Vec<SessionCall>,
    pub(crate) fail_apply: Option<StoreError>,
    pub(crate) fail_remove_pointer: Option<StoreError>,
}

impl ScriptedSession {
    /// A family whose rows are taken verbatim, with a pointer standing
    /// for every one of them.
    pub(crate) fn with_rows(rows: &[(&str, MemberRow)]) -> Self {
        let mut view = FamilyView::founded(
            FamilyId::new("fam").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        );
        let mut pointers = Vec::new();
        for (member, row) in rows {
            view.members.insert(name(member), row.clone());
            pointers.push(name(member));
        }
        Self {
            root: PathBuf::from(ROOT),
            view: Some(view),
            pointers,
            calls: Vec::new(),
            fail_apply: None,
            fail_remove_pointer: None,
        }
    }

    pub(crate) fn ready_at(path: &str) -> Self {
        Self::with_rows(&[(
            "A",
            MemberRow {
                state: MemberState::Ready,
                ..row(path)
            },
        )])
    }

    pub(crate) fn applied(&self) -> Vec<FamilyChange> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                SessionCall::Apply(change) => Some(change.clone()),
                _ => None,
            })
            .collect()
    }
}

impl FamilySession for ScriptedSession {
    fn root(&self) -> &Path {
        &self.root
    }

    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
        self.calls.push(SessionCall::Reread);
        Ok(self.view.clone())
    }

    fn found(
        &mut self,
        _family_id: FamilyId,
        _root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::WriteIndex,
        })
    }

    fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError> {
        self.calls.push(SessionCall::Apply(change.clone()));
        if let Some(error) = self.fail_apply.take() {
            return Err(error);
        }
        let view = self.view.clone().ok_or_else(|| StoreError::NoFamily {
            workspace: self.root.clone(),
        })?;
        // The store's own ordering guard: a row may not be removed while
        // this family's pointer still stands at its recorded path.
        if let FamilyChange::RemoveRow { name, .. } = change
            && self.pointers.contains(name)
        {
            return Err(StoreError::PointerStillInstalled {
                member: name.as_str().to_owned(),
                workspace: self.root.join(&view.members[name].path),
            });
        }
        let validated = gwz_family_model::validate_transition(&view, change)?;
        self.view = match change {
            FamilyChange::Disband => None,
            _ => Some(validated.next),
        };
        Ok(AppliedChange {
            effects: vec![MetadataEffect::IndexWritten],
            view: self.view.clone(),
        })
    }

    fn install_pointer(
        &mut self,
        _name: &MemberName,
        _destination: &Path,
    ) -> Result<AppliedChange, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::WritePointer,
        })
    }

    fn remove_pointer(&mut self, member: &MemberName) -> Result<AppliedChange, StoreError> {
        self.calls.push(SessionCall::RemovePointer(member.clone()));
        if let Some(error) = self.fail_remove_pointer.take() {
            return Err(error);
        }
        let view = self.view.clone().ok_or_else(|| StoreError::NoFamily {
            workspace: self.root.clone(),
        })?;
        let mut effects = Vec::new();
        if let Some(at) = self.pointers.iter().position(|held| held == member) {
            self.pointers.remove(at);
            effects.push(MetadataEffect::PointerRemoved {
                workspace: self.root.join(&view.members[member].path),
            });
        }
        Ok(AppliedChange {
            effects,
            view: Some(view),
        })
    }
}

pub(crate) fn dirty_work() -> Observation<WorkObservation> {
    Observation::Known(WorkObservation {
        entries: vec![WorkEntry {
            path: b"notes.txt".to_vec(),
            kind: WorkKind::Untracked,
            binary: Some(false),
        }],
        ..WorkObservation::default()
    })
}

pub(crate) fn open_merge() -> GwzEvidence {
    GwzEvidence {
        merge: EvidenceState::Open {
            detail: "merge gwz_merge_0001 is open".to_owned(),
        },
        ..GwzEvidence::default()
    }
}
