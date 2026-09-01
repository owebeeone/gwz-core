use super::entry_service::{
    assert_entry_rejected_without_mutation, seed_open, service_fixture,
    service_fixture_with_later_member,
};
use super::*;
use std::io::Write;

// [2026-09-02, R2-E E4.4-6-B: [P3-8] closes (nothing converts, no snapshot exclusion
// grows) for every row here too — see `entry_service.rs::collect_files`.]

#[test]
fn selected_root_service_entry_rejects_complete_checkout_drift_without_mutation() {
    for case in [
        "staged",
        "unstaged",
        "untracked",
        "rename",
        "type-change",
        "unresolved",
    ] {
        let fixture = service_fixture(&format!("v1-rollback-service-root-{case}"));
        install_root_drift(&fixture, case);
        seed_open(&fixture);
        assert_entry_rejected_without_mutation(&fixture, case, "@root");
    }
}

#[test]
fn selected_root_service_entry_rejects_native_state_without_mutation() {
    let fixture = service_fixture("v1-rollback-service-root-native-state");
    let merge_head = fixture.model.participants["@root"].source_commit.as_bytes();
    let mut bytes = merge_head.to_vec();
    bytes.push(b'\n');
    std::fs::write(fixture.root.path.join(".git/MERGE_HEAD"), bytes).unwrap();
    assert_ne!(
        fixture
            .backend
            .repository_state(&fixture.root.path)
            .unwrap(),
        crate::git::GitRepositoryState::Clean
    );
    seed_open(&fixture);
    assert_entry_rejected_without_mutation(&fixture, "native state", "@root");
}

#[test]
fn later_member_semantic_drift_rejects_before_selected_root_mutation() {
    let fixture =
        service_fixture_with_later_member("v1-rollback-service-later-member-semantic-index");
    let member = fixture.root.path.join("members/z");
    git(
        &member,
        &["update-index", "--assume-unchanged", "README.md"],
    );
    std::fs::write(member.join("README.md"), "hidden later-member drift\n").unwrap();
    seed_open(&fixture);
    assert_entry_rejected_without_mutation(&fixture, "later member semantic drift", "mem_z");
}

fn install_root_drift(fixture: &super::entry_service::ServiceFixture, case: &str) {
    let root = &fixture.root.path;
    match case {
        "staged" => {
            std::fs::write(root.join("staged-drift.txt"), "staged\n").unwrap();
            fixture
                .backend
                .stage_paths(root, &["staged-drift.txt"])
                .unwrap();
        }
        "unstaged" => {
            std::fs::write(root.join("selected-root.txt"), "unstaged drift\n").unwrap();
        }
        "untracked" => {
            std::fs::write(root.join("untracked-drift.txt"), "untracked\n").unwrap();
        }
        "rename" => {
            git(root, &["mv", "selected-root.txt", "renamed-root.txt"]);
        }
        "type-change" => {
            std::fs::remove_file(root.join("selected-root.txt")).unwrap();
            std::fs::create_dir(root.join("selected-root.txt")).unwrap();
            std::fs::write(root.join("selected-root.txt/child"), "type change\n").unwrap();
        }
        "unresolved" => install_unresolved_index(root),
        _ => unreachable!(),
    }
}

fn install_unresolved_index(root: &std::path::Path) {
    let repo = git2::Repository::open(root).unwrap();
    let base = repo.blob(b"base\n").unwrap();
    let ours = repo.blob(b"ours\n").unwrap();
    let theirs = repo.blob(b"theirs\n").unwrap();
    let zero = "0".repeat(40);
    let input = format!(
        "0 {zero}\tselected-root.txt\n\
         100644 {base} 1\tselected-root.txt\n\
         100644 {ours} 2\tselected-root.txt\n\
         100644 {theirs} 3\tselected-root.txt\n"
    );
    let mut child = std::process::Command::new("git")
        .args(["update-index", "--index-info"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(
        git2::Repository::open(root)
            .unwrap()
            .index()
            .unwrap()
            .has_conflicts()
    );
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap()
            .success(),
        "git {args:?} failed"
    );
}
