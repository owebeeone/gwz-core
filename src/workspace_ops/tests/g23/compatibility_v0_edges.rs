use super::*;

macro_rules! decode {
    ($record:expr) => {
        crate::workspace_ops::merge::decode_v0_for_r3_tests(
            serde_yaml::to_string($record).unwrap().as_bytes(),
        )
        .unwrap()
    };
}

pub(super) fn assert_legacy_v0_compatibility_edges<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) {
    assert_exact_baseline_recovery(backend, root, record);
    let expected_manifest = record.baseline.manifest_yaml.clone().unwrap();
    let mut missing_baseline = record.clone();
    missing_baseline.baseline.lock_yaml = None;
    missing_baseline.baseline.manifest_yaml = None;
    let decoded = decode!(&missing_baseline);
    let manifest_path = root.join(crate::workspace::WORKSPACE_MANIFEST);
    fs::write(&manifest_path, "unavailable baseline\n").unwrap();
    assert_eq!(
        crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap(),
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted
    );
    fs::write(&manifest_path, expected_manifest).unwrap();

    missing_baseline.mode = crate::workspace_ops::merge::MergeExecutionMode::NoFf;
    assert_unsupported_no_ff(backend, root, &missing_baseline);

    let mut legacy_pending = record.clone();
    legacy_pending.state = OperationState::Executing;
    legacy_pending.publication = None;
    let participant = legacy_pending.participants.values_mut().next().unwrap();
    participant.state = crate::workspace_ops::merge::ParticipantState::Planned;
    participant.resulting_commit = None;
    participant.pending_action = Some(crate::workspace_ops::merge::PendingMergeAction {
        kind: crate::workspace_ops::merge::PendingMergeActionKind::VerifyUpToDate,
        target_branch: participant.target_branch.clone(),
        before_commit: participant.before_commit.clone(),
        source_commit: participant.source_commit.clone(),
        commit_message: participant.commit_message.clone(),
        expected_result: None,
        commit_spec: None,
        extensions: BTreeMap::new(),
    });
    let decoded = decode!(&legacy_pending);
    assert_eq!(
        crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap(),
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted
    );
    legacy_pending.mode = crate::workspace_ops::merge::MergeExecutionMode::NoFf;
    assert_unsupported_no_ff(backend, root, &legacy_pending);

    assert_finalizing_non_domain_rows_stay_v0(backend, root, record);
}

fn assert_finalizing_non_domain_rows_stay_v0<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) {
    let mut ff_only = record.clone();
    ff_only.mode = crate::workspace_ops::merge::MergeExecutionMode::FfOnly;
    assert_valid_unlisted(backend, root, &ff_only);

    for state in [
        crate::workspace_ops::merge::ParticipantState::Merged,
        crate::workspace_ops::merge::ParticipantState::Continued,
    ] {
        let mut unlisted = record.clone();
        unlisted.participants.values_mut().next().unwrap().state = state;
        assert_valid_unlisted(backend, root, &unlisted);
    }

    let mut multi_member = record.clone();
    let mut manifest = crate::artifact::ManifestArtifact::from_yaml(
        multi_member.baseline.manifest_yaml.as_deref().unwrap(),
    )
    .unwrap();
    let mut extra = manifest.members[0].clone();
    extra.id = "mem_extra".to_owned();
    extra.path = "extra".to_owned();
    extra.source_id = "src_extra".to_owned();
    manifest.members.push(extra);
    let manifest_yaml = manifest.to_yaml().unwrap();
    let mut lock = crate::artifact::LockArtifact::from_yaml(
        multi_member.baseline.lock_yaml.as_deref().unwrap(),
    )
    .unwrap();
    let mut extra = lock.members.values().next().unwrap().clone();
    extra.path = "extra".to_owned();
    extra.source_id = Some("src_extra".to_owned());
    lock.members.insert("mem_extra".to_owned(), extra);
    let lock_yaml = lock.to_yaml().unwrap();
    multi_member.baseline.manifest_sha256 = digest(&manifest_yaml);
    multi_member.baseline.lock_sha256 = digest(&lock_yaml);
    multi_member.baseline.manifest_yaml = Some(manifest_yaml);
    multi_member.baseline.lock_yaml = Some(lock_yaml);
    assert_valid_unlisted(backend, root, &multi_member);
}

fn assert_valid_unlisted<B: GitBackend>(backend: &B, root: &Path, record: &MergeOperationRecord) {
    assert_eq!(
        crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decode!(record),
            "r3-test-writer",
        )
        .unwrap(),
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted
    );
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn assert_exact_baseline_recovery<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) {
    let expected_lock = record.baseline.lock_yaml.as_deref().unwrap();
    let expected_manifest = record.baseline.manifest_yaml.as_deref().unwrap();
    let mut missing = record.clone();
    missing.baseline.lock_yaml = None;
    missing.baseline.manifest_yaml = None;
    let crate::workspace_ops::merge::OpenV0Adaptation::Eligible {
        record: adapted, ..
    } = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decode!(&missing),
        "r3-test-writer",
    )
    .unwrap()
    else {
        panic!("exact baseline source did not produce an eligible adaptation")
    };
    assert_eq!(adapted.baseline.lock_yaml.as_deref(), Some(expected_lock));
    assert_eq!(
        adapted.baseline.manifest_yaml.as_deref(),
        Some(expected_manifest)
    );
}

fn assert_unsupported_no_ff<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) {
    assert_eq!(
        crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decode!(record),
            "r3-test-writer",
        )
        .unwrap_err()
        .code,
        ErrorCode::UnsupportedLegacyMode
    );
}

pub(super) fn set_symbolic_head(root: &Path, branch: &str) {
    let output = std::process::Command::new("git")
        .args(["-C", root.to_str().unwrap(), "symbolic-ref", "HEAD"])
        .arg(format!("refs/heads/{branch}"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
