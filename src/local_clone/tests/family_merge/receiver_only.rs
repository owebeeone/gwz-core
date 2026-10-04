//! A member only the receiving workspace records (design §6; operator
//! ruling 2026-10-04, "this is a bug, it should allow merging"; root
//! `dev-docs/GwzLaneIssues.md` L11). gwz-dev registered gwz-sspi after the
//! lane `tr1-8-win` was cloned, and `gwz merge --remote tr1-8-win` refused
//! with `pairing_mismatch` -- even with `--target` leaving gwz-sspi out.
//!
//! Such a member has nothing to import. A set selection (the default,
//! `@all`) leaves it out of the import and the merge and reports it "not in
//! source lane; unchanged"; a selection that names it refuses before any
//! fetch. Its HEAD, worktree, lock row and refs stay exactly as they were,
//! through start, `--status`, `--abort`, `--continue` and `--gc`. The other
//! direction -- a member only the lane has -- still refuses
//! (`a_pairing_set_mismatch_refuses_before_any_fetch`).

use super::*;

/// The report a receiver-only member gets in the family merge's summary.
const LEFT_ALONE: &str = "extra (mem_extra): not in source lane; unchanged";

/// `family(label, ["app"])` whose root then registers `extra` as
/// `mem_extra` and commits its configuration, as gwz-dev did with gwz-sspi:
/// the receiving lock records `mem_extra`, the lane `A`'s does not. The
/// member also holds an untracked note, so "unchanged" covers its worktree.
fn family_with_receiver_only_member(label: &str) -> (Family, PathBuf) {
    let family = family(label, &["app"]);
    let extra = family.member("extra");
    gwz_local_testrepo::TestRepo::init(&extra, &gwz_local_testrepo::RepoSpec::new())
        .commit_files("extra", &[("README", b"extra\n")]);
    handle_add_existing_repo(
        &family.backend,
        &family.root,
        crate::AddExistingRepoRequest {
            meta: meta("req-add-extra"),
            repository_path: extra.to_string_lossy().into_owned(),
            member_path: Some("extra".to_owned()),
            member_id: Some("mem_extra".to_owned()),
            source_id: Some("src_extra".to_owned()),
        },
        "op_add_extra",
    )
    .expect("the root registers a member the lane never had");
    commit_workspace_configuration(&family.root);
    fs::write(extra.join("notes.txt"), "local notes\n").unwrap();
    let receiver = crate::artifact::read_lock(&family.root).unwrap();
    let lane = crate::artifact::read_lock(&family.clone).unwrap();
    assert!(receiver.members.contains_key("mem_extra"));
    assert!(!lane.members.contains_key("mem_extra"));
    (family, extra)
}

/// Everything a family merge must leave alone in the receiver-only member.
#[derive(Debug, PartialEq)]
struct Untouched {
    head: String,
    branch: Option<String>,
    readme: Vec<u8>,
    note: Vec<u8>,
    import_refs: Vec<String>,
    lock_row: crate::artifact::ResolvedMemberArtifact,
}

fn untouched(family: &Family, extra: &Path) -> Untouched {
    let head = family.backend.head(extra).unwrap();
    Untouched {
        head: head.commit.unwrap(),
        branch: head.branch,
        readme: fs::read(extra.join("README")).unwrap(),
        note: fs::read(extra.join("notes.txt")).unwrap(),
        import_refs: import_refs(extra),
        lock_row: crate::artifact::read_lock(&family.root).unwrap().members["mem_extra"].clone(),
    }
}

fn message(response: &crate::MergeResponse) -> &str {
    response.response.meta.message.as_deref().unwrap_or("")
}

fn targets(response: &crate::MergeResponse) -> Vec<&str> {
    let mut targets: Vec<&str> = response
        .repos
        .iter()
        .map(|repo| repo.target_id.as_str())
        .collect();
    targets.sort_unstable();
    targets
}

/// Work in the lane's member and in its root, then a family merge under
/// `selection` that reaches the receiver-only member only through a set
/// selector, or not at all: it completes for the paired repositories, the
/// lane's commits arrive, and the receiver-only member is reported and left
/// exactly as it was.
fn merges_the_paired_members_and_leaves_the_extra_member(
    label: &str,
    selection: Option<crate::Selection>,
) {
    let (family, extra) = family_with_receiver_only_member(label);
    let backend = &family.backend;
    let in_clone = commit(
        &family.clone_member("app"),
        "feature.txt",
        "from A\n",
        "work in A",
        "HEAD",
    );
    let root_work = commit(
        &family.clone,
        "root-work.txt",
        "root work\n",
        "root work in A",
        "HEAD",
    );
    let before = untouched(&family, &extra);

    let mut request = family_merge_request("A", None);
    request.meta.selection = selection;
    let response = handle_merge_with_local_family(
        backend,
        &family.root,
        request,
        "op_receiver_only",
        &NullSink,
    )
    .expect("the paired members merge");

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(targets(&response), ["@root", "mem_app"]);
    assert!(
        message(&response).contains(LEFT_ALONE),
        "{}",
        message(&response)
    );

    // The lane's commits arrived, in the member and in the root.
    assert_eq!(head(backend, &family.member("app")), in_clone);
    assert_eq!(repo(&response, "@root").source_commit, root_work);
    let root = git2::Repository::open(&family.root).unwrap();
    let root_head = git2::Oid::from_str(&head(backend, &family.root)).unwrap();
    assert!(
        root.graph_descendant_of(root_head, git2::Oid::from_str(&root_work).unwrap())
            .unwrap()
    );
    let lock = crate::artifact::read_lock(&family.root).unwrap();
    assert_eq!(
        lock.members["mem_app"].commit.as_deref(),
        Some(in_clone.as_str())
    );

    // The receiver-only member: no import ref, HEAD, worktree and lock row
    // exactly as they were.
    assert_eq!(untouched(&family, &extra), before);
    assert!(family_lock_is_free(&family.root));
}

/// (a) The default selection -- the root and every active member -- leaves
/// the receiver-only member out of the import and the merge.
#[test]
fn a_default_family_merge_leaves_a_member_the_lane_does_not_have_unchanged() {
    merges_the_paired_members_and_leaves_the_extra_member(
        "family-merge-receiver-only-default",
        None,
    );
}

/// (b) An explicit target set that leaves the member out, as in the
/// operator's `gwz --target @root --target gwz-core ... merge --remote
/// tr1-8-win`.
#[test]
fn an_explicit_target_set_without_the_member_merges_the_paired_members() {
    merges_the_paired_members_and_leaves_the_extra_member(
        "family-merge-receiver-only-explicit",
        Some(crate::Selection {
            targets: vec!["@root".into(), "mem_app".into()],
            ..Default::default()
        }),
    );
}

/// (c) A selection that names the receiver-only member -- by id, by path,
/// or beside paired members -- asks for an import from a member the lane
/// does not have: refused before any fetch, naming the member, with nothing
/// written anywhere.
#[test]
fn naming_a_member_the_lane_does_not_have_refuses_before_any_fetch() {
    let (family, extra) = family_with_receiver_only_member("family-merge-receiver-only-named");
    let backend = &family.backend;
    commit(
        &family.clone_member("app"),
        "feature.txt",
        "from A\n",
        "work in A",
        "HEAD",
    );
    let before = untouched(&family, &extra);
    let app_before = head(backend, &family.member("app"));
    let root_before = head(backend, &family.root);

    for named in [
        vec!["mem_extra"],
        vec!["extra"],
        vec!["@root", "mem_app", "extra"],
    ] {
        let mut request = family_merge_request("A", None);
        request.meta.selection = Some(crate::Selection {
            targets: named.iter().map(|token| (*token).to_owned()).collect(),
            ..Default::default()
        });
        let error = handle_merge_with_local_family(
            backend,
            &family.root,
            request,
            "op_receiver_only_named",
            &NullSink,
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::PairingMismatch,
            "{named:?}: {error:?}"
        );
        for expected in [
            "extra (mem_extra)",
            "the source lane `A` has no such member",
            "refused before any fetch; nothing was written",
        ] {
            assert!(
                error.message.contains(expected),
                "{named:?}: {}",
                error.message
            );
        }
    }

    for path in [
        family.member("app"),
        extra.clone(),
        family.root.clone(),
        family.clone_member("app"),
        family.clone.clone(),
    ] {
        assert!(import_refs(&path).is_empty(), "{}", path.display());
    }
    assert!(!family.root.join(".gwz/merge").exists());
    assert!(family_lock_is_free(&family.root));
    assert_eq!(head(backend, &family.member("app")), app_before);
    assert_eq!(head(backend, &family.root), root_before);
    assert_eq!(untouched(&family, &extra), before);
}

/// (d) Recovery beside a receiver-only member: a conflicting family merge
/// (the `README` conflict fixture, members selected through `@all`) stops
/// with the member left out; `--status` and `--abort` complete; a restart
/// stops again, and `--continue` completes it; `--gc` drops the archived
/// record. The member is never a participant and is never touched.
#[test]
fn a_conflicted_family_merge_beside_a_receiver_only_member_aborts_and_continues() {
    let (family, extra) = family_with_receiver_only_member("family-merge-receiver-only-recovery");
    let backend = &family.backend;
    let local = commit(
        &family.member("app"),
        "README",
        "local\n",
        "local edit",
        "HEAD",
    );
    let imported = commit(
        &family.clone_member("app"),
        "README",
        "clone\n",
        "clone edit",
        "HEAD",
    );
    let before = untouched(&family, &extra);
    let lock_before = fs::read(family.root.join(crate::artifact::LOCK_PATH)).unwrap();
    let start = |operation: &str| {
        let started = handle_merge_with_local_family(
            backend,
            &family.root,
            family_merge_request("A", None),
            operation,
            &NullSink,
        )
        .expect("a conflicted start is a response, not an error");
        assert_eq!(
            started.state,
            crate::MergeOperationState::AwaitingResolution
        );
        assert_eq!(targets(&started), ["mem_app"]);
        assert_eq!(repo(&started, "mem_app").source_commit, imported);
        assert_eq!(repo(&started, "mem_app").conflict_paths, ["README"]);
        assert!(
            message(&started).contains(LEFT_ALONE),
            "{}",
            message(&started)
        );
        assert_eq!(untouched(&family, &extra), before);
        started.merge_id.expect("an open merge id")
    };
    let lifecycle = |op: crate::MergeOp, merge_id: Option<String>, operation: &str| {
        handle_merge_with_local_family(
            backend,
            &family.root,
            lifecycle_request(op, merge_id),
            operation,
            &NullSink,
        )
    };

    let first = start("op_receiver_only_conflict");
    let status = lifecycle(crate::MergeOp::Status, None, "op_receiver_only_status").unwrap();
    assert_eq!(status.merge_id.as_deref(), Some(first.as_str()));
    assert_eq!(targets(&status), ["mem_app"]);
    let aborted = lifecycle(crate::MergeOp::Abort, Some(first), "op_receiver_only_abort")
        .expect("abort restores");
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(head(backend, &family.member("app")), local);
    assert_eq!(
        fs::read(family.root.join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(untouched(&family, &extra), before);

    let second = start("op_receiver_only_conflict_again");
    fs::write(family.member("app").join("README"), "resolved\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&family.member("app"), &["README"])
        .unwrap();
    let continued = lifecycle(
        crate::MergeOp::Resume,
        Some(second.clone()),
        "op_receiver_only_continue",
    )
    .expect("continue completes");
    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert_eq!(targets(&continued), ["mem_app"]);
    let result = repo(&continued, "mem_app")
        .resulting_commit
        .clone()
        .unwrap();
    assert_eq!(head(backend, &family.member("app")), result);
    let lock = crate::artifact::read_lock(&family.root).unwrap();
    assert_eq!(
        lock.members["mem_app"].commit.as_deref(),
        Some(result.as_str())
    );
    assert_eq!(untouched(&family, &extra), before);
    // Both imports are retained in the paired member; none was made in the
    // receiver-only one (`untouched` holds its empty list).
    assert_eq!(import_refs(&family.member("app")).len(), 2);

    lifecycle(crate::MergeOp::Gc, Some(second), "op_receiver_only_gc")
        .expect("gc drops the archived record");
    assert_eq!(untouched(&family, &extra), before);
}
