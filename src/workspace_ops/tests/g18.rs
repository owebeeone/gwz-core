use std::collections::BTreeMap;

use crate::artifact::{
    self, ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember,
    ResolvedMemberArtifact, WorkspaceHeader,
};

use super::*;

// P1.2: handle_ls — list members from manifest + lock (no git); materialized filter + selection.

fn member(id: &str, path: &str) -> ManifestMember {
    let name = path.rsplit('/').next().unwrap_or(path);
    ManifestMember {
        id: id.to_owned(),
        path: path.to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: format!("src_{name}"),
        active: true,
        desired: None,
        remotes: Vec::new(),
    }
}

/// Write a manifest with `members`, and a lock that records `materialized` ids as materialized.
fn write_workspace(temp: &std::path::Path, members: Vec<ManifestMember>, materialized: &[&str]) {
    let manifest = ManifestArtifact {
        schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_ops".to_owned(),
        },
        members,
    };
    artifact::write_manifest(temp, &manifest).unwrap();

    let mut lock_members = BTreeMap::new();
    for &id in materialized {
        let path = manifest
            .members
            .iter()
            .find(|member| member.id == id)
            .expect("materialized id is a manifest member")
            .path
            .clone();
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        lock_members.insert(
            id.to_owned(),
            ResolvedMemberArtifact {
                path,
                source_id: Some(format!("src_{name}")),
                source_kind: ArtifactSourceKind::Git,
                commit: Some("abc123def456".to_owned()),
                branch: Some("main".to_owned()),
                detached: Some(false),
                upstream: None,
                dirty: Some(false),
                materialized: Some(true),
            },
        );
    }
    artifact::write_lock(
        temp,
        &LockArtifact {
            schema: artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_ops".to_owned(),
            manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            members: lock_members,
        },
    )
    .unwrap();
}

fn ls_request(member_ids: &[&str], include_unmaterialized: bool) -> crate::LsRequest {
    let mut meta = request_meta();
    if !member_ids.is_empty() {
        meta.selection = Some(crate::Selection {
            all: None,
            member_ids: member_ids.iter().map(|id| id.to_string()).collect(),
            paths: Vec::new(),
            targets: Vec::new(),
            exclude_targets: Vec::new(),
        });
    }
    crate::LsRequest {
        meta,
        include_unmaterialized: include_unmaterialized.then_some(true),
    }
}

fn ids(response: &crate::LsResponse) -> Vec<String> {
    response
        .members
        .as_ref()
        .unwrap()
        .iter()
        .map(|member| member.id.clone())
        .collect()
}

#[test]
fn lists_materialized_members_by_default() {
    let temp = TempDir::new("ls-default");
    write_workspace(
        temp.path(),
        vec![
            member("mem_app", "repos/app"),
            member("mem_lib", "repos/lib"),
        ],
        &["mem_app"],
    );

    let response = handle_ls(temp.path(), ls_request(&[], false), "op").unwrap();
    assert_eq!(
        ids(&response),
        vec!["mem_app"],
        "only the materialized member"
    );

    let entry = &response.members.unwrap()[0];
    assert!(entry.materialized);
    assert_eq!(entry.path, "repos/app");
    assert!(
        entry.abspath.ends_with("repos/app"),
        "abspath: {}",
        entry.abspath
    );
    assert!(std::path::Path::new(&entry.abspath).is_absolute());
}

#[test]
fn include_unmaterialized_lists_all() {
    let temp = TempDir::new("ls-all");
    write_workspace(
        temp.path(),
        vec![
            member("mem_app", "repos/app"),
            member("mem_lib", "repos/lib"),
        ],
        &["mem_app"],
    );

    let response = handle_ls(temp.path(), ls_request(&[], true), "op").unwrap();
    assert_eq!(ids(&response), vec!["mem_app", "mem_lib"]);
    let lib = response
        .members
        .unwrap()
        .into_iter()
        .find(|member| member.id == "mem_lib")
        .unwrap();
    assert!(
        !lib.materialized,
        "mem_lib has no lock entry → not materialized"
    );
}

#[test]
fn selection_scopes_the_listing() {
    let temp = TempDir::new("ls-sel");
    write_workspace(
        temp.path(),
        vec![
            member("mem_app", "repos/app"),
            member("mem_lib", "repos/lib"),
        ],
        &["mem_app", "mem_lib"],
    );

    let response = handle_ls(temp.path(), ls_request(&["mem_lib"], false), "op").unwrap();
    assert_eq!(ids(&response), vec!["mem_lib"]);
}

#[test]
fn empty_workspace_lists_nothing() {
    let temp = TempDir::new("ls-empty");
    write_workspace(temp.path(), Vec::new(), &[]);
    let response = handle_ls(temp.path(), ls_request(&[], false), "op").unwrap();
    assert!(response.members.unwrap().is_empty());
}
