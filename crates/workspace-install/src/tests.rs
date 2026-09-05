//! Orchestration tests: the four ordered steps of design §4, every
//! pre-reservation refusal, and what a failure leaves behind.
//!
//! Every collaborator is a fake, and all three of them write to one
//! [`Journal`], so each test asserts the exact ordered event vector as well
//! as its own outcome. `journal.violations()` is the ordering property
//! itself; the journal's teeth are tested in `test_support`.

use std::path::{Path, PathBuf};

use gwz_copy_contract::{
    CancelFlag, CopyError, CopyErrorCategory, CopyMode, CopyReport, Exclusion, NeverCancelled,
    contract_tests::ScriptedTreeCopier,
};
use gwz_family_model::{
    AllocationId, CloneMode, FamilyId, MemberKind, MemberName, MemberState, PointerObservation,
    Refusal, normalize_member_path,
};
use gwz_family_store_contract::contract_tests::{InMemoryFamilyStore, InMemorySession};
use gwz_family_store_contract::{
    FamilyLocation, FamilyObservation, FamilySession, FamilyStore, StoreError, StoreOperation,
};
use gwz_repo_contract::{
    HeadState, LayoutError, LayoutHazard, ObjectFormat, ObjectId, RepoKey, RepositoryInfo,
};

use crate::test_support::{
    InstallEvent, Journal, JournalCopier, JournalSession, RecordingInstallPorts,
};
use crate::{
    CapturedRepository, CompletionFault, ConfigurationReport, DestinationObservation,
    InstallEffect, InstallError, InstallPortError, InstallRefusal, InstallRequest, InstallStep,
    ManifestReceipt, SourceSnapshot, install,
};

const ROOT: &str = "/root";
/// The row records `../ws-A`, and the store resolves the destination
/// against the root: this is that spelling, which the store accepts.
const DEST: &str = "/root/../ws-A";

fn request(mode: CloneMode) -> InstallRequest {
    InstallRequest {
        name: MemberName::parse("A").unwrap(),
        root: PathBuf::from(ROOT),
        source: PathBuf::from(ROOT),
        destination: PathBuf::from(DEST),
        path: normalize_member_path("../ws-A").unwrap(),
        source_path: None,
        allocation: AllocationId::new("alloc-A").unwrap(),
        mode,
        branch: None,
        exclusions: vec![
            Exclusion::RelativePath(PathBuf::from(".gwz/local-family.yml")),
            Exclusion::RelativePath(PathBuf::from(".gwz/merge")),
        ],
        copy_mode: CopyMode::Auto,
    }
}

fn oid(byte: u8) -> ObjectId {
    ObjectId::from_bytes(ObjectFormat::Sha1, &[byte; 20]).unwrap()
}

fn repository(key: RepoKey, path: &str, branch: &str) -> CapturedRepository {
    CapturedRepository {
        key,
        info: RepositoryInfo {
            path: PathBuf::from(path),
            git_dir: PathBuf::from(path).join(".git"),
            common_dir: PathBuf::from(path).join(".git"),
            bare: false,
            object_format: ObjectFormat::Sha1,
            head: HeadState::Attached {
                branch: branch.to_owned(),
                target: oid(1),
            },
        },
        head: Some(oid(1)),
        branches: vec![branch.to_owned()],
        remotes: vec!["origin".to_owned()],
    }
}

/// Root plus one member, both frozen at a recorded HEAD: the one captured
/// vector every destination member uses (design §4.2).
fn snapshot() -> SourceSnapshot {
    SourceSnapshot {
        repositories: vec![
            repository(RepoKey::Root, ROOT, "main"),
            repository(
                RepoKey::Member {
                    id: "mem_app".to_owned(),
                },
                "/root/app",
                "main",
            ),
        ],
        configuration_digest: "sha256:frozen".to_owned(),
        open_gwz_merge: None,
    }
}

fn copy_report() -> CopyReport {
    CopyReport {
        native_files: 3,
        ordinary_files: 1,
        directories: 2,
        symlinks: 0,
        logical_bytes: 4096,
        warnings: Vec::new(),
    }
}

/// A founded family whose root holds the index, plus the shared journal.
struct Harness {
    store: InMemoryFamilyStore,
    session: InMemorySession,
    journal: Journal,
    copier: ScriptedTreeCopier,
}

fn founded() -> Harness {
    let store = InMemoryFamilyStore::new();
    let mut session = store.try_lock(&FamilyLocation::new(ROOT)).unwrap();
    session
        .found(
            FamilyId::new("fam").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        )
        .unwrap();
    let copier = ScriptedTreeCopier::new();
    copier.script(Ok(copy_report()));
    Harness {
        store,
        session,
        journal: Journal::new(),
        copier,
    }
}

impl Harness {
    fn ports(&self) -> RecordingInstallPorts {
        let mut ports = RecordingInstallPorts::with_journal(&self.journal);
        ports.snapshot(snapshot());
        ports
    }

    fn view(&self) -> gwz_family_model::FamilyView {
        match self
            .store
            .read_view(&FamilyLocation::new(ROOT))
            .expect("the index is readable")
        {
            FamilyObservation::Family { view, .. } => view,
            FamilyObservation::NoFamily => panic!("the family was founded"),
        }
    }

    fn row_state(&self) -> Option<(MemberState, Option<String>)> {
        self.view()
            .member("A")
            .map(|(_, row)| (row.state, row.last_error.clone()))
    }
}

// ---------------------------------------------------------------- step order

#[test]
fn verbatim_runs_the_four_steps_in_order_and_publishes_the_manifest_last() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let report = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect("the install completes");

    assert_eq!(
        harness.journal.events(),
        vec![
            // 1: aggregate the inventory and the destination, then the model.
            InstallEvent::SnapshotSource,
            InstallEvent::ObserveDestination,
            InstallEvent::Reread,
            // 2: reserve, then allocate the destination.
            InstallEvent::Allocate,
            InstallEvent::AllocateDestination,
            // 3: build, install metadata, check, recheck, recapture.
            InstallEvent::CopyTree,
            InstallEvent::InstallPointer,
            InstallEvent::ObserveDestination,
            InstallEvent::RecheckSource,
            InstallEvent::RecaptureConfiguration,
            // 4: manifest last, then ready.
            InstallEvent::PublishManifest,
            InstallEvent::MarkReady,
        ]
    );
    assert!(
        harness.journal.violations().is_empty(),
        "{:?}",
        harness.journal.violations()
    );
    assert_eq!(
        report.effects,
        vec![
            InstallEffect::RowAllocated,
            InstallEffect::DestinationAllocated,
            InstallEffect::TreeCopied,
            InstallEffect::PointerInstalled,
            InstallEffect::ConfigurationInstalled,
            InstallEffect::ManifestPublished,
            InstallEffect::RowReady,
        ]
    );
    assert_eq!(report.copy, Some(copy_report()));
    assert_eq!(harness.row_state(), Some((MemberState::Ready, None)));
    assert_eq!(
        harness.store.pointers().get(Path::new("/ws-A")),
        Some(&PathBuf::from(ROOT)),
        "the pointer is installed at the row's resolved path"
    );

    let copied = harness.copier.calls();
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].source, PathBuf::from(ROOT));
    assert_eq!(copied[0].destination, PathBuf::from(DEST));
    assert_eq!(
        copied[0].exclusions,
        request(CloneMode::Verbatim).exclusions,
        "exclusions are handed to the copier, which applies them while traversing"
    );
}

#[test]
fn clean_constructs_from_one_freeze_vector_covering_every_member_and_never_copies() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let mut wanted = request(CloneMode::Clean);
    wanted.branch = Some("lane/agent-17".to_owned());

    let report = install(&wanted, &mut session, &copier, &mut ports, &NeverCancelled)
        .expect("the install completes");

    assert_eq!(
        harness.journal.events(),
        vec![
            InstallEvent::SnapshotSource,
            InstallEvent::ObserveDestination,
            InstallEvent::Reread,
            InstallEvent::Allocate,
            InstallEvent::AllocateDestination,
            InstallEvent::ConstructRepositories,
            InstallEvent::InstallPointer,
            InstallEvent::ObserveDestination,
            InstallEvent::RecheckSource,
            InstallEvent::RecaptureConfiguration,
            InstallEvent::PublishManifest,
            InstallEvent::MarkReady,
        ]
    );
    assert!(harness.journal.violations().is_empty());
    assert_eq!(
        report.effects,
        vec![
            InstallEffect::RowAllocated,
            InstallEffect::DestinationAllocated,
            InstallEffect::RepositoriesConstructed,
            InstallEffect::PointerInstalled,
            InstallEffect::ConfigurationInstalled,
            InstallEffect::ManifestPublished,
            InstallEffect::RowReady,
        ]
    );
    assert_eq!(report.copy, None, "clean constructs, it never copies");
    assert!(harness.copier.calls().is_empty());

    let built = ports.construction_requests();
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].snapshot, snapshot(), "one captured freeze vector");
    assert!(
        built[0]
            .snapshot
            .repositories
            .iter()
            .any(|repo| repo.key == RepoKey::Root),
        "including the root repository"
    );
    assert_eq!(built[0].branch.as_deref(), Some("lane/agent-17"));
    assert_eq!(built[0].destination, PathBuf::from(DEST));
    for plan in ports.plans() {
        assert_eq!(plan.snapshot, snapshot(), "the same vector recaptures");
        assert_eq!(plan.branch.as_deref(), Some("lane/agent-17"));
    }
}

#[test]
fn a_bare_hub_records_a_bare_row() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    install(
        &request(CloneMode::Bare),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect("the install completes");

    let view = harness.view();
    let (_, row) = view.member("A").expect("the row exists");
    assert_eq!(row.kind, MemberKind::Bare);
    assert_eq!(row.mode, CloneMode::Bare);
    assert_eq!(row.state, MemberState::Ready);
    assert_eq!(row.source_path, ".", "the source is the root");
}

// ------------------------------------------------- step 1: refuse, aggregate

/// Nothing may be written while a refusal is possible, so every refusal
/// asserts the index is untouched.
fn assert_refused_before_reservation(
    harness: &Harness,
    failure: &crate::InstallFailure,
) -> Vec<InstallRefusal> {
    assert!(failure.effects.is_empty(), "{:?}", failure.effects);
    assert!(
        harness.view().members.is_empty(),
        "no row is written before the checks pass"
    );
    assert!(
        !harness.journal.seen(InstallEvent::Allocate),
        "reservation never ran"
    );
    assert!(harness.journal.violations().is_empty());
    match &failure.error {
        InstallError::Refused(refusals) => refusals.clone(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn every_source_layout_hazard_refuses_before_reservation_and_fetches_nothing() {
    let hazards = vec![
        LayoutHazard::GitFile {
            path: PathBuf::from("/root/app/.git"),
        },
        LayoutHazard::ExternalCommonDir {
            path: PathBuf::from("/elsewhere/.git"),
        },
        LayoutHazard::Alternates {
            path: PathBuf::from("/root/app/.git/objects/info/alternates"),
        },
        LayoutHazard::EscapingMetadataLink {
            path: PathBuf::from("/root/app/.git/objects"),
            target: PathBuf::from("/elsewhere/objects"),
        },
        LayoutHazard::EscapingConfig {
            key: "core.hooksPath".to_owned(),
            value: "/outside/hooks".to_owned(),
        },
        LayoutHazard::UnresolvableConfig {
            key: "include.path".to_owned(),
            detail: "unreadable".to_owned(),
        },
        LayoutHazard::PartialClone {
            detail: "promised objects are not local".to_owned(),
        },
        LayoutHazard::EnvironmentOverride {
            variable: "GIT_DIR".to_owned(),
        },
    ];
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.fail_next(
        InstallEvent::SnapshotSource,
        InstallPortError::Layout(LayoutError::Unsupported {
            path: PathBuf::from("/root/app"),
            hazards: hazards.clone(),
        }),
    );
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("an unsupported source layout refuses");

    assert_eq!(failure.step, InstallStep::InventorySource);
    let refusals = assert_refused_before_reservation(&harness, &failure);
    assert_eq!(
        refusals,
        vec![InstallRefusal::SourceLayout(LayoutError::Unsupported {
            path: PathBuf::from("/root/app"),
            hazards,
        })],
        "every hazard the inspector aggregated survives into the refusal"
    );
    assert!(harness.copier.calls().is_empty(), "nothing is copied");
    assert!(harness.store.pointers().is_empty());
}

#[test]
fn name_path_allocation_destination_and_layout_refusals_are_reported_together() {
    let mut harness = founded();
    // `A` holds the name and the path; `B` holds the allocation value.
    for (name, path, allocation) in [("A", "../ws-A", "alloc-other"), ("B", "../ws-B", "alloc-A")] {
        harness
            .session
            .apply(&gwz_family_model::FamilyChange::Allocate {
                name: MemberName::parse(name).unwrap(),
                row: gwz_family_model::MemberRow {
                    path: path.to_owned(),
                    kind: MemberKind::Checkout,
                    state: MemberState::Creating,
                    allocation_id: AllocationId::new(allocation).unwrap(),
                    source_path: ".".to_owned(),
                    mode: CloneMode::Verbatim,
                    last_error: None,
                },
            })
            .expect("the earlier reservations are legal");
    }
    let mut ports = harness.ports();
    ports.fail_next(
        InstallEvent::SnapshotSource,
        InstallPortError::Layout(LayoutError::NotARepository {
            path: PathBuf::from(ROOT),
        }),
    );
    ports.observe(DestinationObservation {
        exists: true,
        nonempty: true,
        is_workspace: true,
        ..DestinationObservation::absent()
    });
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("every check refuses");

    assert_eq!(failure.step, InstallStep::InventorySource);
    assert!(failure.effects.is_empty());
    let InstallError::Refused(refusals) = &failure.error else {
        panic!("expected refusals, got {:?}", failure.error);
    };
    assert_eq!(
        refusals,
        &vec![
            InstallRefusal::SourceLayout(LayoutError::NotARepository {
                path: PathBuf::from(ROOT)
            }),
            InstallRefusal::DestinationNotEmpty {
                destination: PathBuf::from(DEST)
            },
            InstallRefusal::DestinationIsWorkspace {
                destination: PathBuf::from(DEST)
            },
            InstallRefusal::Family(Refusal::NameCollision {
                name: MemberName::parse("A").unwrap(),
                holder_path: "../ws-A".to_owned(),
            }),
            InstallRefusal::Family(Refusal::PathCollision {
                path: "../ws-A".to_owned(),
                holder: "A".to_owned(),
            }),
            InstallRefusal::Family(Refusal::AllocationCollision {
                name: MemberName::parse("A").unwrap(),
                holder: "B".to_owned(),
            }),
        ],
        "one aggregate, not the first failure"
    );
    assert!(failure.to_string().contains("is not empty"));
    // The pre-existing row is untouched: nothing was reserved or recorded.
    assert_eq!(
        harness.row_state(),
        Some((MemberState::Creating, None)),
        "the aggregate refusal writes nothing, not even a diagnostic"
    );
}

#[test]
fn a_nonempty_destination_and_a_destination_that_is_already_a_workspace_each_refuse() {
    for (label, observation, expected) in [
        (
            "nonempty",
            DestinationObservation {
                exists: true,
                nonempty: true,
                ..DestinationObservation::absent()
            },
            InstallRefusal::DestinationNotEmpty {
                destination: PathBuf::from(DEST),
            },
        ),
        (
            "already a workspace",
            DestinationObservation {
                exists: true,
                is_workspace: true,
                ..DestinationObservation::absent()
            },
            InstallRefusal::DestinationIsWorkspace {
                destination: PathBuf::from(DEST),
            },
        ),
    ] {
        let mut harness = founded();
        let mut ports = harness.ports();
        ports.observe(observation);
        let mut session = JournalSession::new(&mut harness.session, &harness.journal);
        let copier = JournalCopier::new(&harness.copier, &harness.journal);
        let failure = install(
            &request(CloneMode::Verbatim),
            &mut session,
            &copier,
            &mut ports,
            &NeverCancelled,
        )
        .expect_err(label);
        assert_eq!(failure.step, InstallStep::Reserve, "{label}");
        assert_eq!(
            assert_refused_before_reservation(&harness, &failure),
            vec![expected],
            "{label}"
        );
    }
}

#[test]
fn verbatim_refuses_an_open_gwz_merge_and_clean_does_not() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut open = snapshot();
    open.open_gwz_merge = Some("merge `m1` is open in @root".to_owned());
    ports.snapshot(open.clone());
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("verbatim refuses an open gwz merge");

    assert_eq!(failure.step, InstallStep::Reserve);
    assert_eq!(
        assert_refused_before_reservation(&harness, &failure),
        vec![InstallRefusal::SourceOpenMerge {
            detail: "merge `m1` is open in @root".to_owned()
        }]
    );
    assert!(failure.to_string().contains("--clean"));

    // The same source, cloned clean, is admitted: clean does not inherit the
    // open merge (design §4.1, §8.3).
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.snapshot(open);
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    install(
        &request(CloneMode::Clean),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect("clean is the documented escape");
    assert_eq!(harness.row_state(), Some((MemberState::Ready, None)));
}

#[test]
fn a_branch_that_already_exists_refuses_before_the_creating_row_aggregating() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let mut wanted = request(CloneMode::Clean);
    wanted.branch = Some("main".to_owned());

    let failure = install(&wanted, &mut session, &copier, &mut ports, &NeverCancelled)
        .expect_err("`-b main` collides in both repositories");

    assert_eq!(failure.step, InstallStep::Reserve);
    assert_eq!(
        assert_refused_before_reservation(&harness, &failure),
        vec![
            InstallRefusal::BranchExists {
                branch: "main".to_owned(),
                member: RepoKey::Root,
            },
            InstallRefusal::BranchExists {
                branch: "main".to_owned(),
                member: RepoKey::Member {
                    id: "mem_app".to_owned()
                },
            },
        ],
        "aggregating over every repository, before the creating row"
    );
}

#[test]
fn a_branch_is_refused_for_verbatim_and_a_missing_root_capture_refuses_clean() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let mut wanted = request(CloneMode::Verbatim);
    wanted.branch = Some("lane/x".to_owned());
    let failure = install(&wanted, &mut session, &copier, &mut ports, &NeverCancelled)
        .expect_err("verbatim copies the source's branches as they sit");
    assert_eq!(
        assert_refused_before_reservation(&harness, &failure),
        vec![InstallRefusal::BranchNotSupported {
            branch: "lane/x".to_owned(),
            mode: CloneMode::Verbatim,
        }]
    );

    let mut harness = founded();
    let mut ports = harness.ports();
    let mut rootless = snapshot();
    rootless
        .repositories
        .retain(|repo| repo.key != RepoKey::Root);
    ports.snapshot(rootless);
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let failure = install(
        &request(CloneMode::Clean),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("every member, including the root, uses the captured vector");
    assert_eq!(
        assert_refused_before_reservation(&harness, &failure),
        vec![InstallRefusal::RootNotCaptured]
    );
}

#[test]
fn a_name_that_is_already_a_git_remote_refuses() {
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.snapshot(snapshot());
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let mut wanted = request(CloneMode::Verbatim);
    wanted.name = MemberName::parse("upstream").unwrap();
    wanted.path = normalize_member_path("../ws-upstream").unwrap();
    let mut with_remote = snapshot();
    with_remote.repositories[1].remotes = vec!["origin".to_owned(), "upstream".to_owned()];
    ports.snapshot(with_remote);

    let failure = install(&wanted, &mut session, &copier, &mut ports, &NeverCancelled)
        .expect_err("the name is already a git remote");

    assert_eq!(
        assert_refused_before_reservation(&harness, &failure),
        vec![InstallRefusal::NameIsRemote(
            gwz_family_model::RemoteNameCollision {
                name: "upstream".to_owned(),
                member: "mem_app".to_owned(),
            }
        )]
    );
}

// ------------------------------------- steps 2-4: what a failure leaves behind

#[test]
fn a_copy_failure_retains_the_destination_and_leaves_an_incomplete_row() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let partial = CopyReport {
        ordinary_files: 2,
        ..CopyReport::default()
    };
    let copier = ScriptedTreeCopier::new();
    copier.script(Err(CopyError::refused_with(
        "app/big.bin",
        CopyErrorCategory::DestinationUnwritable,
        "no space left on device",
        partial,
    )));
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("the copy fails");

    assert_eq!(failure.step, InstallStep::CopyTree);
    assert!(matches!(failure.error, InstallError::Copy(_)));
    assert_eq!(
        failure.effects,
        vec![
            InstallEffect::RowAllocated,
            InstallEffect::DestinationAllocated,
            InstallEffect::ErrorRecorded,
        ],
        "the destination is allocated and retained; nothing is undone"
    );
    let (state, last_error) = harness.row_state().expect("the row is retained");
    assert_eq!(
        state,
        MemberState::Creating,
        "never promoted, never removed"
    );
    let detail = last_error.expect("the row carries its diagnostic");
    assert!(detail.contains("copy tree"), "{detail}");
    assert!(detail.contains("no space left on device"), "{detail}");
    assert!(
        harness.store.pointers().is_empty(),
        "no pointer for an unbuilt destination"
    );
    assert!(!harness.journal.seen(InstallEvent::PublishManifest));
    assert!(harness.journal.violations().is_empty());
}

#[test]
fn source_drift_at_the_recheck_stops_publication() {
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.fail_next(
        InstallEvent::RecheckSource,
        InstallPortError::Drift {
            detail: "@root HEAD moved since the snapshot".to_owned(),
        },
    );
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("observed drift stops the install");

    assert_eq!(failure.step, InstallStep::RecheckSource);
    assert!(matches!(
        failure.error,
        InstallError::Source(ref error) if matches!(**error, InstallPortError::Drift { .. })
    ));
    assert_eq!(
        failure.effects,
        vec![
            InstallEffect::RowAllocated,
            InstallEffect::DestinationAllocated,
            InstallEffect::TreeCopied,
            InstallEffect::PointerInstalled,
            InstallEffect::ErrorRecorded,
        ]
    );
    assert!(
        !harness.journal.seen(InstallEvent::RecaptureConfiguration)
            && !harness.journal.seen(InstallEvent::PublishManifest),
        "nothing is published after drift"
    );
    assert_eq!(
        harness.row_state().map(|(state, _)| state),
        Some(MemberState::Creating)
    );
}

#[test]
fn a_metadata_failure_between_the_manifest_and_ready_leaves_the_row_creating() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    session.fail_next(
        InstallEvent::MarkReady,
        StoreError::Io {
            operation: StoreOperation::WriteIndex,
            path: PathBuf::from("/root/.gwz/local-family.yml"),
            detail: "read-only file system".to_owned(),
        },
    );
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("the index write fails after the manifest");

    assert_eq!(failure.step, InstallStep::MarkReady);
    assert!(matches!(failure.error, InstallError::Store(_)));
    assert_eq!(
        failure.effects,
        vec![
            InstallEffect::RowAllocated,
            InstallEffect::DestinationAllocated,
            InstallEffect::TreeCopied,
            InstallEffect::PointerInstalled,
            InstallEffect::ConfigurationInstalled,
            InstallEffect::ManifestPublished,
            InstallEffect::ErrorRecorded,
        ],
        "the manifest is published and the row is still not ready"
    );
    let (state, last_error) = harness.row_state().expect("the row is retained");
    assert_eq!(
        state,
        MemberState::Creating,
        "an apparently complete destination is not promoted"
    );
    assert!(last_error.expect("diagnostic").contains("mark ready"));
    assert!(harness.journal.violations().is_empty());
}

#[test]
fn the_absent_at_ready_column_is_checked_against_what_the_install_leaves() {
    let residual = vec![
        PathBuf::from(".gwz/local-family.lock"),
        PathBuf::from(".gwz/catalog-final"),
        PathBuf::from(".gwz/checked-artifacts"),
        PathBuf::from("app/.git/worktrees"),
    ];
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.observe(DestinationObservation::absent());
    ports.observe(DestinationObservation {
        family_index: true,
        pointer: PointerObservation::OtherFamily,
        merge_store: true,
        residual: residual.clone(),
        dependencies: vec!["core.hooksPath names /outside/hooks".to_owned()],
        ..DestinationObservation::complete()
    });
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("the destination is not complete");

    assert_eq!(failure.step, InstallStep::CheckDestination);
    let InstallError::Incomplete(faults) = &failure.error else {
        panic!("expected completion faults, got {:?}", failure.error);
    };
    assert_eq!(
        faults,
        &vec![
            CompletionFault::ConflictingMetadata,
            CompletionFault::MergeStorePresent,
            CompletionFault::ResidualPath {
                path: residual[0].clone()
            },
            CompletionFault::ResidualPath {
                path: residual[1].clone()
            },
            CompletionFault::ResidualPath {
                path: residual[2].clone()
            },
            CompletionFault::ResidualPath {
                path: residual[3].clone()
            },
            CompletionFault::NotIndependent {
                detail: "core.hooksPath names /outside/hooks".to_owned()
            },
        ]
    );
    assert!(!harness.journal.seen(InstallEvent::PublishManifest));
    assert_eq!(
        harness.row_state().map(|(state, _)| state),
        Some(MemberState::Creating)
    );
}

#[test]
fn a_missing_pointer_and_a_bare_family_index_are_each_a_completion_fault() {
    for (label, observation, expected) in [
        (
            "no pointer",
            DestinationObservation {
                pointer: PointerObservation::Absent,
                ..DestinationObservation::complete()
            },
            CompletionFault::PointerMissing,
        ),
        (
            "an index and no pointer",
            DestinationObservation {
                family_index: true,
                pointer: PointerObservation::Absent,
                ..DestinationObservation::complete()
            },
            CompletionFault::FamilyIndexPresent,
        ),
        (
            "a pointer to another family",
            DestinationObservation {
                pointer: PointerObservation::OtherFamily,
                ..DestinationObservation::complete()
            },
            CompletionFault::PointerInvalid {
                observed: PointerObservation::OtherFamily,
            },
        ),
    ] {
        let mut harness = founded();
        let mut ports = harness.ports();
        ports.observe(DestinationObservation::absent());
        ports.observe(observation);
        let mut session = JournalSession::new(&mut harness.session, &harness.journal);
        let copier = JournalCopier::new(&harness.copier, &harness.journal);
        let failure = install(
            &request(CloneMode::Verbatim),
            &mut session,
            &copier,
            &mut ports,
            &NeverCancelled,
        )
        .expect_err(label);
        assert_eq!(
            failure.error,
            InstallError::Incomplete(vec![expected]),
            "{label}"
        );
    }
}

#[test]
fn generated_configuration_changes_are_reported_and_a_missed_recapture_stops_clean() {
    let changes = vec![
        PathBuf::from("gwz.conf/gwz.lock.yml"),
        PathBuf::from("gwz.conf/gwz.yml"),
    ];
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.recapture(ConfigurationReport {
        lock_recaptured: true,
        generated_changes: changes.clone(),
    });
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let report = install(
        &request(CloneMode::Clean),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect("the install completes and reports the generated changes");
    assert_eq!(
        report.configuration,
        Some(ConfigurationReport {
            lock_recaptured: true,
            generated_changes: changes,
        }),
        "reported, not hidden in a commit"
    );

    // Without the recapture, clean is incomplete and never flips ready.
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.recapture(ConfigurationReport {
        lock_recaptured: false,
        generated_changes: Vec::new(),
    });
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);
    let failure = install(
        &request(CloneMode::Clean),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("dest-complete includes dest lock/HEAD agreement");
    assert_eq!(failure.step, InstallStep::RecaptureConfiguration);
    assert_eq!(
        failure.error,
        InstallError::Incomplete(vec![CompletionFault::LockNotRecaptured])
    );
    assert!(!harness.journal.seen(InstallEvent::PublishManifest));
    assert_eq!(
        harness.row_state().map(|(state, _)| state),
        Some(MemberState::Creating)
    );
}

#[test]
fn a_copied_marker_that_was_not_regenerated_leaves_the_row_creating() {
    let mut harness = founded();
    let mut ports = harness.ports();
    ports.receipt(ManifestReceipt {
        marker_regenerated: false,
    });
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("a marker vouching for superseded bytes is never accepted");

    assert_eq!(failure.step, InstallStep::PublishManifest);
    assert_eq!(
        failure.error,
        InstallError::Incomplete(vec![CompletionFault::MarkerNotRegenerated])
    );
    assert!(failure.effects.contains(&InstallEffect::ManifestPublished));
    assert_eq!(
        harness.row_state().map(|(state, _)| state),
        Some(MemberState::Creating)
    );
}

#[test]
fn a_pointer_failure_reports_the_stores_partial_effects_and_keeps_the_row() {
    let mut harness = founded();
    harness.store.fail_next(StoreOperation::WritePointer);
    let mut ports = harness.ports();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("the pointer write fails after the marker");

    assert_eq!(failure.step, InstallStep::InstallPointer);
    let InstallError::Store(error) = &failure.error else {
        panic!("expected a store error, got {:?}", failure.error);
    };
    assert!(
        matches!(**error, StoreError::Partial { .. }),
        "the store's own partial effects reach the caller: {error}"
    );
    assert!(!failure.effects.contains(&InstallEffect::PointerInstalled));
    assert!(failure.effects.contains(&InstallEffect::TreeCopied));
    assert_eq!(
        harness.row_state().map(|(state, _)| state),
        Some(MemberState::Creating)
    );
}

#[test]
fn cancellation_stops_before_the_next_effect_and_leaves_everything_in_place() {
    let mut harness = founded();
    let mut ports = harness.ports();
    let cancellation = CancelFlag::new();
    cancellation.cancel();
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &cancellation,
    )
    .expect_err("a cancelled install stops");

    assert_eq!(failure.step, InstallStep::Reserve);
    assert_eq!(failure.error, InstallError::Cancelled);
    assert!(
        failure.effects.is_empty(),
        "cancellation before the reservation writes nothing"
    );
    assert!(harness.view().members.is_empty());
    assert!(harness.journal.violations().is_empty());
}

#[test]
fn an_unimplemented_port_is_an_error_not_a_refusal_and_writes_nothing() {
    let mut harness = founded();
    let mut ports = RecordingInstallPorts::with_journal(&harness.journal);
    let mut session = JournalSession::new(&mut harness.session, &harness.journal);
    let copier = JournalCopier::new(&harness.copier, &harness.journal);

    let failure = install(
        &request(CloneMode::Verbatim),
        &mut session,
        &copier,
        &mut ports,
        &NeverCancelled,
    )
    .expect_err("an unscripted inventory cannot admit the create");

    assert_eq!(failure.step, InstallStep::InventorySource);
    assert!(matches!(
        failure.error,
        InstallError::Source(ref error) if matches!(**error, InstallPortError::Unimplemented { .. })
    ));
    assert!(failure.effects.is_empty());
    assert!(harness.view().members.is_empty());
    assert_eq!(
        harness.journal.events(),
        vec![InstallEvent::SnapshotSource],
        "a port error stops before the rest of the aggregate"
    );
}
