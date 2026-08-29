//! S2.4 requirement-row tests for stateless commit-log group assembly.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Commit, Repository, Signature, Time};

use crate::git::{Git2Backend, GitBackend};
use crate::operation::NullSink;
use crate::workspace_ops::{handle_commit, handle_init_from_sources};

use super::coalesce::{
    COALESCING_WINDOW_SECONDS, CommitLogGroup, CommitLogProvenance, assemble_commit_log_groups,
};
use super::*;

const MARKER_A: &str = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91";
const MARKER_B: &str = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c92";

#[test]
fn l_coa_1_real_trailer_siblings_group_across_repositories() {
    let message = marker_message(b"Ship the workspace change", MARKER_A);
    let groups = assemble_commit_log_groups(
        vec![
            entry("@root", "root-sha", &message, "Author", 100, 104),
            entry("mem_core", "core-sha", &message, "Author", 100, 101),
        ],
        true,
    );

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries().len(), 2);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
}

#[test]
fn l_coa_1_shipped_marker_groups_entries_read_from_git_histories() {
    let fixture = CommitWorkspaceFixture::new("shipped-marker-history");
    let backend = Git2Backend::new();
    fixture.initialize(&backend);

    let manifest = crate::artifact::read_manifest(fixture.workspace()).unwrap();
    let member_path = fixture.workspace().join(&manifest.members[0].path);
    set_repository_identity(fixture.workspace());
    set_repository_identity(&member_path);
    backend
        .add_remote(fixture.workspace(), "origin", fixture.remote_url())
        .unwrap();
    fs::write(member_path.join("work.txt"), "marker integration\n").unwrap();
    backend.stage_paths(&member_path, &["work.txt"]).unwrap();

    handle_commit(
        &backend,
        fixture.workspace(),
        crate::CommitRequest {
            meta: request_meta(),
            message: "original member body\n\nuser text\n---\nstill user text".to_owned(),
            all: None,
            commit_marker: None,
        },
        "op_coalesce_integration",
    )
    .unwrap();

    let marker = crate::artifact::list_markers(fixture.workspace())
        .unwrap()
        .pop()
        .unwrap();
    let root_repository = Repository::open(fixture.workspace()).unwrap();
    let root_commit = root_repository.head().unwrap().peel_to_commit().unwrap();
    let root_seconds = root_commit.committer().when().seconds();
    let member_repository = Repository::open(&member_path).unwrap();
    let shipped_member = member_repository.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(
        root_commit.message_raw_bytes(),
        shipped_member.message_raw_bytes(),
        "the starting siblings came from handle_commit's one message"
    );
    rewrite_head_preserving_shipped_trailer(
        &member_repository,
        &shipped_member,
        root_seconds + COALESCING_WINDOW_SECONDS,
    );
    drop(shipped_member);
    drop(member_repository);
    drop(root_commit);
    drop(root_repository);

    let entries = open_default_head_histories(fixture.workspace())
        .unwrap()
        .iter()
        .map(|history| match history.messages().next().unwrap() {
            CommitLogEvent::Entry(entry) => entry,
            CommitLogEvent::Degradation(record) => {
                panic!("{} degraded: {}", record.target.member_id, record.detail)
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].message, entries[1].message);
    assert_ne!(entries[0].author.name, entries[1].author.name);
    assert_eq!(
        entries[0]
            .committer
            .time
            .seconds
            .abs_diff(entries[1].committer.time.seconds),
        COALESCING_WINDOW_SECONDS as u64
    );

    let groups = assemble_commit_log_groups(entries, true);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries().len(), 2);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(marker.gwz_commit_id)
    );
}

#[test]
fn l_coa_1_terminal_shipped_trailer_survives_patch_separator_in_user_text() {
    let message = marker_message(b"Subject\n\nuser text\n---\nstill user text", MARKER_A);
    let groups = assemble_commit_log_groups(
        vec![
            entry("mem_a", "a", &message, "First", 100, 100),
            entry("mem_b", "b", &message, "Second", 160, 160),
        ],
        true,
    );

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries().len(), 2);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
}

#[test]
fn l_coa_1_only_one_canonical_lowercase_uuid_v7_is_authoritative() {
    let mut non_utf8 = b"body\n\nGWZ-Commit-ID: ".to_vec();
    non_utf8.extend_from_slice(b"01987b0c-2f75-7c4a-9a32-8fd22f7d7c\xff");
    non_utf8.extend_from_slice(b"\nGWZ-Workspace-ID: ws_test");
    let cases = [
        ("arbitrary", marker_message(b"body", "not-a-uuid")),
        (
            "uppercase",
            marker_message(b"body", "01987B0C-2F75-7C4A-9A32-8FD22F7D7C91"),
        ),
        (
            "wrong version",
            marker_message(b"body", "01987b0c-2f75-4c4a-9a32-8fd22f7d7c91"),
        ),
        (
            "wrong hyphen positions",
            marker_message(b"body", "01987b0c2-f75-7c4a-9a32-8fd22f7d7c91"),
        ),
        ("non-UTF-8", non_utf8),
        (
            "empty",
            b"body\n\nGWZ-Commit-ID: \nGWZ-Workspace-ID: ws_test".to_vec(),
        ),
        (
            "continued",
            format!(
                "body\n\nGWZ-Commit-ID: {MARKER_A}\n continuation\nGWZ-Workspace-ID: ws_test"
            )
            .into_bytes(),
        ),
        (
            "blank continuation",
            format!("body\n\nGWZ-Commit-ID: {MARKER_A}\n \nGWZ-Workspace-ID: ws_test")
                .into_bytes(),
        ),
        (
            "duplicate identical",
            format!(
                "body\n\nGWZ-Commit-ID: {MARKER_A}\nGWZ-Commit-ID: {MARKER_A}\nGWZ-Workspace-ID: ws_test"
            )
            .into_bytes(),
        ),
        (
            "duplicate conflicting",
            format!(
                "body\n\nGWZ-Commit-ID: {MARKER_A}\nGWZ-Commit-ID: {MARKER_B}\nGWZ-Workspace-ID: ws_test"
            )
            .into_bytes(),
        ),
    ];

    for (name, message) in cases {
        assert_invalid_marker_singletons(name, message);
    }
}

#[test]
fn l_coa_1_wrong_variant_uuid_v7_looking_claim_is_marker_invalid() {
    let groups = assemble_commit_log_groups(
        [entry(
            "mem_a",
            "a",
            &marker_message(b"body", "01987b0c-2f75-7c4a-1a32-8fd22f7d7c91"),
            "Author",
            100,
            100,
        )],
        true,
    );

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].provenance(), &CommitLogProvenance::MarkerInvalid);
}

#[test]
fn l_coa_9_mangled_separator_claim_is_marker_invalid() {
    let message = format!("body\n\nGWZ-Commit-ID={MARKER_A}\nGWZ-Workspace-ID: ws_test");
    let groups = assemble_commit_log_groups(
        [entry("mem_a", "a", message.as_bytes(), "Author", 100, 100)],
        true,
    );

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].provenance(), &CommitLogProvenance::MarkerInvalid);
}

#[test]
fn l_coa_1_valid_uuid_v7_marker_happy_path_is_unchanged() {
    let message = marker_message(b"valid", MARKER_A);
    let groups = assemble_commit_log_groups(
        vec![
            entry("mem_a", "a", &message, "First", 100, 100),
            entry("mem_b", "b", &message, "Second", 160, 160),
        ],
        true,
    );

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries().len(), 2);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
}

#[test]
fn l_coa_9_identical_invalid_markers_never_heuristic_coalesce() {
    let message = format!("body\n\nGWZ-Commit-ID={MARKER_A}\nGWZ-Workspace-ID: ws_test");
    let groups = assemble_commit_log_groups(
        vec![
            entry("mem_a", "a", message.as_bytes(), "Author", 100, 100),
            entry("mem_b", "b", message.as_bytes(), "Author", 100, 100),
        ],
        true,
    );

    assert_eq!(groups.len(), 2);
    assert!(
        groups
            .iter()
            .all(|group| group.provenance() == &CommitLogProvenance::MarkerInvalid)
    );
}

#[test]
fn l_coa_2_marker_shaped_malformed_claims_never_enter_heuristic() {
    let cases = [
        (
            "missing colon",
            format!("body\n\nGWZ-Commit-ID {MARKER_A}\nGWZ-Workspace-ID: ws_test")
                .into_bytes(),
        ),
        (
            "malformed adjacent line",
            format!(
                "body\n\nGWZ-Commit-ID: {MARKER_A}\nGWZ-Workspace-ID: ws_test\nnot-a-production-trailer"
            )
            .into_bytes(),
        ),
        ("invalid UUID", marker_message(b"body", "invalid")),
    ];

    for (name, message) in cases {
        assert_invalid_marker_singletons(name, message);
    }
}

#[test]
fn l_coa_2_must_merge_same_message_forall_fan_out() {
    let message = b"Apply the same mechanical edit\n\nDetails stay byte-identical.";
    let groups = assemble_commit_log_groups(
        vec![
            entry("mem_a", "a", message, "Author", 1_000, 2_000),
            entry("mem_b", "b", message, "Author", 1_006, 2_008),
            entry("mem_c", "c", message, "Author", 1_009, 2_010),
        ],
        true,
    );

    assert_eq!(groups.len(), 1, "the standing Q-1 rule admits fan-outs");
    assert_eq!(groups[0].entries().len(), 3);
    assert_eq!(groups[0].provenance(), &CommitLogProvenance::Heuristic);
}

#[test]
fn l_coa_2_must_not_merge_same_message_with_different_author() {
    let mut second = entry("mem_b", "b", b"same", "Other", 100, 100);
    second.author.email = b"other@example.invalid".to_vec();

    assert_singletons(vec![
        entry("mem_a", "a", b"same", "Author", 100, 100),
        second,
    ]);
}

#[test]
fn l_coa_2_must_not_merge_same_author_name_with_different_email() {
    let mut second = entry("mem_b", "b", b"same", "Author", 100, 100);
    second.author.email = b"different@example.invalid".to_vec();

    assert_singletons(vec![
        entry("mem_a", "a", b"same", "Author", 100, 100),
        second,
    ]);
}

#[test]
fn l_coa_2_must_not_merge_outside_committer_window() {
    assert_singletons(vec![
        entry("mem_a", "a", b"same", "Author", 100, 200),
        entry("mem_b", "b", b"same", "Author", 100, 211),
    ]);
}

#[test]
fn l_coa_2_must_not_merge_outside_author_window() {
    assert_singletons(vec![
        entry("mem_a", "a", b"same", "Author", 100, 200),
        entry("mem_b", "b", b"same", "Author", 111, 200),
    ]);
}

#[test]
fn l_coa_2_must_not_merge_distinct_markers_with_identical_messages() {
    assert_singletons(vec![
        entry(
            "mem_a",
            "a",
            &marker_message(b"same", MARKER_A),
            "Author",
            100,
            100,
        ),
        entry(
            "mem_b",
            "b",
            &marker_message(b"same", MARKER_B),
            "Author",
            100,
            100,
        ),
    ]);
}

#[test]
fn l_coa_2_must_not_merge_marked_commit_with_matching_unmarked_commit() {
    assert_singletons(vec![
        entry(
            "mem_a",
            "a",
            &marker_message(b"same", MARKER_A),
            "Author",
            100,
            100,
        ),
        entry("mem_b", "b", b"same", "Author", 100, 100),
    ]);
}

#[test]
fn l_coa_2_must_not_merge_same_repository_twins() {
    assert_singletons(vec![
        entry("mem_a", "a1", b"same", "Author", 100, 100),
        entry("mem_a", "a2", b"same", "Author", 100, 100),
    ]);
}

#[test]
fn l_coa_2_must_not_merge_rebase_restamps_with_old_author_dates() {
    assert_singletons(vec![
        entry("mem_a", "a", b"fmt", "Author", 100, 10_000),
        entry("mem_b", "b", b"fmt", "Author", 10_000, 10_001),
    ]);
}

#[test]
fn l_coa_2_uses_raw_message_and_author_identity_bytes() {
    let raw = b"non-utf8: \xff\n";
    let first = entry("mem_a", "a", raw, "Author", 100, 100);
    let second = entry("mem_b", "b", raw, "Author", 100, 100);
    let different_message = entry("mem_c", "c", b"non-utf8: \xfe\n", "Author", 100, 100);
    let mut different_author = entry("mem_d", "d", raw, "Author", 100, 100);
    different_author.author.name = b"Author\xff".to_vec();

    let groups = assemble_commit_log_groups(
        vec![first.clone(), second, different_message, different_author],
        true,
    );

    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].entries().len(), 2);
    assert_eq!(groups[0].provenance(), &CommitLogProvenance::Heuristic);
    assert!(
        groups[0]
            .entries()
            .iter()
            .all(|entry| entry.message == first.message)
    );
}

#[test]
fn l_coa_3_no_coalesce_yields_singleton_groups() {
    let message = marker_message(b"same", MARKER_A);
    let groups = assemble_commit_log_groups(
        vec![
            entry("mem_a", "a", &message, "Author", 100, 100),
            entry("mem_b", "b", &message, "Author", 100, 100),
            entry(
                "mem_invalid",
                "invalid",
                &marker_message(b"invalid", "not-a-uuid"),
                "Author",
                100,
                100,
            ),
        ],
        false,
    );

    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group.entries().len() == 1));
    assert!(
        groups[..2]
            .iter()
            .all(|group| group.provenance() == &CommitLogProvenance::None)
    );
    assert_eq!(groups[2].provenance(), &CommitLogProvenance::MarkerInvalid);
}

#[test]
fn group_assembly_window_fragment_keeps_marker_provenance_key() {
    let groups = assemble_commit_log_groups(
        [entry(
            "mem_a",
            "a",
            &marker_message(b"marked", MARKER_A),
            "Author",
            100,
            100,
        )],
        true,
    );

    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
}

#[test]
fn marker_authority_groups_heuristic_ineligible_siblings_at_window_boundary() {
    let groups = assemble_commit_log_groups(
        vec![
            entry(
                "mem_a",
                "a",
                &marker_message(b"first body", MARKER_A),
                "First",
                100,
                100,
            ),
            entry(
                "mem_b",
                "b",
                &marker_message(b"different body", MARKER_A),
                "Second",
                160,
                160,
            ),
        ],
        true,
    );

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries().len(), 2);
    assert_eq!(groups[0].ordering_timestamp_seconds(), 160);
    assert_eq!(COALESCING_WINDOW_SECONDS, 60);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
}

#[test]
fn l_coa_4_and_6_latest_timestamp_and_all_provenance_values() {
    let groups = assemble_commit_log_groups(
        vec![
            entry(
                "mem_marker_a",
                "ma",
                &marker_message(b"marked", MARKER_A),
                "Author",
                100,
                101,
            ),
            entry(
                "mem_marker_b",
                "mb",
                &marker_message(b"marked", MARKER_A),
                "Author",
                100,
                109,
            ),
            entry("mem_heur_a", "ha", b"heuristic", "Author", 200, 205),
            entry("mem_heur_b", "hb", b"heuristic", "Author", 208, 210),
            entry("mem_single", "s", b"singleton", "Author", 300, 301),
            entry(
                "mem_invalid",
                "i",
                &marker_message(b"invalid", "not-a-uuid"),
                "Author",
                400,
                401,
            ),
        ],
        true,
    );

    let marker = group_with(&groups, &CommitLogProvenance::Marker(MARKER_A.to_owned()));
    assert_eq!(marker.ordering_timestamp_seconds(), 109);
    let heuristic = group_with(&groups, &CommitLogProvenance::Heuristic);
    assert_eq!(heuristic.ordering_timestamp_seconds(), 210);
    let singleton = group_with(&groups, &CommitLogProvenance::None);
    assert_eq!(singleton.ordering_timestamp_seconds(), 301);
    let invalid = group_with(&groups, &CommitLogProvenance::MarkerInvalid);
    assert_eq!(invalid.ordering_timestamp_seconds(), 401);
    assert_eq!(COALESCING_WINDOW_SECONDS, 60);
}

fn assert_singletons(entries: Vec<CommitLogEntry>) {
    let groups = assemble_commit_log_groups(entries, true);
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|group| group.entries().len() == 1));
    assert!(
        !groups
            .iter()
            .any(|group| group.provenance() == &CommitLogProvenance::Heuristic)
    );
}

fn assert_invalid_marker_singletons(name: &str, message: Vec<u8>) {
    let groups = assemble_commit_log_groups(
        vec![
            entry("mem_a", "a", &message, "Author", 100, 100),
            entry("mem_b", "b", &message, "Author", 100, 100),
        ],
        true,
    );
    assert_eq!(groups.len(), 2, "{name}");
    assert!(
        groups
            .iter()
            .all(|group| group.provenance() == &CommitLogProvenance::MarkerInvalid),
        "{name}"
    );
}

fn group_with<'a>(
    groups: &'a [CommitLogGroup],
    provenance: &CommitLogProvenance,
) -> &'a CommitLogGroup {
    groups
        .iter()
        .find(|group| group.provenance() == provenance)
        .unwrap()
}

fn rewrite_head_preserving_shipped_trailer(
    repository: &Repository,
    shipped: &Commit<'_>,
    seconds: i64,
) {
    let message = shipped.message_raw_bytes();
    let boundary = message
        .windows(2)
        .rposition(|bytes| bytes == b"\n\n")
        .unwrap();
    let mut rewritten = b"different body deliberately defeats byte identity".to_vec();
    rewritten.extend_from_slice(&message[boundary..]);
    let rewritten = std::str::from_utf8(&rewritten).unwrap();
    let author = Signature::new(
        "Different Author",
        "different-author@example.invalid",
        &Time::new(seconds, 0),
    )
    .unwrap();
    let committer = Signature::new(
        "Different Committer",
        "different-committer@example.invalid",
        &Time::new(seconds, 0),
    )
    .unwrap();
    let tree = shipped.tree().unwrap();
    let parents = shipped.parents().collect::<Vec<Commit<'_>>>();
    let parent_refs = parents.iter().collect::<Vec<_>>();
    let rewritten_id = repository
        .commit(None, &author, &committer, rewritten, &tree, &parent_refs)
        .unwrap();
    repository
        .find_reference(repository.head().unwrap().name().unwrap())
        .unwrap()
        .set_target(rewritten_id, "rewrite test fixture identity and time")
        .unwrap();
}

fn set_repository_identity(path: &Path) {
    let repository = Repository::open(path).unwrap();
    let mut config = repository.config().unwrap();
    config.set_str("user.name", "GWZ Coalesce Test").unwrap();
    config
        .set_str("user.email", "gwz-coalesce@example.invalid")
        .unwrap();
}

fn request_meta() -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: "req_coalesce".to_owned(),
        schema_version: "gwz.protocol/v0".to_owned(),
        ..Default::default()
    }
}

struct CommitWorkspaceFixture {
    base: PathBuf,
    workspace: PathBuf,
    remote: PathBuf,
}

impl CommitWorkspaceFixture {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "gwz-core-coalesce-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&base).unwrap();
        Self {
            workspace: base.join("workspace"),
            remote: base.join("remote.git"),
            base,
        }
    }

    fn workspace(&self) -> &Path {
        &self.workspace
    }

    fn remote_url(&self) -> &str {
        self.remote.to_str().unwrap()
    }

    fn initialize(&self, backend: &Git2Backend) {
        let source = self.base.join("source");
        backend.create_repo(&source).unwrap();
        set_repository_identity(&source);
        fs::write(source.join("README.md"), "initial\n").unwrap();
        backend.stage_paths(&source, &["README.md"]).unwrap();
        backend.commit(&source, "initial", false).unwrap();
        let bare = Repository::init_bare(&self.remote).unwrap();
        bare.set_head("refs/heads/main").unwrap();
        backend
            .add_remote(&source, "origin", self.remote.to_str().unwrap())
            .unwrap();
        backend
            .push(&source, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();

        handle_init_from_sources(
            backend,
            &self.base,
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: self.workspace.to_string_lossy().into_owned(),
                sources: vec![crate::SourceUrl {
                    url: self.remote.to_string_lossy().into_owned(),
                    path: None,
                    remote_name: None,
                    branch: None,
                }],
                target: None,
                workspace_id: Some("ws_coalesce".to_owned()),
            },
            "op_coalesce_init",
            &NullSink,
        )
        .unwrap();
    }
}

impl Drop for CommitWorkspaceFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn marker_message(message: &[u8], marker: &str) -> Vec<u8> {
    let mut marked = message.to_vec();
    marked.extend_from_slice(b"\n\nGWZ-Commit-ID: ");
    marked.extend_from_slice(marker.as_bytes());
    marked.extend_from_slice(b"\nGWZ-Workspace-ID: ws_test");
    marked
}

fn entry(
    member_id: &str,
    commit_id: &str,
    message: &[u8],
    author_name: &str,
    author_seconds: i64,
    committer_seconds: i64,
) -> CommitLogEntry {
    CommitLogEntry {
        target: CommitLogTarget {
            member_id: member_id.to_owned(),
            member_path: member_id.trim_start_matches("mem_").to_owned(),
        },
        commit_id: commit_id.to_owned(),
        parent_ids: Vec::new(),
        author: CommitLogIdentity {
            name: author_name.as_bytes().to_vec(),
            email: b"author@example.invalid".to_vec(),
            time: CommitLogTime {
                seconds: author_seconds,
                offset_minutes: 600,
            },
        },
        committer: CommitLogIdentity {
            name: b"Committer".to_vec(),
            email: b"committer@example.invalid".to_vec(),
            time: CommitLogTime {
                seconds: committer_seconds,
                offset_minutes: 600,
            },
        },
        message: message.to_vec(),
        message_encoding: None,
    }
}
