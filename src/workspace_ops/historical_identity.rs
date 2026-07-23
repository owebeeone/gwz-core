use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// One unique historical commit for a member, with every artifact that recorded it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HistoricalCommitEvidence {
    pub(crate) commit: String,
    pub(crate) provenance: Vec<String>,
}

pub(crate) type HistoricalIdentityEvidence = BTreeMap<String, Vec<HistoricalCommitEvidence>>;

/// Collect, per requested member, every non-null commit recorded by snapshots and
/// commit markers. Commits are deduplicated while all provenance labels are retained.
/// Artifact readers fail closed, so an unreadable snapshot or marker aborts collection.
pub(crate) fn historical_member_commits(
    root: &Path,
    member_ids: &[String],
) -> ModelResult<HistoricalIdentityEvidence> {
    let requested: BTreeSet<&str> = member_ids.iter().map(String::as_str).collect();
    let mut collected: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = member_ids
        .iter()
        .map(|member_id| (member_id.clone(), BTreeMap::new()))
        .collect();

    for snapshot in artifact::list_snapshots(root)? {
        collect_artifact_members(
            &requested,
            &mut collected,
            format!("snapshot:{}", snapshot.snapshot_id),
            &snapshot.members,
        );
    }
    for marker in artifact::list_markers(root)? {
        collect_artifact_members(
            &requested,
            &mut collected,
            format!("marker:{}", marker.gwz_commit_id),
            &marker.members,
        );
    }

    Ok(collected
        .into_iter()
        .map(|(member_id, commits)| {
            let evidence = commits
                .into_iter()
                .map(|(commit, provenance)| HistoricalCommitEvidence {
                    commit,
                    provenance: provenance.into_iter().collect(),
                })
                .collect();
            (member_id, evidence)
        })
        .collect())
}

fn collect_artifact_members(
    requested: &BTreeSet<&str>,
    collected: &mut BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    provenance: String,
    members: &BTreeMap<String, artifact::ResolvedMemberArtifact>,
) {
    for (member_id, member) in members {
        if !requested.contains(member_id.as_str()) {
            continue;
        }
        let Some(commit) = member.commit.as_ref() else {
            continue;
        };
        collected
            .entry(member_id.clone())
            .or_default()
            .entry(commit.clone())
            .or_default()
            .insert(provenance.clone());
    }
}

/// Verify every unique recorded commit against the local repository. No network access
/// is performed. Empty evidence is a successful zero-count result; the caller decides
/// whether that is sufficient for its explicit or inferred identity flow.
pub(crate) fn verify_historical_identity<B: GitBackend>(
    backend: &B,
    repo: &Path,
    evidence: &[HistoricalCommitEvidence],
) -> ModelResult<usize> {
    let mut missing = Vec::new();
    for item in evidence {
        if !backend.commit_exists(repo, &item.commit)? {
            missing.push(format!("{} ({})", item.commit, item.provenance.join(", ")));
        }
    }
    if missing.is_empty() {
        Ok(evidence.len())
    } else {
        Err(ModelError::new(
            ErrorCode::SourceIdentityMismatch,
            format!(
                "repository is missing historical commit(s): {}; fetch the required history before retrying (GWZ does not fetch automatically)",
                missing.join("; ")
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::artifact::{
        ArtifactSourceKind, CreatedByArtifact, MarkerArtifact, MarkerRootArtifact,
        ResolvedMemberArtifact, SnapshotArtifact,
    };
    use crate::git::{Git2Backend, GitBackend};

    use super::*;

    #[test]
    fn collects_deduplicated_snapshot_and_marker_commits_with_provenance() {
        let temp = TempDir::new("historical-collect");
        let commit = "1111111111111111111111111111111111111111";
        artifact::write_snapshot(temp.path(), &snapshot("snap_one", Some(commit))).unwrap();
        artifact::write_marker(temp.path(), &marker(Some(commit))).unwrap();
        artifact::write_snapshot(temp.path(), &snapshot("snap_empty", None)).unwrap();

        let evidence = historical_member_commits(temp.path(), &["mem_app".to_owned()]).unwrap();

        assert_eq!(
            evidence["mem_app"],
            vec![HistoricalCommitEvidence {
                commit: commit.to_owned(),
                provenance: vec![
                    "marker:01987b0c-2f75-7c4a-9a32-8fd22f7d7c91".to_owned(),
                    "snapshot:snap_one".to_owned(),
                ],
            }]
        );
    }

    #[test]
    fn unreadable_historical_artifact_fails_closed() {
        let temp = TempDir::new("historical-invalid");
        let dir = temp.path().join(artifact::SNAPSHOT_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.yaml"), "not: [valid").unwrap();

        assert!(historical_member_commits(temp.path(), &["mem_app".to_owned()]).is_err());
    }

    #[test]
    fn verification_checks_every_commit_and_reports_all_missing_provenance() {
        let temp = TempDir::new("historical-verify");
        let repo = temp.path().join("repo");
        let backend = Git2Backend::new();
        backend.create_repo(&repo).unwrap();
        fs::write(repo.join("tracked.txt"), "one\n").unwrap();
        run_git(&repo, &["add", "tracked.txt"]);
        run_git(&repo, &["commit", "-m", "initial"]);
        let existing = backend.head(&repo).unwrap().commit.unwrap();
        let missing = "0000000000000000000000000000000000000000";
        let evidence = vec![
            HistoricalCommitEvidence {
                commit: existing,
                provenance: vec!["snapshot:snap_one".to_owned()],
            },
            HistoricalCommitEvidence {
                commit: missing.to_owned(),
                provenance: vec!["marker:marker_one".to_owned()],
            },
        ];

        let error = verify_historical_identity(&backend, &repo, &evidence).unwrap_err();
        assert_eq!(error.code, ErrorCode::SourceIdentityMismatch);
        assert!(error.message.contains(missing));
        assert!(error.message.contains("marker:marker_one"));
        assert_eq!(verify_historical_identity(&backend, &repo, &[]).unwrap(), 0);
    }

    fn snapshot(snapshot_id: &str, commit: Option<&str>) -> SnapshotArtifact {
        SnapshotArtifact {
            schema: artifact::SNAPSHOT_SCHEMA.to_owned(),
            workspace_id: "ws_test".to_owned(),
            snapshot_id: snapshot_id.to_owned(),
            created_at: "2026-07-10T00:00:00Z".to_owned(),
            created_by: CreatedByArtifact {
                actor_id: "agent_test".to_owned(),
            },
            selected_members: vec!["mem_app".to_owned()],
            members: [("mem_app".to_owned(), resolved(commit))].into(),
        }
    }

    fn marker(commit: Option<&str>) -> MarkerArtifact {
        MarkerArtifact {
            schema: artifact::MARKER_SCHEMA.to_owned(),
            gwz_commit_id: "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91".to_owned(),
            workspace_id: "ws_test".to_owned(),
            origin_url_hash: None,
            created_at: "2026-07-10T00:00:00Z".to_owned(),
            created_by: CreatedByArtifact {
                actor_id: "agent_test".to_owned(),
            },
            root: MarkerRootArtifact {
                path: ".".to_owned(),
                before_commit: None,
                branch: Some("main".to_owned()),
            },
            selected_targets: vec!["mem_app".to_owned()],
            committed_targets: vec!["mem_app".to_owned()],
            members: [("mem_app".to_owned(), resolved(commit))].into(),
            merge: None,
        }
    }

    fn resolved(commit: Option<&str>) -> ResolvedMemberArtifact {
        ResolvedMemberArtifact {
            path: "repos/app".to_owned(),
            source_id: Some("src_app".to_owned()),
            source_kind: ArtifactSourceKind::Git,
            commit: commit.map(ToOwned::to_owned),
            materialized: Some(true),
            ..ResolvedMemberArtifact::default()
        }
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=GWZ",
                "-c",
                "user.email=gwz@example.invalid",
            ])
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("gwz-core-{name}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
