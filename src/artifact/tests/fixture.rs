//! Golden artifact bytes, sample records and the temp-directory helper
//! shared by every test in this module.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// The banner is part of the golden bytes: it must survive every regeneration, so a
/// change that drops it fails the round-trip pins below.
pub(crate) const MANIFEST_GOLDEN: &str = "# Machine-managed by gwz. Hand edits to gwz.conf/ are detected and refused.\n# Structural changes: `gwz repo <add|clone|create|detach|attach|sync>`.\n# Already edited? Revert it, or `gwz init --update --force` to accept this state.\nschema: gwz.workspace/v0\nworkspace:\n  id: ws_01\nmembers:\n- id: mem_01\n  path: repos/example\n  type: git\n  source_id: src_01\n  active: true\n  desired:\n    branch: main\n  remotes:\n  - name: origin\n    url: git@example.invalid:example.git\n    fetch: true\n    push: true\n";

pub(crate) const LOCK_GOLDEN: &str = "schema: gwz.lock/v0\nworkspace_id: ws_01\nmanifest_schema: gwz.workspace/v0\nmembers:\n  mem_01:\n    path: repos/example\n    source_id: src_01\n    source_kind: git\n    commit: abc123\n    branch: main\n    detached: false\n    upstream: origin/main\n    dirty: false\n    materialized: true\n";

pub(crate) const SNAPSHOT_GOLDEN: &str = "schema: gwz.snapshot/v0\nworkspace_id: ws_01\nsnapshot_id: snap_demo\ncreated_at: 2026-06-15T00:00:00Z\ncreated_by:\n  actor_id: agent_01\nselected_members:\n- mem_01\nmembers:\n  mem_01:\n    path: repos/example\n    source_kind: git\n    commit: abc123\n";

pub(crate) fn sample_manifest() -> ManifestArtifact {
    ManifestArtifact {
        schema: WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_01".to_owned(),
        },
        members: vec![ManifestMember {
            private: false,
            id: "mem_01".to_owned(),
            path: "repos/example".to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: "src_01".to_owned(),
            active: true,
            desired: Some(DesiredRefArtifact {
                branch: Some("main".to_owned()),
                ..DesiredRefArtifact::default()
            }),
            remotes: vec![RemoteArtifact {
                name: "origin".to_owned(),
                url: "git@example.invalid:example.git".to_owned(),
                fetch: true,
                push: true,
            }],
        }],
    }
}

pub(crate) fn sample_lock() -> LockArtifact {
    LockArtifact {
        schema: LOCK_SCHEMA.to_owned(),
        workspace_id: "ws_01".to_owned(),
        manifest_schema: WORKSPACE_SCHEMA.to_owned(),
        members: [("mem_01".to_owned(), sample_resolved_member())].into(),
    }
}

pub(crate) fn sample_snapshot() -> SnapshotArtifact {
    SnapshotArtifact {
        schema: SNAPSHOT_SCHEMA.to_owned(),
        workspace_id: "ws_01".to_owned(),
        snapshot_id: "snap_demo".to_owned(),
        created_at: "2026-06-15T00:00:00Z".to_owned(),
        created_by: CreatedByArtifact {
            actor_id: "agent_01".to_owned(),
        },
        selected_members: vec!["mem_01".to_owned()],
        members: [("mem_01".to_owned(), sample_short_member())].into(),
    }
}

pub(crate) fn sample_marker() -> MarkerArtifact {
    MarkerArtifact {
        schema: MARKER_SCHEMA.to_owned(),
        gwz_commit_id: "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91".to_owned(),
        workspace_id: "ws_01".to_owned(),
        origin_url_hash: Some(format!("sha256:{}", "0".repeat(64))),
        created_at: "2026-06-15T00:00:00Z".to_owned(),
        created_by: CreatedByArtifact {
            actor_id: "agent_01".to_owned(),
        },
        root: MarkerRootArtifact {
            path: ".".to_owned(),
            before_commit: Some("abc123".to_owned()),
            branch: Some("main".to_owned()),
        },
        selected_targets: vec!["@root".to_owned(), "mem_01".to_owned()],
        committed_targets: vec!["mem_01".to_owned(), "@root".to_owned()],
        members: [("mem_01".to_owned(), sample_short_member())].into(),
        merge: None,
    }
}

pub(crate) fn sample_resolved_member() -> ResolvedMemberArtifact {
    ResolvedMemberArtifact {
        path: "repos/example".to_owned(),
        source_id: Some("src_01".to_owned()),
        source_kind: ArtifactSourceKind::Git,
        commit: Some("abc123".to_owned()),
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: Some("origin/main".to_owned()),
        dirty: Some(false),
        materialized: Some(true),
    }
}

pub(crate) fn sample_short_member() -> ResolvedMemberArtifact {
    ResolvedMemberArtifact {
        path: "repos/example".to_owned(),
        source_kind: ArtifactSourceKind::Git,
        commit: Some("abc123".to_owned()),
        ..ResolvedMemberArtifact::default()
    }
}

pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("gwz-core-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
