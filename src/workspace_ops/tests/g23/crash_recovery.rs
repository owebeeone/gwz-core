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

use crate::checked_artifact::entry::CrashRecoveryDecision;
use crate::checked_artifact::{InjectedVolumeDescription, with_identity_unavailable};

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
            }
        );
    }

    assert_eq!(
        CrashRecoveryDecision::Supported.crash_recovery_protocol(),
        crate::MergeCrashRecovery {
            supported: true,
            filesystem: None,
            gap: None,
        }
    );
}
