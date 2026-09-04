//! DR-1 ship (1) W3 — crash recovery as a capability, not a gate.
//!
//! `GwzM5-8DR1-WarnOrRefuse-Charter.md` §2/§3.1/§3.4/§3.6/§3.8 (2026-09-03) is
//! this file's binding spec. Every row here runs on the CI host's own volume,
//! which is ABOVE the bar (ext4 on Linux, APFS on macOS), so the below-bar
//! shape is presented through the charter's test-only seam (§3.8):
//! `with_identity_unavailable` makes `HostPlatform::dir_identity` answer the
//! same `Unsupported(PersistentFilesystemIdentity, …)` a btrfs or tmpfs volume
//! answers and `describe_volume` answer the injected description. Production
//! code is untouched by it — the seam is `cfg(test)` on both halves.

use super::*;

use crate::checked_artifact::entry::{CrashRecoveryDecision, crash_recovery_decision};
use crate::checked_artifact::{
    InjectedVolumeDescription, with_handle_probe_unavailable, with_identity_unavailable,
};

/// A `--no-ff` start request, otherwise the ordinary one.
fn no_ff_request() -> crate::MergeRequest {
    crate::MergeRequest {
        mode: Some(crate::MergeMode::NoFf),
        ..request(false)
    }
}

/// A below-bar volume that names itself and claims neither locality nor
/// volatility — the plain `no durable filesystem identity` shape (btrfs).
fn below_bar(name: Option<&str>) -> InjectedVolumeDescription {
    InjectedVolumeDescription {
        name: name.map(str::to_owned),
        remote: false,
        volatile: false,
    }
}

/// Every `Diagnostic`/`Warn` message one invocation emitted.
fn diagnostics(sink: &CollectingSink) -> Vec<String> {
    sink.take()
        .into_iter()
        .filter(|event| {
            event.kind == crate::EventKind::Diagnostic && event.severity == crate::Severity::Warn
        })
        .filter_map(|event| event.message)
        .collect()
}

/// The catalog's own final slot, wherever it could be under this workspace.
///
/// The production catalog is workspace-rooted (`catalog_lease/witness.rs`), so
/// `.gwz/catalog-final` is where it lands; the walk is the belt-and-braces half
/// — nothing named `catalog-final` may exist ANYWHERE under a workspace whose
/// merge decided the catalog is unavailable.
fn catalog_entries(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("catalog-"))
            {
                found.push(path.clone());
            }
            if path.is_dir() && !path.is_symlink() {
                walk(&path, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found
}

/// The open v1 records under `.gwz/merge`.
fn open_records(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root.join(".gwz/merge"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|value| value == "yaml"))
                .collect()
        })
        .unwrap_or_default()
}

/// A two-member workspace whose `app` member CONFLICTS on a `--no-ff` merge:
/// both sides changed `README.md` off the same base.
fn conflicting_workspace(label: &str) -> (TempDir, crate::git::Git2Backend) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let (_app, _lib) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let (app_base, _) = feature_commit(&backend, &app, "README.md", "source\n");
    commit_file(
        &app,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&app_base).unwrap()],
    )
    .unwrap();
    feature_commit(&backend, &temp.path().join("lib"), "README.md", "source\n");
    (temp, backend)
}

/// The three shared setup steps: a one-member workspace with a source commit.
fn workspace(label: &str) -> (TempDir, crate::git::Git2Backend, RemoteFixture) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let fixture = init_one_member_workspace(temp.path(), &backend, label);
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    (temp, backend, fixture)
}

/// Run `body` on a volume that proves NEITHER durable identity NOR persistent
/// file handles — the overlay-without-`nfs_export` shape.
///
/// **Two substrates, one row** (`GwzM5-8M5d-Charter.md` §3; the seam is ship
/// (1) §3.8's, extended at this step). The CI hosts are APFS and ext4, both of
/// which answer the handle probe, so by default the seam presents the refusal a
/// real overlay gives — byte-identical, because the injected error IS
/// `persistent_identity_unsupported()`. When the workflow has actually mounted
/// such a volume and put the fixture on it (`.github/workflows/
/// linux-identity-probe.yml`'s `handle-fail merge` job, which sets `TMPDIR` to
/// the overlay and `GWZ_M5D_REAL_HANDLE_FAIL=1`), the seam is NOT armed and the
/// same assertions run against the real kernel. Not a skip either way: the row
/// runs on every host, and the workflow's job fails if the real volume does not
/// reproduce it.
fn on_a_handle_fail_volume<T>(body: impl FnOnce() -> T) -> T {
    if std::env::var_os("GWZ_M5D_REAL_HANDLE_FAIL").is_some() {
        return body();
    }
    with_identity_unavailable(below_bar(Some("overlay")), || {
        with_handle_probe_unavailable(body)
    })
}

/// The ONE diagnostic a handle-fail invocation emits, asserted in the form both
/// substrates share: ship (1)'s sentence in front, the M5d clause appended.
///
/// The volume's NAME is deliberately not pinned here — the seam injects
/// `overlay`, and a real mount is named by `/proc/self/mountinfo`, which may
/// answer `unknown` on a kernel that does not resolve it. The exact text over
/// the whole gap space is pinned by the unit rows below, which are seam-only.
fn assert_one_handle_fail_diagnostic(sink: &CollectingSink) {
    let emitted = diagnostics(sink);
    assert_eq!(
        emitted.len(),
        1,
        "one diagnostic, not two: the raw write itself never warns -- {emitted:?}"
    );
    assert!(
        emitted[0].starts_with("crash recovery is unsupported on "),
        "{}",
        emitted[0]
    );
    assert!(
        emitted[0].contains(". Merge will continue. Use --filesystem-strict to refuse."),
        "ship (1)'s sentence must survive byte-identical in front: {}",
        emitted[0]
    );
    assert!(
        emitted[0].ends_with(
            "Selected-root and --preserve abort may refuse until the workspace is on a \
             handle-capable volume."
        ),
        "{}",
        emitted[0]
    );
}

/// Everything a handle-fail start must leave behind, asserted once.
fn assert_raw_record_completed(root: &Path, response: &crate::MergeResponse) {
    assert!(
        !response.crash_recovery.as_ref().unwrap().supported,
        "a handle-fail volume is below the identity bar too"
    );
    assert_eq!(
        response.crash_recovery.as_ref().unwrap().handles_ok,
        Some(false),
        "the machine truth a JSON consumer reads instead of stderr"
    );
    assert_eq!(catalog_entries(root), Vec::<PathBuf>::new());
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert!(!response.open, "{response:?}");
    let merge_id = response.merge_id.as_deref().unwrap();
    let archived = root.join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let value: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(&archived).unwrap()).unwrap();
    assert_eq!(
        value["schema"].as_str(),
        Some("gwz.merge-operation/v1"),
        "the handle-fail start still writes the v1 envelope"
    );
    let leftovers: Vec<PathBuf> = fs::read_dir(root.join(".gwz/merge"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|value| value == "tmp"))
        .collect();
    assert_eq!(
        leftovers,
        Vec::<PathBuf>::new(),
        "the raw writer publishes by rename and leaves no staged temporary behind"
    );
}

/// **The default, executed end to end** (charter §0's second bullet, §3.1's
/// start arm, §3.4's two channels).
///
/// A `--no-ff` start on a volume that cannot prove durable identity STARTS. It
/// prints exactly one warning carrying the operator's exact sentence, activates
/// no catalog at all, writes its v1 record through the checked boundary anyway,
/// and runs the merge to its archived terminal state. The response carries the
/// machine truth so a Json/Porcelain consumer never has to read stderr.
#[test]
fn a_below_bar_no_ff_start_warns_once_creates_no_catalog_and_completes() {
    let (temp, backend, _fixture) = workspace("w3-below-bar-start");
    let sink = CollectingSink::default();

    let response = with_identity_unavailable(below_bar(Some("btrfs")), || {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            no_ff_request(),
            "op_w3_below_bar",
            &sink,
        )
    })
    .unwrap();

    assert_eq!(
        diagnostics(&sink),
        vec![
            "crash recovery is unsupported on btrfs (no durable filesystem identity). Merge will \
             continue. Use --filesystem-strict to refuse."
                .to_owned()
        ],
        "exactly one diagnostic, carrying the operator's exact sentence"
    );
    assert_eq!(
        response.crash_recovery,
        Some(crate::MergeCrashRecovery {
            supported: false,
            filesystem: Some("btrfs".to_owned()),
            gap: Some(crate::MergeCrashRecoveryGap::NoDurableIdentity),
            // M5d charter §3: below the bar but the HANDLE probe still
            // answers, which is the btrfs / NFS / tmpfs shape. Checked
            // create, and no reverse-door clause on the diagnostic above.
            handles_ok: Some(true),
        })
    );
    assert_eq!(
        catalog_entries(temp.path()),
        Vec::<PathBuf>::new(),
        "a below-bar start must activate no catalog"
    );

    // The record was written — through the checked boundary, whose LEGACY
    // identity probe this volume still satisfies (charter §4.1) — and the
    // merge finished, so it is archived rather than open.
    let merge_id = response.merge_id.as_deref().unwrap();
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert!(!response.open, "{response:?}");
    assert_eq!(open_records(temp.path()), Vec::<PathBuf>::new());
    let archived = temp.path().join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let text = fs::read_to_string(&archived).unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    assert_eq!(
        value["schema"].as_str(),
        Some("gwz.merge-operation/v1"),
        "the below-bar start still writes the v1 envelope"
    );
}

/// **`--filesystem-strict`, executed** (charter §0's third bullet, §3.6).
///
/// The same start with the flag refuses BEFORE any lease, any record and any
/// Git work. The message carries the gap sentence and the remedy; the workspace
/// is byte-identical to the way it was before the call — no record, no catalog,
/// and every member's HEAD where it was.
#[test]
fn a_below_bar_no_ff_start_refuses_before_any_write_under_filesystem_strict() {
    let (temp, backend, _fixture) = workspace("w3-below-bar-strict");
    let member = temp.path().join("remote");
    let head_before = backend.head(&member).unwrap().commit.unwrap();
    let root_head_before = backend.head(temp.path()).unwrap().commit;
    let sink = CollectingSink::default();

    let mut request = no_ff_request();
    request.filesystem_strict = Some(true);
    let error = with_identity_unavailable(below_bar(Some("btrfs")), || {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            request,
            "op_w3_strict",
            &sink,
        )
    })
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(
        error
            .message
            .contains("crash recovery is unsupported on btrfs (no durable filesystem identity)"),
        "{}",
        error.message
    );
    for named in [
        "persistent file handles",
        "durable filesystem identity",
        "--filesystem-strict",
        "--abort",
    ] {
        assert!(error.message.contains(named), "{}", error.message);
    }
    assert!(
        diagnostics(&sink).is_empty(),
        "a refusal warns nobody: the flag asked for the refusal"
    );

    assert_eq!(open_records(temp.path()), Vec::<PathBuf>::new());
    assert!(!temp.path().join(".gwz/merge/done").exists());
    assert_eq!(catalog_entries(temp.path()), Vec::<PathBuf>::new());
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(head_before.as_str()),
        "no Git work may precede the strict refusal"
    );
    assert_eq!(backend.head(temp.path()).unwrap().commit, root_head_before);
}

/// **The gap, the parenthetical and the name, over the whole space**
/// (charter §0's parenthetical list, §0.1's tmpfs ruling, §3.3).
///
/// Volatile wins over remote, remote over the bare absence, and a volume the
/// platform cannot name is rendered `unknown` rather than refused. The tmpfs row
/// is the operator's ruling of 2026-09-03 executed: a volatile volume is a
/// CATALOG-admission refusal that the decision point maps onto the warning path,
/// never a merge refusal.
#[test]
fn every_gap_and_an_unnamed_volume_word_the_warning_and_the_response() {
    let cases = [
        (
            "volatile",
            InjectedVolumeDescription {
                name: Some("tmpfs".to_owned()),
                remote: false,
                volatile: true,
            },
            crate::MergeCrashRecoveryGap::VolatileFilesystem,
            "crash recovery is unsupported on tmpfs (volatile filesystem)",
        ),
        (
            "remote",
            InjectedVolumeDescription {
                name: Some("nfs".to_owned()),
                remote: true,
                volatile: false,
            },
            crate::MergeCrashRecoveryGap::RemoteFilesystem,
            "crash recovery is unsupported on nfs (remote filesystem)",
        ),
        (
            "volatile-wins-over-remote",
            InjectedVolumeDescription {
                name: Some("tmpfs".to_owned()),
                remote: true,
                volatile: true,
            },
            crate::MergeCrashRecoveryGap::VolatileFilesystem,
            "crash recovery is unsupported on tmpfs (volatile filesystem)",
        ),
        (
            "unnamed",
            below_bar(None),
            crate::MergeCrashRecoveryGap::NoDurableIdentity,
            "crash recovery is unsupported on unknown (no durable filesystem identity)",
        ),
    ];

    for (label, injected, gap, sentence) in cases {
        let (temp, backend, _fixture) = workspace(&format!("w3-gap-{label}"));
        let sink = CollectingSink::default();
        let expected_name = injected.name.clone();

        let response = with_identity_unavailable(injected, || {
            crate::workspace_ops::handle_merge_with_events(
                &backend,
                temp.path(),
                no_ff_request(),
                format!("op_w3_gap_{label}"),
                &sink,
            )
        })
        .unwrap();

        assert_eq!(
            diagnostics(&sink),
            vec![format!(
                "{sentence}. Merge will continue. Use --filesystem-strict to refuse."
            )],
            "{label}"
        );
        assert_eq!(
            response.crash_recovery,
            Some(crate::MergeCrashRecovery {
                supported: false,
                filesystem: expected_name,
                gap: Some(gap),
                handles_ok: Some(true),
            }),
            "{label}"
        );
        assert_eq!(
            catalog_entries(temp.path()),
            Vec::<PathBuf>::new(),
            "{label}"
        );
    }
}

/// **A conflicting below-bar attempt, continued and aborted** (charter §0's
/// "Ctrl-C / abort / resume of a live process work as today", §3.1's forward
/// service loop and its untouched reverse arms).
///
/// The start stops for resolution; a `--continue` is a NEW PROCESS, decides for
/// itself, warns ONCE for that invocation and proceeds on the plain lease with
/// no catalog; the response carries the decision. On a second below-bar
/// workspace the same open attempt is cleared by `--abort`, which decides
/// nothing, warns nothing and answers `crash_recovery = None` — the
/// capability-free path, untouched by this step.
#[test]
fn a_below_bar_continue_decides_once_and_abort_stays_capability_free() {
    let (temp, backend) = conflicting_workspace("w3-below-bar-continue");
    let app = temp.path().join("app");
    let started_sink = CollectingSink::default();
    let started = with_identity_unavailable(below_bar(Some("btrfs")), || {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            no_ff_request(),
            "op_w3_continue_start",
            &started_sink,
        )
    })
    .unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    assert!(started.open);
    assert_eq!(
        diagnostics(&started_sink).len(),
        1,
        "one diagnostic per process, start included"
    );
    assert_eq!(catalog_entries(temp.path()), Vec::<PathBuf>::new());

    fs::write(app.join("README.md"), "resolved\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&app, &["README.md"])
        .unwrap();

    let continued_sink = CollectingSink::default();
    let continued = with_identity_unavailable(below_bar(Some("btrfs")), || {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, started.merge_id.clone()),
            "op_w3_continue",
            &continued_sink,
        )
    })
    .unwrap();

    assert_eq!(
        diagnostics(&continued_sink),
        vec![
            "crash recovery is unsupported on btrfs (no durable filesystem identity). Merge will \
             continue. Use --filesystem-strict to refuse."
                .to_owned()
        ],
        "a continue is a new process: it decides once and warns once"
    );
    assert_eq!(
        continued.crash_recovery,
        Some(crate::MergeCrashRecovery {
            supported: false,
            filesystem: Some("btrfs".to_owned()),
            gap: Some(crate::MergeCrashRecoveryGap::NoDurableIdentity),
            handles_ok: Some(true),
        })
    );
    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert!(!continued.open, "{continued:?}");
    assert_eq!(
        catalog_entries(temp.path()),
        Vec::<PathBuf>::new(),
        "no forward arm of a below-bar attempt may activate a catalog"
    );

    // Abort of a below-bar attempt, on its own workspace: it is on the
    // capability-free list BY PATH and decides nothing at all.
    let (aborting, backend) = conflicting_workspace("w3-below-bar-abort");
    let open = with_identity_unavailable(below_bar(Some("btrfs")), || {
        handle_merge(
            &backend,
            aborting.path(),
            no_ff_request(),
            "op_w3_abort_open",
        )
    })
    .unwrap();
    assert!(open.open, "{open:?}");

    let abort_sink = CollectingSink::default();
    let aborted = with_identity_unavailable(below_bar(Some("btrfs")), || {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            aborting.path(),
            recovery_request(crate::MergeOp::Abort, open.merge_id.clone()),
            "op_w3_abort",
            &abort_sink,
        )
    })
    .unwrap();
    assert!(!aborted.open, "{aborted:?}");
    assert_eq!(
        aborted.crash_recovery, None,
        "abort decides nothing, so it reports nothing"
    );
    assert!(
        diagnostics(&abort_sink).is_empty(),
        "abort never asks about crash recovery"
    );
    assert_eq!(catalog_entries(aborting.path()), Vec::<PathBuf>::new());
    assert!(no_open_record(aborting.path()));
}

/// **The warning's wording, unit-tested over the whole gap space**
/// (charter §3.4's exact text).
///
/// One string, three parentheticals, `unknown` for a volume nobody could name.
/// The `warning: ` prefix is the DRIVER's and must not appear here.
#[test]
fn the_warning_renders_the_operators_exact_sentence() {
    let cases = [
        (
            Some("btrfs"),
            crate::MergeCrashRecoveryGap::NoDurableIdentity,
            "crash recovery is unsupported on btrfs (no durable filesystem identity). Merge will \
             continue. Use --filesystem-strict to refuse.",
        ),
        (
            Some("fuse.sshfs"),
            crate::MergeCrashRecoveryGap::RemoteFilesystem,
            "crash recovery is unsupported on fuse.sshfs (remote filesystem). Merge will \
             continue. Use --filesystem-strict to refuse.",
        ),
        (
            Some("tmpfs"),
            crate::MergeCrashRecoveryGap::VolatileFilesystem,
            "crash recovery is unsupported on tmpfs (volatile filesystem). Merge will continue. \
             Use --filesystem-strict to refuse.",
        ),
        (
            None,
            crate::MergeCrashRecoveryGap::NoDurableIdentity,
            "crash recovery is unsupported on unknown (no durable filesystem identity). Merge \
             will continue. Use --filesystem-strict to refuse.",
        ),
    ];

    for (filesystem, gap, expected) in cases {
        let decision = CrashRecoveryDecision::Unsupported {
            filesystem: filesystem.map(str::to_owned),
            gap,
            handles_ok: true,
        };
        assert_eq!(decision.crash_recovery_warning(), expected);
        assert!(
            !decision.crash_recovery_warning().starts_with("warning: "),
            "the `warning: ` prefix belongs to the drivers"
        );
        assert_eq!(
            decision.crash_recovery_protocol(),
            crate::MergeCrashRecovery {
                supported: false,
                filesystem: filesystem.map(str::to_owned),
                gap: Some(gap),
                handles_ok: Some(true),
            }
        );
    }

    assert_eq!(
        CrashRecoveryDecision::Supported.crash_recovery_protocol(),
        crate::MergeCrashRecovery {
            supported: true,
            filesystem: None,
            gap: None,
            handles_ok: None,
        }
    );
}

/// **M5d step (3): the decision learns HANDLE capability, and says so once**
/// (`GwzM5-8M5d-Charter.md` §3, "Where handle capability is learned").
///
/// Three shapes, one function. Below the bar with the handle probe answering —
/// the btrfs / NFS / tmpfs shape, and the shape a first merge takes on APFS or
/// ext4 with no `.gwz` yet — is `handles_ok = true` and ship (1)'s sentence
/// unchanged. Below the bar with the probe refusing is `handles_ok = false` and
/// the SAME diagnostic with the reverse-door limit appended, never a second
/// one. Above the bar the field is ABSENT.
///
/// The `.gwz`-absence row is the charter's revision-5 ruling (S-P2-3) executed:
/// the probe is the workspace root's, so a workspace that has never merged
/// reads `true` rather than being punished for a private directory that does
/// not exist yet.
#[test]
fn the_decision_learns_handle_capability_at_the_decision_point() {
    let (temp, _backend, _fixture) = workspace("m5d-handle-decision");
    assert!(
        !temp.path().join(".gwz/merge").exists(),
        "the row below is about a workspace with no merge directory yet"
    );

    let handles = with_identity_unavailable(below_bar(Some("btrfs")), || {
        crash_recovery_decision(temp.path())
    })
    .unwrap();
    assert_eq!(
        handles,
        CrashRecoveryDecision::Unsupported {
            filesystem: Some("btrfs".to_owned()),
            gap: crate::MergeCrashRecoveryGap::NoDurableIdentity,
            handles_ok: true,
        },
        "a missing .gwz on a handle-capable volume is not a capability gap"
    );
    assert_eq!(
        handles.crash_recovery_warning(),
        "crash recovery is unsupported on btrfs (no durable filesystem identity). Merge will \
         continue. Use --filesystem-strict to refuse.",
        "handles intact: ship (1)'s sentence, and nothing appended"
    );

    let handle_fail = with_identity_unavailable(below_bar(Some("overlay")), || {
        with_handle_probe_unavailable(|| crash_recovery_decision(temp.path()))
    })
    .unwrap();
    assert_eq!(
        handle_fail,
        CrashRecoveryDecision::Unsupported {
            filesystem: Some("overlay".to_owned()),
            gap: crate::MergeCrashRecoveryGap::NoDurableIdentity,
            handles_ok: false,
        }
    );
    assert_eq!(
        handle_fail.crash_recovery_warning(),
        "crash recovery is unsupported on overlay (no durable filesystem identity). Merge will \
         continue. Use --filesystem-strict to refuse. Selected-root and --preserve abort may \
         refuse until the workspace is on a handle-capable volume.",
        "ONE diagnostic: ship (1)'s sentence byte-identical, then the appended limit"
    );
    let same_volume_with_handles = CrashRecoveryDecision::Unsupported {
        filesystem: Some("overlay".to_owned()),
        gap: crate::MergeCrashRecoveryGap::NoDurableIdentity,
        handles_ok: true,
    };
    assert!(
        handle_fail
            .crash_recovery_warning()
            .starts_with(&same_volume_with_handles.crash_recovery_warning()),
        "the appended clause must not disturb the sentence every doc and driver pin matches"
    );
    assert_eq!(
        handle_fail.crash_recovery_protocol(),
        crate::MergeCrashRecovery {
            supported: false,
            filesystem: Some("overlay".to_owned()),
            gap: Some(crate::MergeCrashRecoveryGap::NoDurableIdentity),
            handles_ok: Some(false),
        }
    );

    // Above the bar the host answers for itself, and the field is absent.
    let above = crash_recovery_decision(temp.path()).unwrap();
    assert_eq!(above, CrashRecoveryDecision::Supported);
    assert_eq!(above.crash_recovery_protocol().handles_ok, None);
}

/// **The raw record create, executed end to end** (charter §3's table row
/// "Handle probe fails … record create: **raw**", and §8's acceptance).
///
/// On a volume that can prove neither durable identity nor persistent file
/// handles, a `--no-ff` start still runs: it warns ONCE with the appended
/// clause, activates no catalog, publishes its v1 record through the raw
/// verified writer instead of the checked boundary, and completes. Before M5d
/// this start died at `create_merge_store_record` AFTER its warning had been
/// printed — ship (1) charter §4.1's recorded limit — which is the regression
/// this row exists to keep closed.
#[test]
fn a_handle_fail_no_ff_start_writes_its_record_raw_and_completes() {
    let (temp, backend, _fixture) = workspace("m5d-handle-fail-start");
    let sink = CollectingSink::default();

    let response = on_a_handle_fail_volume(|| {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            no_ff_request(),
            "op_m5d_handle_fail",
            &sink,
        )
    })
    .unwrap();

    assert_one_handle_fail_diagnostic(&sink);
    assert_raw_record_completed(temp.path(), &response);
}

/// **The same on an ORDINARY `gwz merge`** — charter §8's acceptance sentence,
/// which is about `gwz merge <source>` and not only `--no-ff`.
///
/// `ACTIVE_WRITER_FLOOR` is `V1` on this tree, so an ordinary start writes a v1
/// record and takes exactly the door this step widened. This is the row the
/// workflow's real-overlay job runs first: it is the shape a user actually
/// types.
#[test]
fn a_handle_fail_ordinary_start_writes_its_record_raw_and_completes() {
    let (temp, backend, _fixture) = workspace("m5d-handle-fail-ordinary");
    let sink = CollectingSink::default();

    let response = on_a_handle_fail_volume(|| {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            request(false),
            "op_m5d_handle_fail_ordinary",
            &sink,
        )
    })
    .unwrap();

    assert_one_handle_fail_diagnostic(&sink);
    assert_raw_record_completed(temp.path(), &response);
}

/// **A conflicted handle-fail attempt, resolved and continued** (charter §8:
/// the forward path still runs; §3.1's "a continue is a NEW PROCESS").
///
/// The continue decides for itself on the same volume, reaches the same answer,
/// warns once with the same appended clause, and finishes the merge — all on a
/// record that was created raw.
#[test]
fn a_handle_fail_attempt_continues_after_resolution() {
    let (temp, backend) = conflicting_workspace("m5d-handle-fail-continue");
    let app = temp.path().join("app");
    let start_sink = CollectingSink::default();
    let started = on_a_handle_fail_volume(|| {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            no_ff_request(),
            "op_m5d_hf_start",
            &start_sink,
        )
    })
    .unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    assert!(started.open);
    assert_one_handle_fail_diagnostic(&start_sink);
    assert_eq!(
        started.crash_recovery.as_ref().unwrap().handles_ok,
        Some(false)
    );

    fs::write(app.join("README.md"), "resolved\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&app, &["README.md"])
        .unwrap();

    let sink = CollectingSink::default();
    let continued = on_a_handle_fail_volume(|| {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, started.merge_id.clone()),
            "op_m5d_hf_continue",
            &sink,
        )
    })
    .unwrap();

    assert_one_handle_fail_diagnostic(&sink);
    assert_raw_record_completed(temp.path(), &continued);
}

/// **The reverse doors' refusal names ONE escape, and it is not this door**
/// (charter §3(b)).
///
/// Every reverse checked door — a selected root's artifacts, a preservation
/// bundle, the root preservation image on either root kind — refuses on a
/// handle-fail volume, because the charter forbids reverse-path raw. What
/// changes is the sentence: the substrate remedy advertises `gwz merge
/// --abort`, which IS the door refusing, so the escape offered here is the one
/// that needs neither handles on this volume nor an old binary.
#[test]
fn a_reverse_checked_door_on_a_handle_fail_volume_names_the_one_escape() {
    let (temp, _backend, _fixture) = workspace("m5d-reverse-door");
    let root = temp.path();
    let relative = Path::new("gwz.lock");

    let refusals = with_handle_probe_unavailable(|| {
        vec![
            crate::checked_artifact::entry::observe_merge_root_artifact(root, relative)
                .map(|_| ())
                .unwrap_err(),
            crate::checked_artifact::entry::remove_merge_root_artifact(root, relative, b"x")
                .unwrap_err(),
            crate::checked_artifact::entry::replace_merge_root_artifact(root, relative, b"x", b"y")
                .unwrap_err(),
            crate::checked_artifact::entry::observe_merge_preservation_bundle(
                root,
                Path::new(".gwz/stash/bundle"),
                None,
            )
            .map(|_| ())
            .unwrap_err(),
            crate::checked_artifact::entry::observe_merge_preservation_workspace(
                root, relative, None,
            )
            .map(|_| ())
            .unwrap_err(),
        ]
    });

    for refusal in &refusals {
        assert_eq!(refusal.code, ErrorCode::UnsupportedOperation, "{refusal:?}");
        for named in [
            "does not expose the persistent file handles",
            "copy the whole workspace onto a volume that proves them",
            "`gwz merge --abort` there",
            "--preserve",
            "APFS",
            "ext4",
            "NTFS",
        ] {
            assert!(refusal.message.contains(named), "{}", refusal.message);
        }
        assert!(
            !refusal.message.contains("--filesystem-strict"),
            "the substrate remedy's escapes are circular at this door: {}",
            refusal.message
        );
        assert!(
            !refusal.message.contains("0.13"),
            "an old binary is not an accepted escape for an open v1 record: {}",
            refusal.message
        );
    }
}

/// **Above the bar a handle failure stays an ANOMALY** (charter §3: "Above the
/// bar, a boundary `Unsupported` is still an error").
///
/// With only the handle probe refusing — the catalog's identity bar untouched,
/// so the decision answers `Supported` — the start does NOT take the raw arm
/// and does NOT warn. It fails at the create door with today's unchanged
/// substrate text, because that combination is not a filesystem the charter
/// plans around; it is a volume behaving inconsistently.
#[test]
fn a_handle_failure_above_the_bar_is_still_an_error_with_todays_text() {
    let (temp, backend, _fixture) = workspace("m5d-above-bar-anomaly");
    let sink = CollectingSink::default();

    let error = with_handle_probe_unavailable(|| {
        crate::workspace_ops::handle_merge_with_events(
            &backend,
            temp.path(),
            no_ff_request(),
            "op_m5d_above_bar_anomaly",
            &sink,
        )
    })
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(
        error
            .message
            .contains("durable filesystem identity is unsupported"),
        "{}",
        error.message
    );
    assert!(
        diagnostics(&sink).is_empty(),
        "an above-bar start decides `Supported` and warns nobody"
    );
    assert_eq!(open_records(temp.path()), Vec::<PathBuf>::new());
}
