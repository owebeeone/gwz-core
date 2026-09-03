use super::*;
use crate::workspace_ops::tests::TempDir;

#[derive(Default)]
struct FakeBackend {
    calls: std::cell::RefCell<Vec<String>>,
    simulations: std::cell::RefCell<Vec<String>>,
    simulation_conflict: Option<&'static str>,
    dirty: Option<&'static str>,
    drift: Option<&'static str>,
    failure: Option<(FailurePoint, &'static str)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailurePoint {
    IsRepository,
    Status,
    Head,
    MergeState,
    MergeAnalysis,
    ReadRef,
}

impl FakeBackend {
    fn fail(&self, point: FailurePoint, path: &Path) -> ModelResult<()> {
        if self.failure == Some((point, key(path))) {
            let (code, message) = match point {
                FailurePoint::MergeState => (
                    ErrorCode::InvalidRequest,
                    "repository has an integration operation in progress: RebaseMerge",
                ),
                FailurePoint::MergeAnalysis => {
                    (ErrorCode::GitCommandFailed, "revspec 'feature/x' not found")
                }
                _ => (ErrorCode::IoError, "backend probe failed"),
            };
            return Err(ModelError::new(code, message));
        }
        Ok(())
    }
}

impl PlanningBackend for FakeBackend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        self.fail(FailurePoint::IsRepository, path)?;
        Ok(true)
    }
    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        self.fail(FailurePoint::Status, path)?;
        Ok(GitStatus {
            untracked: usize::from(self.dirty == Some(key(path))),
            ..GitStatus::clean()
        })
    }
    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        self.fail(FailurePoint::Head, path)?;
        Ok(GitHeadState {
            branch: Some("main".into()),
            commit: Some(format!("before-{}", key(path))),
            is_detached: false,
        })
    }
    fn merge_state(&self, path: &Path) -> ModelResult<Option<GitNativeMergeState>> {
        self.fail(FailurePoint::MergeState, path)?;
        if self.dirty == Some("integration") {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "rebase in progress",
            ));
        }
        Ok(None)
    }
    fn merge_analysis(&self, path: &Path, _: &str, _: &str) -> ModelResult<GitMergeAnalysis> {
        self.fail(FailurePoint::MergeAnalysis, path)?;
        self.calls.borrow_mut().push(key(path).to_owned());
        Ok(GitMergeAnalysis {
            target_branch: "main".into(),
            target_commit: format!("before-{}", key(path)),
            source_commit: format!("source-{}", key(path)),
            kind: GitMergeAnalysisKind::TrueMerge,
            commit_identity_required: true,
            prediction_complete: false,
        })
    }
    fn merge_simulate(&self, path: &Path, _: &str, _: &str) -> ModelResult<GitMergeSimulation> {
        let key = key(path);
        self.simulations.borrow_mut().push(key.to_owned());
        Ok(if self.simulation_conflict == Some(key) {
            GitMergeSimulation::Conflicts(vec![format!("{key}.txt")])
        } else {
            GitMergeSimulation::Clean
        })
    }
    fn read_ref(&self, path: &Path, _: &str) -> ModelResult<Option<String>> {
        self.fail(FailurePoint::ReadRef, path)?;
        Ok(Some(if self.drift == Some(key(path)) {
            "moved".into()
        } else {
            format!("before-{}", key(path))
        }))
    }
    fn read_file_at_commit(
        &self,
        _: &Path,
        _: &str,
        relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        Ok(match relative_path {
            artifact::LOCK_PATH => Some(b"baseline-lock".to_vec()),
            WORKSPACE_MANIFEST => Some(b"baseline-manifest".to_vec()),
            _ => None,
        })
    }
}

fn key(path: &Path) -> &str {
    if path.join("z").is_dir() && path.join("a").is_dir() {
        "root"
    } else {
        path.file_name().unwrap().to_str().unwrap()
    }
}

fn fixture() -> (TempDir, ManifestArtifact, LockArtifact) {
    let root = TempDir::new("merge-plan");
    for path in ["z", "a"] {
        fs::create_dir(root.path().join(path)).unwrap();
    }
    let manifest = ManifestArtifact::from_yaml("schema: gwz.workspace/v0\nworkspace:\n  id: ws_test\nmembers:\n- id: mem_z\n  path: z\n  type: git\n  source_id: src_z\n  active: true\n  remotes: []\n- id: mem_old\n  path: old\n  type: git\n  source_id: src_old\n  active: false\n  remotes: []\n- id: mem_a\n  path: a\n  type: git\n  source_id: src_a\n  active: true\n  remotes: []\n").unwrap();
    let lock = LockArtifact::from_yaml("schema: gwz.lock/v0\nworkspace_id: ws_test\nmanifest_schema: gwz.workspace/v0\nmembers:\n  mem_z:\n    path: z\n    source_id: src_z\n    source_kind: git\n    commit: before-z\n    branch: main\n    materialized: true\n  mem_a:\n    path: a\n    source_id: src_a\n    source_kind: git\n    commit: before-a\n    branch: main\n    materialized: true\n").unwrap();
    (root, manifest, lock)
}

fn request(selection: Option<crate::Selection>, dry_run: bool) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: crate::RequestMeta {
            request_id: "req".into(),
            schema_version: "gwz.v0".into(),
            selection,
            dry_run: dry_run.then_some(true),
            ..Default::default()
        },
        op: crate::MergeOp::Start,
        source_ref: Some("feature/x".into()),
        merge_id: None,
        mode: None,
        message: None,
        preserve: None,
        filesystem_strict: None,
    }
}

fn build(
    backend: &FakeBackend,
    fixture: &(TempDir, ManifestArtifact, LockArtifact),
    request: &crate::MergeRequest,
) -> ModelResult<MergePlan> {
    build_merge_plan(
        backend,
        fixture.0.path(),
        request,
        &fixture.1,
        &fixture.2,
        MergeBaseline {
            lock_sha256: format!("{:x}", Sha256::digest(b"baseline-lock")),
            manifest_sha256: format!("{:x}", Sha256::digest(b"baseline-manifest")),
            lock_yaml: Some("baseline-lock".to_owned()),
            manifest_yaml: Some("baseline-manifest".to_owned()),
            lock_commit_sha256: Some(format!("{:x}", Sha256::digest(b"baseline-lock"))),
            manifest_commit_sha256: Some(format!("{:x}", Sha256::digest(b"baseline-manifest"))),
            root_head: Some("before-root".into()),
            root_branch: Some("main".into()),
            extensions: Default::default(),
        },
    )
}

fn ids(plan: &MergePlan) -> Vec<&str> {
    plan.participants
        .iter()
        .map(|participant| participant.target_id.as_str())
        .collect()
}

#[test]
fn selection_freezes_active_members_in_manifest_order() {
    let fixture = fixture();
    let backend = FakeBackend::default();
    let plan = build(&backend, &fixture, &request(None, false)).unwrap();
    assert_eq!(ids(&plan), ["mem_z", "mem_a"]);
    assert_eq!(plan.participants[0].before_commit, "before-z");
    assert_eq!(plan.participants[0].source_commit, "source-z");
    let reversed = crate::Selection {
        targets: vec!["mem_a".into(), "mem_z".into()],
        ..Default::default()
    };
    assert_eq!(
        ids(&build(&backend, &fixture, &request(Some(reversed), false)).unwrap()),
        ["mem_z", "mem_a"]
    );
}

#[test]
fn explicit_root_is_appended_after_frozen_member_order() {
    let root = crate::Selection {
        targets: vec!["@root".into()],
        ..Default::default()
    };
    let plan = build(&Default::default(), &fixture(), &request(Some(root), false)).unwrap();
    assert_eq!(ids(&plan), ["@root"]);
    assert_eq!(plan.participants[0].target_kind, MergeTargetKind::Root);
    assert_eq!(plan.participants[0].path, ".");
    assert_eq!(plan.participants[0].before_commit, "before-root");
    assert_eq!(plan.participants[0].source_commit, "source-root");

    let mixed = crate::Selection {
        targets: vec!["mem_a".into(), "@root".into(), "mem_z".into()],
        ..Default::default()
    };
    let plan = build(
        &Default::default(),
        &fixture(),
        &request(Some(mixed), false),
    )
    .unwrap();
    assert_eq!(ids(&plan), ["mem_z", "mem_a", "@root"]);
    assert_eq!(
        plan.participants
            .iter()
            .map(|participant| participant.target_kind)
            .collect::<Vec<_>>(),
        [
            MergeTargetKind::Member,
            MergeTargetKind::Member,
            MergeTargetKind::Root,
        ]
    );

    for selection in [
        crate::Selection {
            all: Some(true),
            ..Default::default()
        },
        crate::Selection {
            targets: vec!["@all".into()],
            ..Default::default()
        },
        crate::Selection {
            all: Some(true),
            targets: vec!["@root".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        },
    ] {
        assert_eq!(
            ids(&build(
                &Default::default(),
                &fixture(),
                &request(Some(selection), false)
            )
            .unwrap()),
            ["mem_z", "mem_a"]
        );
    }
}

#[test]
fn explicit_root_distinguishes_persisted_and_git_normalized_metadata_bytes() {
    let fixture = fixture();
    let selection = crate::Selection {
        targets: vec!["@root".into()],
        ..Default::default()
    };
    let plan = build_merge_plan(
        &FakeBackend::default(),
        fixture.0.path(),
        &request(Some(selection), false),
        &fixture.1,
        &fixture.2,
        MergeBaseline {
            lock_sha256: format!("{:x}", Sha256::digest(b"baseline-lock\r\n")),
            manifest_sha256: format!("{:x}", Sha256::digest(b"baseline-manifest\r\n")),
            lock_yaml: Some("baseline-lock\r\n".to_owned()),
            manifest_yaml: Some("baseline-manifest\r\n".to_owned()),
            lock_commit_sha256: Some(format!("{:x}", Sha256::digest(b"baseline-lock"))),
            manifest_commit_sha256: Some(format!("{:x}", Sha256::digest(b"baseline-manifest"))),
            root_head: Some("before-root".into()),
            root_branch: Some("main".into()),
            extensions: Default::default(),
        },
    )
    .unwrap();

    assert_eq!(ids(&plan), ["@root"]);
    assert_ne!(
        plan.baseline.lock_sha256,
        plan.baseline.lock_commit_sha256.unwrap()
    );
}

#[test]
fn dry_run_is_advisory_and_full_preflight_precedes_any_execution() {
    let fixture = fixture();
    let mut backend = FakeBackend {
        simulation_conflict: Some("a"),
        ..Default::default()
    };
    let normal = build(&backend, &fixture, &request(None, false)).unwrap();
    assert!(
        normal
            .participants
            .iter()
            .all(|participant| !participant.prediction_complete)
    );
    backend.calls.borrow_mut().clear();
    let predicted = build(&backend, &fixture, &request(None, true)).unwrap();
    assert_eq!(*backend.simulations.borrow(), ["z", "a"]);
    assert!(
        predicted
            .participants
            .iter()
            .all(|participant| participant.prediction_complete)
    );
    assert!(
        predicted.participants[0]
            .predicted_conflict_paths
            .is_empty()
    );
    assert_eq!(
        predicted.participants[1].predicted_conflict_paths,
        ["a.txt"]
    );
    backend.calls.borrow_mut().clear();
    backend.dirty = Some("a");
    assert_eq!(
        build(&backend, &fixture, &request(None, false))
            .unwrap_err()
            .code,
        ErrorCode::DirtyMember
    );
    assert_eq!(*backend.calls.borrow(), ["z"]);
}

#[test]
fn ff_only_rejects_the_complete_preflight_before_execution() {
    let fixture = fixture();
    let backend = FakeBackend::default();
    let mut value = request(None, false);
    value.mode = Some(crate::MergeMode::FfOnly);

    let error = build(&backend, &fixture, &value).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_z"));
    assert_eq!(error.member_path.as_deref(), Some("z"));
    assert_eq!(*backend.calls.borrow(), ["z", "a"]);
    assert!(backend.simulations.borrow().is_empty());
}

#[test]
fn preflight_propagates_integration_errors_and_rejects_target_drift() {
    let integration = FakeBackend {
        dirty: Some("integration"),
        ..Default::default()
    };
    assert_eq!(
        build(&integration, &fixture(), &request(None, false))
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let backend = FakeBackend {
        drift: Some("a"),
        ..Default::default()
    };
    assert_eq!(
        build(&backend, &fixture(), &request(None, false))
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
}

#[test]
fn second_member_missing_source_has_member_context_and_preserves_backend_code() {
    let backend = FakeBackend {
        failure: Some((FailurePoint::MergeAnalysis, "a")),
        ..Default::default()
    };

    let error = build(&backend, &fixture(), &request(None, false)).unwrap_err();

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(error.member_path.as_deref(), Some("a"));
    assert!(error.message.starts_with("member 'mem_a' at 'a':"));
    let wire = crate::GwzError::from(&error);
    assert_eq!(wire.member_id.as_deref(), Some("mem_a"));
    assert_eq!(wire.member_path.as_deref(), Some("a"));
    assert_eq!(wire.target_kind, Some(crate::TargetKind::Member));
}

#[test]
fn second_member_foreign_integration_state_has_member_context() {
    let backend = FakeBackend {
        failure: Some((FailurePoint::MergeState, "a")),
        ..Default::default()
    };

    let error = build(&backend, &fixture(), &request(None, false)).unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(error.member_path.as_deref(), Some("a"));
    assert!(error.message.contains("RebaseMerge"));
    assert!(error.message.starts_with("member 'mem_a' at 'a':"));
}

#[test]
fn every_fallible_backend_preflight_probe_adds_member_context() {
    for point in [
        FailurePoint::IsRepository,
        FailurePoint::Status,
        FailurePoint::Head,
        FailurePoint::MergeState,
        FailurePoint::MergeAnalysis,
        FailurePoint::ReadRef,
    ] {
        let backend = FakeBackend {
            failure: Some((point, "a")),
            ..Default::default()
        };

        let error = build(&backend, &fixture(), &request(None, false)).unwrap_err();

        assert_eq!(error.member_id.as_deref(), Some("mem_a"), "{point:?}");
        assert_eq!(error.member_path.as_deref(), Some("a"), "{point:?}");
        assert!(
            error.message.starts_with("member 'mem_a' at 'a':"),
            "{point:?}: {error}"
        );
    }
}
