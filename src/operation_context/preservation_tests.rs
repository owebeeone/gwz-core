use super::*;
use crate::git::*;

const MARKER: &str = "gwz.conf/markers/merge_context.yaml";
const LOCK: &str = crate::artifact::LOCK_PATH;

fn write(context: &OperationContext, root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    context
        .filesystem()
        .create_directories(path.parent().unwrap())
        .unwrap();
    if context.filesystem().kind(&path).is_ok() {
        context.filesystem().remove_file(&path).unwrap();
    }
    let file = context.filesystem().create_file(&path).unwrap();
    context.filesystem().write_all(&file, bytes).unwrap();
}

fn form(context: &OperationContext, root: &Path) -> GitRootManagedForm {
    let index = context.repository().repository_index(root).unwrap();
    let fact = |path: &str| {
        index
            .entries
            .iter()
            .find(|entry| entry.path == path.as_bytes())
            .map_or_else(
                || GitRootManagedIndexFact::Absent {
                    path: path.as_bytes().to_vec(),
                },
                |entry| {
                    GitRootManagedIndexFact::Present(GitRootManagedIndexEntry {
                        path: entry.path.clone(),
                        object_id: entry
                            .object_id
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect(),
                        mode: entry.mode,
                        stage: 0,
                        assume_valid: false,
                        skip_worktree: false,
                        intent_to_add: false,
                    })
                },
            )
    };
    let candidate = |path: &str| {
        context
            .filesystem()
            .read(&root.join(path))
            .ok()
            .map(|bytes| GitCandidateFile {
                path: path.into(),
                bytes,
            })
    };
    GitRootManagedForm {
        marker: candidate(MARKER),
        lock: candidate(LOCK).unwrap(),
        index: GitRootManagedIndexForm {
            marker: fact(MARKER),
            lock: fact(LOCK),
        },
    }
}

#[test]
fn context_root_preservation_reopens_between_physical_steps() {
    let world = TestWorld::selected();
    let context = world.context();
    let workspace = context.filesystem().test_workspace().unwrap();
    let root = workspace.path();
    context
        .repository()
        .test_init_repo(root, &TestRepoSpec::default())
        .unwrap();
    write(&context, root, LOCK, b"restore lock\n");
    context.repository().stage_paths(root, &[LOCK]).unwrap();
    let restore_commit = context
        .repository()
        .commit(root, "restore", false)
        .unwrap()
        .commit;
    let restore_clean_form = form(&context, root);
    write(&context, root, LOCK, b"attached lock\n");
    context.repository().stage_paths(root, &[LOCK]).unwrap();
    let attached_commit = context
        .repository()
        .commit(root, "attached", false)
        .unwrap()
        .commit;
    let attached_clean_form = form(&context, root);
    write(&context, root, MARKER, b"handoff marker\n");
    write(&context, root, LOCK, b"handoff lock\n");
    context
        .repository()
        .stage_paths(root, &[LOCK, MARKER])
        .unwrap();
    let handoff_form = form(&context, root);
    let boundary = b"/.gwz/\n";
    write(&context, root, ".git/info/exclude", boundary);
    write(&context, root, "user-work.txt", b"preserve me\n");
    let spec = GitRootPreservationSpec {
        attached_branch: "main".into(),
        attached_commit,
        restore_commit,
        managed_marker_path: MARKER.into(),
        attached_clean_form,
        restore_clean_form,
        handoff_form,
        handoff_boundary: boundary.to_vec(),
        excluded_worktree_paths: vec![],
    };
    let prepared = context
        .repository()
        .prepare_root_preservation_stash(root, &spec)
        .unwrap();
    let guard = GitRootPreservationGuard::NormalizedPreimage {
        sha256: prepared.normalized_image.preimage_sha256.clone(),
    };
    drop(context);
    for object in [
        GitRootManagedObject::MarkerParentDirectory,
        GitRootManagedObject::MarkerWorktree,
        GitRootManagedObject::LockWorktree,
        GitRootManagedObject::Index,
    ] {
        let reopened = world.context();
        let step = GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
            object,
            source: GitRootManagedFormName::Handoff,
            goal: GitRootManagedFormName::AttachedClean,
        });
        if object == GitRootManagedObject::LockWorktree {
            crate::checked_artifact::fail_next_checked_artifact_at(
                crate::checked_artifact::CheckedArtifactFault::AfterSourceRetirement,
            );
            assert!(
                reopened
                    .repository()
                    .execute_root_preservation_step_checked(root, &spec, &step, &guard)
                    .is_err()
            );
            // A fresh invocation must use this world's detached source and staging evidence.
        }
        let reopened = world.context();
        reopened
            .repository()
            .execute_root_preservation_step_checked(root, &spec, &step, &guard)
            .unwrap();
        // Recovery must recognize the exact completed step without repeating it.
        assert_eq!(
            world
                .context()
                .repository()
                .execute_root_preservation_step_checked(root, &spec, &step, &guard)
                .unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete
        );
    }
    let step = GitRootPreservationPhysicalStep::CreateStash {
        merge_id: "merge_context".into(),
    };
    world
        .context()
        .repository()
        .execute_root_preservation_step_checked(root, &spec, &step, &guard)
        .unwrap();
    assert_eq!(
        world
            .context()
            .repository()
            .execute_root_preservation_step_checked(root, &spec, &step, &guard)
            .unwrap(),
        GitCheckedPreservationMutation::AlreadyComplete
    );
    let reopened = world.context();
    let stashes = reopened
        .repository()
        .preservation_stashes(root, "merge_context")
        .unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].head_commit, spec.attached_commit);
    assert!(stashes[0].image.dirty.untracked);
    assert_eq!(
        reopened.filesystem().read(&root.join(LOCK)).unwrap(),
        b"attached lock\n"
    );
    assert!(
        reopened
            .filesystem()
            .kind(&root.join("user-work.txt"))
            .is_err()
    );
}
