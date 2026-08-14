use super::*;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundObservationRequest, ObservationKind, ResolvedV1Action, V1LifecycleRequest,
    resolve_observation,
};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::run_test as run;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use crate::{model::ErrorCode, workspace_ops::merge::MergeRecordError};
use sha2::{Digest, Sha256};

#[test]
fn halted_entry_issues_one_bound_preservation_transition_after_global_preflight() {
    let mut fixture = dirty_integrated_fixture("v1-preservation-entry");
    fixture.model.state = OperationState::Halted;
    fixture.model.preservation_publication_handoff = None;
    add_failed_member(&mut fixture, true);
    let current = fixture.current();
    let request = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Preserve,
        ObservationKind::PreservationEntry,
    )
    .unwrap();
    let observation =
        super::super::observe(&fixture.backend, &fixture.context(), &current, &request).unwrap();

    assert!(matches!(
        resolve_observation(
            &current,
            V1LifecycleRequest::Preserve,
            request,
            observation,
            None,
        )
        .unwrap(),
        ResolvedV1Action::Apply(_)
    ));
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn later_member_preflight_failure_changes_no_record_or_earlier_repository() {
    let mut fixture = dirty_integrated_fixture("v1-preservation-entry-global");
    fixture.model.state = OperationState::Halted;
    fixture.model.preservation_publication_handoff = None;
    add_failed_member(&mut fixture, false);
    fixture.seed_open();

    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let head_before = fixture.backend.head(&fixture.member).unwrap();
    let image_before = fixture
        .backend
        .preservation_image(&fixture.member, true)
        .unwrap();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("invalid later member unexpectedly passed preflight"),
        Err(error) => error,
    };

    assert_eq!(error.member_id.as_deref(), Some("mem_b"), "{error:?}");
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), head_before);
    assert_eq!(
        fixture
            .backend
            .preservation_image(&fixture.member, true)
            .unwrap(),
        image_before
    );
    assert!(
        fixture
            .backend
            .read_ref(
                &fixture.member,
                &format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn bundle_collision_is_rejected_before_any_preservation_mutation() {
    let mut fixture = dirty_integrated_fixture("v1-preservation-entry-bundle");
    fixture.model.state = OperationState::Halted;
    fixture.model.preservation_publication_handoff = None;
    add_failed_member(&mut fixture, true);
    fixture.seed_open();
    let bundle = crate::stash::bundle_path(
        &fixture.root.path,
        &format!("stash_{}", fixture.model.merge_id),
    );
    fs::create_dir_all(bundle.parent().unwrap()).unwrap();
    fs::write(&bundle, "foreign bundle\n").unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let head_before = fixture.backend.head(&fixture.member).unwrap();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("foreign preservation bundle unexpectedly passed preflight"),
        Err(error) => error,
    };

    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), head_before);
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .backend
            .read_ref(
                &fixture.member,
                &format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id),
            )
            .unwrap()
            .is_none()
    );
}

fn add_failed_member(fixture: &mut PreservationFixture, materialize: bool) {
    let path = fixture.root.path.join("members/b");
    let before = if materialize {
        fixture.backend.create_repo(&path).unwrap();
        commit_file(&path, "README.md", "before b\n", "before b", &[]).unwrap()
    } else {
        fixture.before.clone()
    };
    let mut row = fixture.model.participants["mem_a"].clone();
    row.path = "members/b".into();
    row.before_commit = before.clone();
    row.source_commit = before;
    row.state = ParticipantState::Failed;
    row.resulting_commit = None;
    row.error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "injected halt cause".into(),
        detail: None,
    });
    row.pending_action = None;
    row.preservation.clear();
    let mut manifest = crate::artifact::ManifestArtifact::from_yaml(
        fixture.model.baseline.manifest_yaml.as_deref().unwrap(),
    )
    .unwrap();
    let mut manifest_member = manifest.members[0].clone();
    manifest_member.id = "mem_b".into();
    manifest_member.path = "members/b".into();
    manifest_member.source_id = "src_b".into();
    manifest.members.push(manifest_member);
    let manifest = manifest.to_yaml().unwrap();
    fixture.model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    fixture.model.baseline.manifest_yaml = Some(manifest);
    fixture.model.selected_targets.push("mem_b".into());
    fixture.model.participants.insert("mem_b".into(), row);
}
