//! The command gate that refuses a hand-edited `gwz.conf`.
//!
//! # Which commands
//!
//! The gate is a **filter, not a coverage guarantee**. Each structural-mutation handler
//! calls it by hand, and the gate then drops anything whose [`OpenMergeCommand`] decision
//! is not `Block`. So the gated set is `{call sites} ∩ {Block}` — the filter makes it
//! impossible to gate something a merge leaves reachable, but nothing makes a new `Block`
//! command inherit the gate. Two `Block` commands are deliberately outside it:
//!
//! * **`Forall`** — implemented entirely CLI-side, unreachable from this crate. It writes
//!   no conf, so it cannot enshrine an edit; it would act on a fabricated member set.
//! * **`MergeStart`** — dispatched inside `workspace_ops::merge`, which this lane does not
//!   own. Merge-owned dispatch is deliberately ungated.
//!
//! Everything else `Block` is gated, and every production write of `gwz.yml` /
//! `gwz.lock.yml` sits downstream of one of those call sites. The filter is what keeps the
//! gate out of the merge lane's way: a `Block` command already gets `OpenOperation` from
//! `enforce_open_merge_gate` before the conf gate can speak, so the merge lane's own
//! errors are never displaced, and read-only commands stay usable on a damaged workspace.
//!
//! # Which drift
//!
//! A digest mismatch alone is not a hand edit. Everything git produces — a pull, a
//! checkout, a branch switch, the composition commit a root merge writes — leaves the conf
//! file byte-identical to its committed blob while the side-car marker lags behind. Only
//! an *uncommitted* difference can be something a person or an agent typed. So the gate
//! refuses on `mismatch AND uncommitted`, and **reconciles** the marker otherwise, which is
//! what makes a workspace self-heal after every git-side rewrite instead of bricking.
//!
//! # The reconcile is a write
//!
//! It therefore obeys write discipline: it happens only when the caller passes the
//! workspace mutation guard it holds, and never during a dry run. A dry run still *refuses*
//! on positive evidence — it must tell the truth about what the real run would do — it just
//! leaves the tree byte-identical. When the reconcile does write, it also stages the
//! marker, so the repair lands in the same commit as the files it vouches for.
//!
//! # Accepted residuals
//!
//! Layer 2's detection strength is deliberately modest; a false positive bricks a
//! workspace, so every one of these is the correct trade rather than an oversight:
//!
//! * **Hand edit followed by `git commit`.** The file then matches HEAD, so the gate
//!   reconciles and the edit becomes the baseline. Committing an edit is a deliberate act
//!   well past the accidental-agent-edit case this defends.
//! * **Hand edit with a recomputed marker.** The marker is a plain `sha256:<hex>` of file
//!   bytes; anything that can run `shasum` can restate it. This is a guardrail, not a
//!   security boundary.
//! * **`gwz.conf` untracked or gitignored, or a root that is not a Git repository.**
//!   `uncommitted_among` yields the empty set, so the gate reconciles and layer 2 is inert
//!   for that workspace. Nothing enforces that `gwz.conf` is tracked.
//! * **`Forall` and `MergeStart`**, per the exclusions above.
//! * **The staged-`gwz.conf` dirt class.** Gated commands run `sync_workspace_boundary`,
//!   which stages `gwz.conf` wholesale; a reconcile therefore leaves a staged marker.
//!   That class of dirt pre-exists this lane and is accepted as-is.

use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::operation::{OpenMergeCommand, OpenMergeGateDecision};
use crate::workspace_ops::WorkspaceMutationGuard;

/// Whether this run may perform the reconcile **write**.
///
/// `Some` only when the caller holds the workspace mutation guard and is not a dry run —
/// the two conditions together. `guarded_workspace_root` already yields `None` for a dry
/// run, but `acquire_workspace_mutation_guard` does not, so the flag is passed explicitly
/// rather than inferred.
pub(crate) fn reconcile_authority(
    guard: Option<&WorkspaceMutationGuard>,
    dry_run: bool,
) -> Option<&WorkspaceMutationGuard> {
    guard.filter(|_| !dry_run)
}

/// Refuse `command` if `gwz.conf` carries an uncommitted hand edit; reconcile the marker
/// with anything git wrote when `reconcile` grants the authority to write; never fail for
/// an ambiguous state.
pub(crate) fn assert_conf_unmodified_for<B>(
    backend: &B,
    root: &Path,
    command: OpenMergeCommand,
    reconcile: Option<&WorkspaceMutationGuard>,
) -> ModelResult<()>
where
    B: GitBackend,
{
    if command.gate_decision() != OpenMergeGateDecision::Block {
        return Ok(());
    }
    let artifact::ConfIntegrityVerdict::Mismatch(drifted) = artifact::inspect_conf_integrity(root)
    else {
        // Verified, not enrolled, or an unreadable marker: nothing positive to act on.
        return Ok(());
    };

    let uncommitted = uncommitted_among(backend, root, &drifted);
    if !uncommitted.is_empty() {
        // Positive evidence. A dry run refuses too — it reports what the real run would do.
        return Err(artifact::conf_hand_edit_error(&uncommitted));
    }
    // Git moved these files and the marker has not caught up. Repair it, but only with the
    // authority to write; a dry run leaves the tree exactly as it found it.
    if reconcile.is_some() {
        artifact::refresh_conf_integrity_marker(root)?;
        stage_marker(backend, root)?;
    }
    Ok(())
}

/// Stage the repaired marker so it lands in the same commit as the files it vouches for.
/// A root that is not a Git repository has nothing to stage into and is not an error.
fn stage_marker<B>(backend: &B, root: &Path) -> ModelResult<()>
where
    B: GitBackend,
{
    if root.join(artifact::CONF_INTEGRITY_MARKER_PATH).exists()
        && backend.is_repository(root).unwrap_or(false)
    {
        backend.stage_paths(root, &[artifact::CONF_INTEGRITY_MARKER_PATH])?;
    }
    Ok(())
}

/// The subset of `paths` that differ from what the root repo has committed.
///
/// A root that is not a Git repository, or a status call that fails, yields the empty set:
/// without evidence of an uncommitted change this must not refuse.
fn uncommitted_among<B>(backend: &B, root: &Path, paths: &[String]) -> Vec<String>
where
    B: GitBackend,
{
    let Ok(status) = backend.status(root) else {
        return Vec::new();
    };
    paths
        .iter()
        .filter(|path| {
            status.files.iter().any(|file| {
                &&file.path == path
                    && !(file.index_status.trim().is_empty()
                        && file.worktree_status.trim().is_empty())
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::artifact::{ConfIntegrityVerdict, write_manifest};
    use crate::git::Git2Backend;
    use crate::model::ErrorCode;
    use crate::workspace::WORKSPACE_MANIFEST;
    use crate::workspace_ops::tests::TempDir;

    /// Every `OpenMergeCommand`, exhaustive **by construction**: the match below has no
    /// wildcard arm, so a new enum variant fails to compile here until it is classified.
    fn every_command() -> Vec<OpenMergeCommand> {
        use OpenMergeCommand as C;

        let mut all = Vec::new();
        let mut next = Some(C::StageConflictResolution);
        while let Some(command) = next {
            all.push(command);
            next = match command {
                C::StageConflictResolution => Some(C::BranchList),
                C::BranchList => Some(C::BranchMutate),
                C::BranchMutate => Some(C::Capture),
                C::Capture => Some(C::CloneWorkspace),
                C::CloneWorkspace => Some(C::Commit),
                C::Commit => Some(C::Diff),
                C::Diff => Some(C::Forall),
                C::Forall => Some(C::InitNewWorkspace),
                C::InitNewWorkspace => Some(C::InitExistingPlan),
                C::InitExistingPlan => Some(C::InitUpdate),
                C::InitUpdate => Some(C::Ls),
                C::Ls => Some(C::Materialize),
                C::Materialize => Some(C::Pull),
                C::Pull => Some(C::Push),
                C::Push => Some(C::RepoMutate),
                C::RepoMutate => Some(C::Snapshot),
                C::Snapshot => Some(C::SnapshotList),
                C::SnapshotList => Some(C::StashList),
                C::StashList => Some(C::StashMutate),
                C::StashMutate => Some(C::Status),
                C::Status => Some(C::TagList),
                C::TagList => Some(C::TagMutate),
                C::TagMutate => Some(C::MergeStatus),
                C::MergeStatus => Some(C::MergeRecovery),
                C::MergeRecovery => Some(C::MergeGc),
                C::MergeGc => Some(C::MergeStart),
                C::MergeStart => None,
            };
        }
        all
    }

    /// A committed workspace: manifest, lock, and marker all in HEAD.
    fn committed_workspace(name: &str) -> (TempDir, Git2Backend) {
        let temp = temp_dir(name);
        let backend = Git2Backend::new();
        backend.create_repo(&temp.path).unwrap();
        write_manifest(&temp.path, &sample()).unwrap();
        commit_all(&temp.path);
        (temp, backend)
    }

    fn temp_dir(name: &str) -> TempDir {
        // Nanos, not `{:?}`: SystemTime's Debug rendering carries braces,
        // spaces, and a colon, and a colon is not a legal Windows filename
        // character -- every test in this module failed its fixture with
        // os error 267 on the first Windows dispatch (2026-08-29).
        let path = std::env::temp_dir().join(format!(
            "gwz-core-conf-gate-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    fn sample() -> crate::artifact::ManifestArtifact {
        crate::artifact::ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: crate::artifact::WorkspaceHeader {
                id: "ws_gate".to_owned(),
            },
            members: Vec::new(),
        }
    }

    fn retype_manifest(root: &Path, from: &str, to: &str) {
        let path = root.join(WORKSPACE_MANIFEST);
        let edited = fs::read_to_string(&path).unwrap().replace(from, to);
        fs::write(&path, edited).unwrap();
    }

    /// `git status --porcelain`-shaped view of the root, for byte-identity assertions.
    fn porcelain(backend: &Git2Backend, root: &Path) -> Vec<String> {
        let mut lines: Vec<String> = backend
            .status(root)
            .unwrap()
            .files
            .iter()
            .map(|file| {
                format!(
                    "{}{} {}",
                    file.index_status, file.worktree_status, file.path
                )
            })
            .collect();
        lines.sort();
        lines
    }

    fn commit_all(root: &Path) {
        let repo = git2::Repository::open(root).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
        let parents: Vec<_> = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .into_iter()
            .map(|id| repo.find_commit(id).unwrap())
            .collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "state",
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap();
    }

    /// The real guard a non-dry-run handler holds, over the workspace under test.
    fn held(root: &Path) -> WorkspaceMutationGuard {
        crate::workspace_ops::acquire_workspace_mutation_guard(
            root,
            None,
            OpenMergeCommand::RepoMutate,
            false,
        )
        .unwrap()
        .into_guard()
        .expect("a non-dry-run acquisition yields the mutating arm")
    }

    #[test]
    fn an_uncommitted_hand_edit_refuses_a_blocking_command() {
        let (temp, backend) = committed_workspace("hand-edit");
        retype_manifest(&temp.path, "ws_gate", "ws_typed");

        let guard = held(&temp.path);
        let error = assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(Some(&guard), false),
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert!(error.message.contains(WORKSPACE_MANIFEST));
    }

    #[test]
    fn a_committed_git_side_rewrite_is_reconciled_and_the_marker_is_staged() {
        // What `git pull`, a checkout, or a root merge's composition commit leaves behind:
        // the conf file moved with the repo and the marker lagged. Refusing here is the
        // bricking failure mode, so the gate repairs — and stages what it repaired, so the
        // fix rides the next commit instead of lingering as unstaged dirt.
        let (temp, backend) = committed_workspace("git-rewrite");
        retype_manifest(&temp.path, "ws_gate", "ws_pulled");
        commit_all(&temp.path);
        assert!(artifact::inspect_conf_integrity(&temp.path).refuses());

        let guard = held(&temp.path);
        assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(Some(&guard), false),
        )
        .unwrap();

        assert_eq!(
            artifact::inspect_conf_integrity(&temp.path),
            ConfIntegrityVerdict::Verified
        );
        assert!(
            porcelain(&backend, &temp.path).iter().any(|line| {
                line.starts_with('M') && line.ends_with(artifact::CONF_INTEGRITY_MARKER_PATH)
            }),
            "the repaired marker was not staged: {:?}",
            porcelain(&backend, &temp.path)
        );
    }

    #[test]
    fn a_dry_run_refuses_a_real_hand_edit_but_writes_nothing() {
        // [P1-1] A dry run must tell the truth about what the real run would do, and must
        // leave the tree byte-identical while doing it.
        let (temp, backend) = committed_workspace("dry-run-refuses");
        retype_manifest(&temp.path, "ws_gate", "ws_typed");
        let before = porcelain(&backend, &temp.path);

        let error = assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(None, true),
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(porcelain(&backend, &temp.path), before);
    }

    #[test]
    fn a_dry_run_never_reconciles_and_leaves_porcelain_byte_identical() {
        // The probe that failed review: a dry run over a stale-but-clean marker used to
        // rewrite it, leaving `M gwz.conf/markers/conf-integrity.yml` behind.
        let (temp, backend) = committed_workspace("dry-run-pure");
        retype_manifest(&temp.path, "ws_gate", "ws_pulled");
        commit_all(&temp.path);
        assert!(artifact::inspect_conf_integrity(&temp.path).refuses());
        let before = porcelain(&backend, &temp.path);
        let marker_before = fs::read(temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH)).unwrap();

        assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(None, true),
        )
        .unwrap();

        assert_eq!(porcelain(&backend, &temp.path), before);
        assert_eq!(
            fs::read(temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH)).unwrap(),
            marker_before
        );
        // Still drifted, because nothing was repaired.
        assert!(artifact::inspect_conf_integrity(&temp.path).refuses());
    }

    #[test]
    fn the_reconcile_write_requires_the_mutation_guard() {
        // Guard discipline: without the guard the gate reports honestly and writes nothing,
        // even outside a dry run.
        let (temp, backend) = committed_workspace("no-guard");
        retype_manifest(&temp.path, "ws_gate", "ws_pulled");
        commit_all(&temp.path);
        let marker_before = fs::read(temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH)).unwrap();

        assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(None, false),
        )
        .unwrap();

        assert_eq!(
            fs::read(temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH)).unwrap(),
            marker_before
        );
    }

    #[test]
    fn a_tampered_marker_alone_is_repaired_because_the_files_match_head() {
        // [P2-6] The shipped semantics, asserted honestly. Rewriting the marker alone is
        // NOT caught: the conf files still match HEAD, so this is indistinguishable from
        // git having moved them, and the gate repairs the marker. That is an accepted
        // residual (see the module docs), not a safety property.
        let (temp, backend) = committed_workspace("marker-tamper");
        let marker_path = temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH);
        let tampered = fs::read_to_string(&marker_path)
            .unwrap()
            .replace("sha256:", "sha256:0");
        fs::write(&marker_path, tampered).unwrap();
        assert!(artifact::inspect_conf_integrity(&temp.path).refuses());

        let guard = held(&temp.path);
        assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(Some(&guard), false),
        )
        .unwrap();

        assert_eq!(
            artifact::inspect_conf_integrity(&temp.path),
            ConfIntegrityVerdict::Verified
        );
    }

    #[test]
    fn a_tampered_marker_with_an_uncommitted_file_still_refuses() {
        // The genuine refusal that survives: the marker moved AND a conf file differs from
        // what is committed. Only the uncommitted file is named, which is the honest set.
        let (temp, backend) = committed_workspace("marker-tamper-dirty");
        let marker_path = temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH);
        let tampered = fs::read_to_string(&marker_path)
            .unwrap()
            .replace("sha256:", "sha256:0");
        fs::write(&marker_path, tampered).unwrap();
        retype_manifest(&temp.path, "ws_gate", "ws_typed");

        let guard = held(&temp.path);
        let error = assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(Some(&guard), false),
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert!(error.message.contains(WORKSPACE_MANIFEST));
    }

    #[test]
    fn only_commands_an_open_merge_blocks_are_gated() {
        // The load-bearing filter property, over EVERY variant of the enum. Anything
        // reachable during a merge passes a drifted workspace through untouched, because
        // the merge lane owns those bytes.
        let (temp, backend) = committed_workspace("gate-scope");
        retype_manifest(&temp.path, "ws_gate", "ws_typed");
        let commands = every_command();
        assert_eq!(
            commands.len(),
            27,
            "the enum grew; classify the new variant"
        );

        let guard = held(&temp.path);
        for command in commands {
            let refused = assert_conf_unmodified_for(
                &backend,
                &temp.path,
                command,
                reconcile_authority(Some(&guard), false),
            )
            .is_err();
            assert_eq!(
                refused,
                command.gate_decision() == OpenMergeGateDecision::Block,
                "{command:?} gated against its own open-merge decision"
            );
        }
    }

    #[test]
    fn a_workspace_with_no_marker_passes_every_command() {
        let (temp, backend) = committed_workspace("grandfather");
        fs::remove_file(temp.path.join(artifact::CONF_INTEGRITY_MARKER_PATH)).unwrap();
        retype_manifest(&temp.path, "ws_gate", "ws_typed");

        let guard = held(&temp.path);
        assert_conf_unmodified_for(
            &backend,
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(Some(&guard), false),
        )
        .unwrap();
    }

    #[test]
    fn a_root_that_is_not_a_git_repository_never_refuses() {
        // Without a repo there is no committed blob to compare against, so there is no
        // evidence of a hand edit — and a fixture workspace must not be bricked. The
        // reconcile must also not blow up trying to stage into a non-repo.
        let temp = temp_dir("bare");
        write_manifest(&temp.path, &sample()).unwrap();
        retype_manifest(&temp.path, "ws_gate", "ws_typed");
        assert!(artifact::inspect_conf_integrity(&temp.path).refuses());

        // No mutation guard exists for a non-workspace root, so the gate is asked without
        // reconcile authority: the property under test is that it does not refuse.
        assert_conf_unmodified_for(
            &Git2Backend::new(),
            &temp.path,
            OpenMergeCommand::RepoMutate,
            reconcile_authority(None, false),
        )
        .unwrap();

        // And the reconcile's staging step is a no-op rather than an error there, so a
        // reconcile reached with authority cannot fail on a non-repo either.
        stage_marker(&Git2Backend::new(), &temp.path).unwrap();
    }
}
