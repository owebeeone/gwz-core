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
        private: false,
        id: id.to_owned(),
        path: path.to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: format!("src_{name}"),
        active: true,
        desired: None,
        remotes: Vec::new(),
    }
}

/// Write a manifest with `members`, and a lock that records `materialized` ids
/// as materialized. Every such member's worktree directory is created too:
/// `materialized` in an `ls` listing is the FILESYSTEM's answer, not the
/// lock's claim (GwzOpenDecisions D3), so a fixture that records a member
/// without putting it on disk is describing the quiet-clone bug rather than
/// an ordinary workspace. `write_workspace_leaving_absent` is for that case.
fn write_workspace(temp: &std::path::Path, members: Vec<ManifestMember>, materialized: &[&str]) {
    write_workspace_leaving_absent(temp, members, materialized, &[]);
}

/// As `write_workspace`, but the ids in `absent` get their lock row and no
/// directory: what `gwz clone` leaves behind when it quietly skips a private
/// member whose access was refused (it removes the directory and rewrites no
/// lock).
fn write_workspace_leaving_absent(
    temp: &std::path::Path,
    members: Vec<ManifestMember>,
    materialized: &[&str],
    absent: &[&str],
) {
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
        if !absent.contains(&id) {
            std::fs::create_dir_all(temp.join(&lock_members[id].path)).unwrap();
        }
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
fn forall_resolves_root_and_members_with_its_own_action() {
    let temp = TempDir::new("forall-target-service");
    write_workspace(
        temp.path(),
        vec![
            member("mem_app", "repos/app"),
            member("mem_lib", "repos/lib"),
        ],
        &["mem_app"],
    );
    let mut request = ls_request(&[], false);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["@all".into()],
        ..Default::default()
    });
    let listed = resolve_forall_targets(temp.path(), request, "op_forall_targets").unwrap();
    assert_eq!(listed.response.meta.action, crate::ActionKind::Forall);
    assert_eq!(ids(&listed), ["@root", "mem_app"]);
    assert_eq!(
        listed.members.as_ref().unwrap()[0].target_kind,
        Some(crate::TargetKind::Root)
    );
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
        std::path::Path::new(&entry.abspath).ends_with(std::path::Path::new("repos").join("app")),
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

/// GwzOpenDecisions D3. `gwz clone` of a workspace quietly skips a private
/// member whose access is refused: the directory is removed, no row is
/// returned, and the lock is not rewritten (a clone materializes a LOCK
/// target), so the lock goes on recording the member as materialized. The
/// listing must not repeat that claim. The row is still listed -- hiding it
/// is what made the discrepancy invisible -- with `materialized: false` and
/// the reason.
#[test]
fn a_privately_skipped_member_is_listed_unmaterialized_with_its_reason() {
    let temp = TempDir::new("ls-private-skipped");
    let mut secret = member("mem_secret", "repos/secret");
    secret.private = true;
    write_workspace_leaving_absent(
        temp.path(),
        vec![member("mem_app", "repos/app"), secret],
        &["mem_app", "mem_secret"],
        &["mem_secret"],
    );

    // Listed by default: the lock claims it, so it is not silently dropped.
    let response = handle_ls(temp.path(), ls_request(&[], false), "op").unwrap();
    assert_eq!(ids(&response), vec!["mem_app", "mem_secret"]);
    let members = response.members.unwrap();
    let app = members.iter().find(|m| m.id == "mem_app").unwrap();
    assert!(app.materialized, "the public member really is on disk");
    assert_eq!(app.note, None, "an ordinary row carries no note");

    let secret = members.iter().find(|m| m.id == "mem_secret").unwrap();
    assert!(
        !secret.materialized,
        "nothing is on disk, whatever the lock says"
    );
    assert_eq!(secret.note.as_deref(), Some("private, skipped"));
    assert!(!std::path::Path::new(&secret.abspath).exists());
}

/// The same disagreement on a member that is not private -- a directory
/// removed by hand, say -- is reported too, with a reason that does not
/// blame a privacy policy it has nothing to do with.
#[test]
fn a_recorded_member_missing_from_disk_is_unmaterialized_with_a_plain_reason() {
    let temp = TempDir::new("ls-absent-public");
    write_workspace_leaving_absent(
        temp.path(),
        vec![member("mem_app", "repos/app")],
        &["mem_app"],
        &["mem_app"],
    );
    let response = handle_ls(temp.path(), ls_request(&[], false), "op").unwrap();
    let app = &response.members.unwrap()[0];
    assert!(!app.materialized);
    assert_eq!(
        app.note.as_deref(),
        Some("recorded in the lock but absent on disk")
    );
}

/// A member the lock never materialized is unchanged: omitted by default,
/// listed by `--unmaterialized`, and carrying no note -- there is no
/// disagreement to report, only an unbuilt member.
#[test]
fn a_never_materialized_member_carries_no_note() {
    let temp = TempDir::new("ls-never-materialized");
    write_workspace(
        temp.path(),
        vec![
            member("mem_app", "repos/app"),
            member("mem_lib", "repos/lib"),
        ],
        &["mem_app"],
    );
    let response = handle_ls(temp.path(), ls_request(&[], true), "op").unwrap();
    let lib = response
        .members
        .unwrap()
        .into_iter()
        .find(|member| member.id == "mem_lib")
        .unwrap();
    assert!(!lib.materialized);
    assert_eq!(lib.note, None);
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
