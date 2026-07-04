//! A real-on-disk GWZ workspace fixture for the D3 handler/producer tests.
//!
//! Builds a temp workspace with a `gwz.conf/gwz.yml` manifest, a workspace-root
//! Git repo, and one or more materialized Git member repos, so `handle_diff` runs
//! end to end (plan → manifest → producer → output log) against actual libgit2
//! repositories. The root member-exclusion (AD11) is exercised because members
//! live under the root worktree.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{
    self, ArtifactSourceKind, ManifestArtifact, ManifestMember, WorkspaceHeader,
};
use crate::git::GitBackend;
use crate::protocol::generated::{DiffOptions, DiffRequest, RequestMeta, WorkspaceRef};

pub(crate) const WS_ID: &str = "ws_diff";

/// A self-cleaning temp workspace root.
pub(crate) struct Workspace {
    pub(crate) root: PathBuf,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Workspace {
    /// Create a workspace root with an initialized root Git repo and an empty
    /// manifest (no members yet). Add members with [`add_member`].
    pub(crate) fn new(tag: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("gwz-core-d3-{tag}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        init_repo(&root);
        let manifest = ManifestArtifact {
            schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: WS_ID.to_owned(),
            },
            members: Vec::new(),
        };
        artifact::write_manifest(&root, &manifest).unwrap();
        Workspace { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Register + initialize a materialized Git member at `path` (relative to the
    /// workspace root) with id `member_id`.
    pub(crate) fn add_member(&self, member_id: &str, path: &str) -> PathBuf {
        let member_root = self.root.join(path);
        init_repo(&member_root);
        // Give the member an initial commit so the root repo (which sees the
        // member as an embedded gitlink) never trips "does not have a commit
        // checked out" during its own `git add -A`.
        run_git(
            &member_root,
            &["commit", "--allow-empty", "-m", "member init"],
        );
        let mut manifest = artifact::read_manifest(&self.root).unwrap();
        manifest.members.push(ManifestMember {
            id: member_id.to_owned(),
            path: path.to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: "src_x".to_owned(),
            active: true,
            desired: None,
            remotes: Vec::new(),
        });
        artifact::write_manifest(&self.root, &manifest).unwrap();
        member_root
    }

    /// Write a file under a repo (workspace root or a member root).
    pub(crate) fn write(repo: &Path, rel: &str, contents: &[u8]) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, contents).unwrap();
    }

    pub(crate) fn remove(repo: &Path, rel: &str) {
        let _ = fs::remove_file(repo.join(rel));
    }

    pub(crate) fn commit(repo: &Path, message: &str) {
        run_git(repo, &["add", "-A"]);
        run_git(repo, &["commit", "-m", message]);
    }

    /// A `DiffRequest` scoped to this workspace (root passed explicitly so
    /// discovery is deterministic), with the given options and no operands.
    pub(crate) fn request(&self, options: DiffOptions) -> DiffRequest {
        DiffRequest {
            meta: RequestMeta {
                request_id: "req_1".to_owned(),
                schema_version: "gwz.proto/v0".to_owned(),
                workspace: Some(WorkspaceRef {
                    root: Some(self.root.to_string_lossy().into_owned()),
                    workspace_id: Some(WS_ID.to_owned()),
                }),
                ..Default::default()
            },
            options: Some(options),
            ..Default::default()
        }
    }
}

pub(crate) fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    crate::git::Git2Backend::new().create_repo(root).unwrap();
    run_git(root, &["config", "user.name", "GWZ"]);
    run_git(root, &["config", "user.email", "gwz@example.invalid"]);
    run_git(root, &["config", "core.autocrlf", "false"]);
    run_git(root, &["config", "diff.renames", "true"]);
}

pub(crate) fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
