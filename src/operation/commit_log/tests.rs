//! S2.1 requirement-row tests for default HEAD cursors.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Commit, ObjectType, Oid, Repository, Signature, Time};

use crate::artifact::{
    ArtifactSourceKind, ManifestArtifact, ManifestMember, RemoteArtifact, WORKSPACE_SCHEMA,
    WorkspaceHeader,
};

use super::*;

#[test]
fn l_sel_2_default_selection_includes_root_and_all_active_members() {
    let fixture = Fixture::new("default-selection");
    Repository::init(fixture.path().join("app")).unwrap();
    Repository::init(fixture.path().join("lib")).unwrap();
    Repository::init(fixture.path().join("old")).unwrap();
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_lib", "lib", true),
        member("mem_old", "old", false),
    ]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(
        histories
            .iter()
            .map(|history| history.target().member_id.as_str())
            .collect::<Vec<_>>(),
        ["@root", "mem_app", "mem_lib"]
    );
    assert_eq!(
        histories
            .iter()
            .map(|history| history.target().member_path.as_str())
            .collect::<Vec<_>>(),
        [".", "app", "lib"]
    );
}

#[test]
fn l_rng_2_no_operand_histories_start_at_each_repository_head() {
    let fixture = Fixture::new("head-history");
    let root_first = commit(&fixture.root, "root first", 100, &[]);
    let root_head = commit(&fixture.root, "root head", 200, &[root_first]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let member_first = commit(&member_repo, "member first", 300, &[]);
    let member_head = commit(&member_repo, "member head", 400, &[member_first]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let later_root = commit(&fixture.root, "after cursor open", 500, &[root_head]);

    assert_eq!(entry_ids(&histories[0]), [root_head, root_first]);
    assert_eq!(entry_ids(&histories[1]), [member_head, member_first]);
    assert_eq!(fixture.root.head().unwrap().target(), Some(later_root));
    let Some(CommitLogEvent::Entry(entry)) = histories[0].messages().next() else {
        panic!("root cursor did not emit a structured entry");
    };
    assert_eq!(entry.target.member_id, "@root");
    assert_eq!(entry.parent_ids, [root_first.to_string()]);
    assert_eq!(entry.message, b"root head");
    assert_eq!(entry.author.name, b"Test Author");
    assert_eq!(entry.committer.time.seconds, 200);
}

#[test]
fn entry_preserves_leading_newlines_from_the_raw_commit_message() {
    let fixture = Fixture::new("raw-leading-newline");
    let expected = b"\nleading newline is data\n";
    raw_commit(&fixture.root, expected, None);
    fixture.write_manifest(&[]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let Some(CommitLogEvent::Entry(entry)) = histories[0].messages().next() else {
        panic!("root cursor did not emit the raw commit");
    };

    assert_eq!(entry.message, expected);
}

#[test]
fn entry_preserves_non_utf8_encoding_header_bytes() {
    let fixture = Fixture::new("raw-non-utf8-encoding");
    let expected = b"ISO-8859-\xff";
    raw_commit(&fixture.root, b"encoded message\n", Some(expected));
    fixture.write_manifest(&[]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let Some(CommitLogEvent::Entry(entry)) = histories[0].messages().next() else {
        panic!("root cursor did not emit the raw commit");
    };
    assert_eq!(entry.message_encoding.as_deref(), Some(expected.as_slice()));
}

#[test]
fn l_rng_5_local_read_is_network_free_and_does_not_take_the_mutation_lock() {
    let fixture = Fixture::new("local-read");
    let head = commit(&fixture.root, "local", 100, &[]);
    fixture
        .root
        .remote("origin", "file:///definitely/absent/must-not-connect")
        .unwrap();
    fixture.write_manifest(&[]);
    let git_head_path = fixture.root.path().join("HEAD");
    let git_config_path = fixture.root.path().join("config");
    let before_head = fs::read(&git_head_path).unwrap();
    let before_config = fs::read(&git_config_path).unwrap();

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[0]), [head]);
    assert_eq!(fs::read(git_head_path).unwrap(), before_head);
    assert_eq!(fs::read(git_config_path).unwrap(), before_config);
    assert!(!fixture.root.path().join("FETCH_HEAD").exists());
    assert!(!fixture.path().join(".gwz").exists());
}

#[test]
fn l_ord_1_cursor_matches_git_log_default_order() {
    let fixture = Fixture::new("git-default-order");
    let base = commit(&fixture.root, "base", 50, &[]);
    let left_one = commit(&fixture.root, "left one", 100, &[base]);
    let left_two = commit(&fixture.root, "left two", 400, &[left_one]);
    let right = detached_commit(&fixture.root, "right", 300, &[base]);
    let merge = commit(&fixture.root, "merge", 500, &[left_two, right]);
    fixture.write_manifest(&[]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let actual = entry_ids(&histories[0]);
    let output = Command::new("git")
        .args(["-C", fixture.path().to_str().unwrap(), "log", "--format=%H"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected = String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| Oid::from_str(line).unwrap())
        .collect::<Vec<_>>();
    let topo_output = Command::new("git")
        .args([
            "-C",
            fixture.path().to_str().unwrap(),
            "log",
            "--topo-order",
            "--format=%H",
        ])
        .output()
        .unwrap();

    assert_eq!(actual, expected);
    assert_eq!(actual.first(), Some(&merge));
    assert_ne!(output.stdout, topo_output.stdout);
}

#[test]
fn l_tol_1_unreadable_member_degrades_without_stopping_other_histories() {
    let fixture = Fixture::new("degrade-one");
    let root_head = commit(&fixture.root, "root", 100, &[]);
    let good_repo = Repository::init(fixture.path().join("good")).unwrap();
    let good_head = commit(&good_repo, "good", 200, &[]);
    fs::create_dir(fixture.path().join("bad")).unwrap();
    let mut generated = member("mem_generated", "generated", true);
    generated.source_kind = ArtifactSourceKind::Generated;
    fixture.write_manifest(&[
        member("mem_good", "good", true),
        member("mem_bad", "bad", true),
        generated,
    ]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[0]), [root_head]);
    assert_eq!(entry_ids(&histories[1]), [good_head]);
    assert_eq!(
        degradations(&histories[2])
            .iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>(),
        [CommitLogDegradationKind::RepositoryUnreadable]
    );
    assert_eq!(
        degradations(&histories[3])[0].kind,
        CommitLogDegradationKind::UnsupportedSourceKind
    );
}

#[test]
fn l_tol_3_unborn_repository_contributes_no_entries_and_a_degradation() {
    let fixture = Fixture::new("unborn");
    commit(&fixture.root, "root", 100, &[]);
    Repository::init(fixture.path().join("empty")).unwrap();
    fixture.write_manifest(&[member("mem_empty", "empty", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let events = histories[1].messages().collect::<Vec<_>>();

    assert!(
        events
            .iter()
            .all(|event| !matches!(event, CommitLogEvent::Entry(_)))
    );
    assert!(matches!(
        events.as_slice(),
        [CommitLogEvent::Degradation(CommitLogDegradation {
            kind: CommitLogDegradationKind::UnbornHead,
            ..
        })]
    ));
}

#[test]
fn l_tol_4_detached_member_logs_normally_from_the_detached_commit() {
    let fixture = Fixture::new("detached");
    commit(&fixture.root, "root", 100, &[]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let detached = commit(&member_repo, "detached target", 200, &[]);
    commit(&member_repo, "branch head", 300, &[detached]);
    member_repo.set_head_detached(detached).unwrap();
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[1]), [detached]);
    assert!(degradations(&histories[1]).is_empty());
}

#[test]
fn l_tol_5_shallow_member_contributes_every_locally_available_commit() {
    let fixture = Fixture::new("shallow");
    commit(&fixture.root, "root", 100, &[]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let first = commit(&member_repo, "first", 200, &[]);
    let head = commit(&member_repo, "head", 300, &[first]);
    fs::write(member_repo.path().join("shallow"), format!("{head}\n")).unwrap();
    drop(member_repo);
    assert!(
        Repository::open(fixture.path().join("app"))
            .unwrap()
            .is_shallow()
    );
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[1]), [head]);
    assert!(degradations(&histories[1]).is_empty());
}

#[test]
fn l_tol_6_conf_integrity_mismatch_does_not_gate_history_reads() {
    let fixture = Fixture::new("no-conf-gate");
    let head = commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[]);
    crate::artifact::refresh_conf_integrity_marker(fixture.path()).unwrap();
    let manifest_path = fixture.path().join(crate::workspace::WORKSPACE_MANIFEST);
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    fs::write(&manifest_path, format!("{manifest}\n")).unwrap();
    assert!(crate::artifact::inspect_conf_integrity(fixture.path()).refuses());

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[0]), [head]);
}

fn entry_ids(history: &RepositoryHistory) -> Vec<Oid> {
    history
        .messages()
        .filter_map(|event| match event {
            CommitLogEvent::Entry(entry) => Some(Oid::from_str(&entry.commit_id).unwrap()),
            CommitLogEvent::Degradation(_) => None,
        })
        .collect()
}

fn degradations(history: &RepositoryHistory) -> Vec<CommitLogDegradation> {
    history
        .messages()
        .filter_map(|event| match event {
            CommitLogEvent::Entry(_) => None,
            CommitLogEvent::Degradation(record) => Some(record),
        })
        .collect()
}

fn member(id: &str, path: &str, active: bool) -> ManifestMember {
    ManifestMember {
        id: id.to_owned(),
        path: path.to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: format!("src_{}", id.trim_start_matches("mem_")),
        active,
        desired: None,
        remotes: Vec::<RemoteArtifact>::new(),
    }
}

fn commit(repo: &Repository, message: &str, seconds: i64, parents: &[Oid]) -> Oid {
    let oid = detached_commit(repo, message, seconds, parents);
    match repo.head() {
        Ok(head) if head.is_branch() => {
            repo.reference(head.name().unwrap(), oid, true, message)
                .unwrap();
        }
        _ => {
            repo.reference("refs/heads/main", oid, true, message)
                .unwrap();
            repo.set_head("refs/heads/main").unwrap();
        }
    }
    oid
}

fn detached_commit(repo: &Repository, message: &str, seconds: i64, parents: &[Oid]) -> Oid {
    let signature =
        Signature::new("Test Author", "test@example.com", &Time::new(seconds, 0)).unwrap();
    let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parents = parents
        .iter()
        .map(|oid| repo.find_commit(*oid).unwrap())
        .collect::<Vec<Commit<'_>>>();
    let parent_refs = parents.iter().collect::<Vec<_>>();
    repo.commit(None, &signature, &signature, message, &tree, &parent_refs)
        .unwrap()
}

fn raw_commit(repo: &Repository, message: &[u8], encoding: Option<&[u8]>) -> Oid {
    let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
    let mut raw = format!(
        "tree {tree_id}\nauthor Test Author <test@example.com> 100 +0000\ncommitter Test Author <test@example.com> 100 +0000\n"
    )
    .into_bytes();
    if let Some(encoding) = encoding {
        raw.extend_from_slice(b"encoding ");
        raw.extend_from_slice(encoding);
        raw.push(b'\n');
    }
    raw.push(b'\n');
    raw.extend_from_slice(message);

    let oid = repo.odb().unwrap().write(ObjectType::Commit, &raw).unwrap();
    repo.reference("refs/heads/main", oid, true, "raw test commit")
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    oid
}

struct Fixture {
    path: PathBuf,
    root: Repository,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gwz-core-commit-log-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        let root = Repository::init(&path).unwrap();
        Self { path, root }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_manifest(&self, members: &[ManifestMember]) {
        let manifest = ManifestArtifact {
            schema: WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: "ws_test".to_owned(),
            },
            members: members.to_vec(),
        };
        fs::create_dir_all(self.path.join(crate::workspace::WORKSPACE_DIR)).unwrap();
        fs::write(
            self.path.join(crate::workspace::WORKSPACE_MANIFEST),
            manifest.to_yaml().unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
