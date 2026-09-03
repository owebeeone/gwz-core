//! The A1 activation's own suite — the enabled paths, executed.
//!
//! Safety review `GwzM5-8A1Activation-ReviewSafety.md` §2 is this package's
//! binding spec. These tests execute what the activation turned on: the
//! contract-§2 writer floor producing a v1 record, `--no-ff` running to a
//! two-parent integration through production dispatch, and the R1/R2 coupled
//! pair moving as one gate.
//!
//! **M5d.** The activation's OTHER half — the adaptation precheck, the atomic
//! upgrade, and the "an eligible row completes under v0" cases — tested the
//! open-v0 migration path. `GwzM5-8M5d-Charter.md` §2 deletes that path
//! outright ("No whitelist. No `open_v0`. No 'valid unlisted stays on the v0
//! lifecycle.'"), so those cases left with it; `g23/pre_014_refusal.rs` is
//! what an open v0 record answers now.

use super::*;

use crate::workspace_ops::merge::{RecordVersion, RequestedSemantics, select_record_version};

/// A `--no-ff` start request, otherwise identical to the ordinary one.
fn no_ff_request() -> crate::MergeRequest {
    crate::MergeRequest {
        mode: Some(crate::MergeMode::NoFf),
        ..request(false)
    }
}

/// The exact `(schema, record_schema_version)` pair on disk for `merge_id`,
/// wherever the record currently lives.
fn envelope_on_disk(root: &Path, merge_id: &str) -> (String, u64) {
    let open = root.join(format!(".gwz/merge/{merge_id}.yaml"));
    let done = root.join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let path = if open.is_file() { open } else { done };
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("record at '{}' is readable: {error}", path.display()));
    let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    (
        value["schema"].as_str().unwrap().to_owned(),
        value["record_schema_version"].as_u64().unwrap(),
    )
}

/// **R4, executed.** The contract-§2 writer floor chooses the version at
/// creation and the created record carries that version's envelope. Pre-A1
/// `start/record.rs` hard-coded `gwz.merge-operation/v0` / `0` for every
/// start; the activated no-ff surface writes `gwz.merge-operation/v1` / `1`.
///
/// **EXTENDED at DR-1 ship (1) W3** (`GwzM5-8DR1-WarnOrRefuse-Charter.md`
/// §3.1/§3.4, 2026-09-03) rather than duplicated: this is the ABOVE-bar start,
/// on the CI host's own ext4/APFS volume and outside the §3.8 seam, and after
/// W3 it must still activate the catalog and must now answer
/// `crash_recovery = supported`. The `catalog-final` assertion is also the
/// ANTI-VACUITY anchor for `crash_recovery.rs`'s "no catalog anywhere" rows: it
/// proves the same no-ff start on the same host DOES create that directory when
/// the decision is `Supported`, so their absence there is a measured difference
/// and not a walk that never finds anything.
/// **R4, executed.** The contract-§2 writer floor chooses the version at
/// creation and the created record carries that version's envelope. Pre-A1
/// `start/record.rs` hard-coded `gwz.merge-operation/v0` / `0` for every
/// start; the activated no-ff surface writes `gwz.merge-operation/v1` / `1`.
///
/// **EXTENDED at DR-1 ship (1) W3** (`GwzM5-8DR1-WarnOrRefuse-Charter.md`
/// §3.1/§3.4, 2026-09-03) rather than duplicated: this is the ABOVE-bar start,
/// on the CI host's own ext4/APFS volume and outside the §3.8 seam, and after
/// W3 it must still activate the catalog and must now answer
/// `crash_recovery = supported`. The `catalog-final` assertion is also the
/// ANTI-VACUITY anchor for `crash_recovery.rs`'s "no catalog anywhere" rows: it
/// proves the same no-ff start on the same host DOES create that directory when
/// the decision is `Supported`, so their absence there is a measured difference
/// and not a walk that never finds anything.
#[test]
fn the_production_writer_floor_writes_a_v1_record_for_no_ff() {
    let temp = TempDir::new("a1-writer-floor-v1");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-writer-floor-v1-source");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );

    let response = handle_merge(&backend, temp.path(), no_ff_request(), "op_a1_floor").unwrap();
    let merge_id = response.merge_id.as_deref().unwrap();

    assert_eq!(
        envelope_on_disk(temp.path(), merge_id),
        ("gwz.merge-operation/v1".to_owned(), 1),
        "the writer floor's record carries the v1 envelope"
    );
    assert_eq!(
        select_record_version(RequestedSemantics::NoFf).unwrap(),
        RecordVersion::V1
    );
    assert!(
        temp.path().join(".gwz/catalog-final").is_dir(),
        "an above-bar no-ff start still activates the catalog"
    );
    assert_eq!(
        response.crash_recovery,
        Some(crate::MergeCrashRecovery {
            supported: true,
            filesystem: None,
            gap: None,
        }),
        "an above-bar start reports crash recovery as supported, and names no gap"
    );
}

/// **R1 + the v1 writer, executed end to end.** A fast-forwardable member
/// merged with `--no-ff` gets a real two-parent integration commit whose
/// parents are exactly the member's prior HEAD and the source commit — the
/// behaviour the pre-A1 typed refusal made unreachable.
#[test]
fn no_ff_start_publishes_a_two_parent_integration_commit() {
    let temp = TempDir::new("a1-no-ff-two-parent");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-no-ff-two-parent-source");
    let member = temp.path().join("remote");
    let (before, source) = feature_commit(&backend, &member, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), no_ff_request(), "op_a1_no_ff").unwrap();

    let head = backend.head(&member).unwrap().commit.unwrap();
    assert_ne!(
        head, source,
        "no-ff must not fast-forward onto the source commit"
    );
    let repository = git2::Repository::open(&member).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(&head).unwrap())
        .unwrap();
    let parents: Vec<String> = commit.parent_ids().map(|id| id.to_string()).collect();
    assert_eq!(parents, vec![before, source], "{response:?}");
}

/// **The R1/R2 coupled pair, pinned.** `validate.rs`'s NoFf refusal and
/// `runtime/dispatch.rs`'s `mode != Some(NoFf)` message-validation exclusion
/// were two halves of one gate. Landing R1 alone would let a NoFf start carry
/// an unvalidated custom message into record creation, because the v1 forward
/// path consumes `row.commit_message` from the record and performs no
/// request-message validation of its own.
///
/// This is the inversion of M5b's designed marker
/// `custom_messages_validate_while_no_ff_remains_reserved`: NoFf is no longer
/// reserved, and every invalid custom-message body that an ordinary start
/// rejects a NoFf start must reject identically.
#[test]
fn the_coupled_pair_validates_custom_messages_on_no_ff_starts() {
    let temp = TempDir::new("a1-coupled-pair");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-coupled-pair-source");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );

    for body in ["", " \t\n", "\u{2003}\r\n", "subject\0body"] {
        let mut ordinary = request(false);
        ordinary.message = Some(body.to_owned());
        let mut no_ff = no_ff_request();
        no_ff.message = Some(body.to_owned());

        let ordinary_error =
            handle_merge(&backend, temp.path(), ordinary, "op_a1_message_v0").unwrap_err();
        let no_ff_error =
            handle_merge(&backend, temp.path(), no_ff, "op_a1_message_v1").unwrap_err();

        assert_eq!(ordinary_error.code, ErrorCode::MergeValidationFailed);
        assert_eq!(
            no_ff_error.code, ordinary_error.code,
            "the coupled pair rejects {body:?} on both modes"
        );
        assert_ne!(
            no_ff_error.code,
            ErrorCode::MergePhaseUnsupported,
            "T-1 inverted: no-ff is no longer a reserved phase"
        );
        assert!(
            !no_ff_error.message.contains("not yet activated"),
            "{}",
            no_ff_error.message
        );
    }
}

/// **A conflicted `--no-ff` merge, driven to completion — the defect this
/// package fixes, end to end.**
///
/// On 0.13.0 this workspace was a trap. The start left an open v1 record; every
/// blocked verb (`commit`, `capture`, `push`, `pull`, ...) answered
/// `UnsupportedRecordVersion: ... requires A1 ...` because discovery ran
/// through the v0 store's v0-only decoder and the version error propagated
/// before the open-merge gate could speak; `add` failed the same way, so
/// nothing could be staged; and `merge --continue` then refused with
/// `conflict resolution is not ready`. The only exit was `merge --abort`,
/// which threw the merge away. The identical fixture with an ORDINARY merge —
/// which writes v0 — worked, which is what made the version, not the merge,
/// the cause.
///
/// The three claims, in order: a blocked verb names the open merge and its
/// remedy, `add` routes to the conflicted participant and stages it, and
/// `--continue` then completes with a real two-parent integration commit.
#[test]
fn a_conflicted_no_ff_merge_blocks_mutators_stages_its_conflicts_and_continues() {
    let temp = TempDir::new("a1-no-ff-conflict-stage");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-no-ff-conflict-stage-src");
    let member = temp.path().join("remote");
    let (base, source) = feature_commit(&backend, &member, "README.md", "source\n");
    let local = commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();

    let started = handle_merge(&backend, temp.path(), no_ff_request(), "op_a1_conflict").unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution,
        "{started:?}"
    );
    assert_eq!(
        merge_repo(&started, "mem_remote").state,
        crate::MergeParticipantState::Conflicted
    );
    assert_eq!(
        envelope_on_disk(temp.path(), &merge_id),
        ("gwz.merge-operation/v1".to_owned(), 1),
        "the conflicted no-ff record is the v1 one the gates could not read"
    );

    // (1) A `Block` verb refuses with the open-merge remedy, not a version error.
    let blocked = handle_commit(
        &backend,
        temp.path(),
        crate::CommitRequest {
            meta: request_meta(),
            message: "blocked during merge".to_owned(),
            all: None,
            commit_marker: None,
        },
        "op_a1_blocked_commit",
    )
    .unwrap_err();
    assert_eq!(blocked.code, ErrorCode::OpenOperation, "{blocked:?}");
    for named in [
        merge_id.as_str(),
        "is open",
        "merge status",
        "merge continue",
        "merge abort",
    ] {
        assert!(blocked.message.contains(named), "{}", blocked.message);
    }

    // (2) `add` routes the pathspec to the conflicted participant and stages it.
    fs::write(member.join("README.md"), "resolved\n").unwrap();
    let staged = handle_stage(
        &backend,
        temp.path(),
        crate::StageRequest {
            meta: request_meta(),
            cwd: temp.path().to_string_lossy().into_owned(),
            pathspecs: vec!["remote/README.md".to_owned()],
            all: None,
        },
        "op_a1_stage",
    )
    .unwrap();
    assert_eq!(
        staged.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(
        backend
            .merge_state(&member)
            .unwrap()
            .is_none_or(|state| state.conflict_paths.is_empty()),
        "staging must have resolved the member's conflict"
    );

    // (3) `--continue` completes, and the integration commit really has two parents.
    let continued = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id)),
        "op_a1_continue",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert!(!continued.open);
    let head = backend.head(&member).unwrap().commit.unwrap();
    let repository = git2::Repository::open(&member).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(&head).unwrap())
        .unwrap();
    assert_eq!(
        commit
            .parent_ids()
            .map(|id| id.to_string())
            .collect::<Vec<_>>(),
        vec![local, source],
        "{continued:?}"
    );
}
