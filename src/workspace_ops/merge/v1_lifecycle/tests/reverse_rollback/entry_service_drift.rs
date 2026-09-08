use super::entry_service::{
    assert_entry_rejected_without_mutation, seed_open, service_fixture,
    service_fixture_with_later_member,
};
use super::*;

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
    let merge_head = &fixture.model.participants["@root"].source_commit;
    fixture
        .backend
        .test_set_repository_state(
            &fixture.root.path,
            crate::git::GitRepositoryState::Merge,
            Some(merge_head),
        )
        .unwrap();
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
    let mut entries = fixture.backend.test_read_index(&member).unwrap();
    entries
        .iter_mut()
        .find(|entry| entry.path == b"README.md")
        .unwrap()
        .assume_valid = true;
    fixture
        .backend
        .test_replace_index(&member, &entries)
        .unwrap();
    write_for_test(&member.join("README.md"), b"hidden later-member drift\n").unwrap();
    seed_open(&fixture);
    assert_entry_rejected_without_mutation(&fixture, "later member semantic drift", "mem_z");
}

fn install_root_drift(fixture: &super::entry_service::ServiceFixture, case: &str) {
    let root = &fixture.root.path;
    match case {
        "staged" => {
            write_for_test(&root.join("staged-drift.txt"), b"staged\n").unwrap();
            fixture
                .backend
                .stage_paths(root, &["staged-drift.txt"])
                .unwrap();
        }
        "unstaged" => {
            write_for_test(&root.join("selected-root.txt"), b"unstaged drift\n").unwrap();
        }
        "untracked" => {
            write_for_test(&root.join("untracked-drift.txt"), b"untracked\n").unwrap();
        }
        "rename" => {
            make_filesystem()
                .rename(
                    &root.join("selected-root.txt"),
                    &root.join("renamed-root.txt"),
                    crate::filesystem::RenameMode::NoReplace,
                )
                .unwrap();
        }
        "type-change" => {
            make_filesystem()
                .remove_file(&root.join("selected-root.txt"))
                .unwrap();
            make_filesystem()
                .create_directories(&root.join("selected-root.txt"))
                .unwrap();
            write_for_test(&root.join("selected-root.txt/child"), b"type change\n").unwrap();
        }
        "unresolved" => install_unresolved_index(root),
        _ => unreachable!(),
    }
}

fn install_unresolved_index(root: &std::path::Path) {
    let backend = make_repository();
    let entry = backend
        .test_read_index(root)
        .unwrap()
        .into_iter()
        .find(|entry| entry.path == b"selected-root.txt")
        .unwrap();
    let entries = [1, 2, 3]
        .map(|stage| {
            let mut entry = entry.clone();
            entry.stage = stage;
            entry
        })
        .to_vec();
    backend.test_replace_index(root, &entries).unwrap();
}
