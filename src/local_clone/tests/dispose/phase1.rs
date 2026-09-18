//! Phase 1 traceability (`GwzLaneCleanFixesPlan.md` S1.8; requirements
//! R17, R19): what recognising the copy clears, what it must still refuse,
//! and exactly what is left for Phase 2.
//!
//! The lane issues register (gwz-dev `dev-docs/GwzLaneIssues.md`, L1)
//! counted 112 hazard entries for every lane of the gwz-dev workspace, 60
//! `dirty` and 52 `unpreserved-history`, identical in every lane because
//! they described what the copy inherited. The fixture here is that
//! workspace in miniature -- a root and a member, each carrying a native
//! stash, a reflog-only commit, ignored user data and build caches -- and
//! the two tests are the two halves of R0: an integrated lane disposes in
//! one command (R0), and a lane holding anything the family does not still
//! refuses (R0.1, R19).

use super::*;

/// Add one `.git/info/exclude` rule to the repository at `path`, so a whole
/// directory is ignored and Git reports it as one entry -- the shape a real
/// cache has, and the shape the register counted once per directory.
fn ignore(path: &Path, rule: &str) {
    let exclude = path.join(".git/info/exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    let mut rules = fs::read(&exclude).unwrap_or_default();
    rules.extend_from_slice(format!("{rule}\n").as_bytes());
    fs::write(&exclude, rules).unwrap();
}

fn write(path: &Path, relative: &str, contents: &[u8]) {
    let file = path.join(relative);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contents).unwrap();
}

/// A workspace shaped like the one the register measured: a root and a
/// member, each with a native stash and a reflog-only commit, ignored user
/// data, a tagged cache, a `__pycache__`, a compiled extension and a build
/// tool's convenience symlink pointing outside the workspace.
///
/// Every one of them is the **family's**, made before any lane exists, so a
/// verbatim lane inherits the lot and owns none of it.
fn workspace_like_the_register_measured(label: &str) -> (FamilyFixture, Vec<String>) {
    let fixture = clean_family_workspace(label);
    let root = fixture.root.clone();
    let app = root.join("app");
    let mut carried = Vec::new();

    for repository in [&root, &app] {
        for rule in [
            "/target/",
            "/__pycache__/",
            "/bazel-out",
            "/notes.txt",
            "/_ext.abi3.so",
        ] {
            ignore(repository, rule);
        }
        write(repository, "target/CACHEDIR.TAG", CACHEDIR_TAG);
        write(repository, "target/debug/build.bin", b"built once\n");
        write(
            repository,
            "__pycache__/module.cpython-313.pyc",
            b"\x00pyc\n",
        );
        write(repository, "notes.txt", b"ignored user data\n");
        write(repository, "_ext.abi3.so", b"\x7fELF fixture\n");
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                std::os::unix::fs::symlink("/tmp", repository.join("bazel-out")).unwrap();
            }
        }
        // A reflog-only commit: made, then abandoned by `reset --hard`.
        let base = head_of(repository);
        carried.push(commit_in(
            repository,
            "abandoned.txt",
            "abandoned\n",
            "abandoned before any lane existed",
        ));
        reset_hard(repository, &base);
        // And a native stash, which leaves the worktree clean again.
        carried.push(stash_in(repository));
    }
    (fixture, carried)
}

/// The `CACHEDIR.TAG` signature, first line exactly as the specification
/// writes it. Phase 2's recogniser (S2.1) checks it; Phase 1 only needs the
/// fixture to look like a real cache.
const CACHEDIR_TAG: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55\n";

/// R0, R17, R19 (history half): the milestone. A whole verbatim lane of a
/// workspace carrying stashes, reflog-only commits, ignored user data and
/// caches, whose own work has been merged back, disposes in **one command**
/// -- no waiver, no operator comparison -- and the family keeps every one
/// of the things the lane was cleared over.
#[test]
fn a_merged_verbatim_lane_of_a_workspace_like_this_one_disposes_in_one_command() {
    let (fixture, carried) = workspace_like_the_register_measured("dispose-phase1-whole");
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    assert!(a.join("target/CACHEDIR.TAG").is_file(), "the copy is whole");
    assert!(a.join("app/notes.txt").is_file());

    // The lane does its work and it is integrated, as an agent's lane is.
    let in_lane = commit_in(
        &a.join("app"),
        "feature.txt",
        "from the lane\n",
        "work done in the lane",
    );
    crate::workspace_ops::handle_merge_with_local_family(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        crate::MergeRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@all".into()],
                    exclude_targets: vec!["@root".into()],
                    ..Default::default()
                }),
                ..meta("req-family-merge")
            },
            op: crate::MergeOp::Start,
            local_source_name: Some("A".to_owned()),
            ..Default::default()
        },
        "op_family_merge",
        &NullSink,
    )
    .expect("the family merge integrates the lane's work");
    assert_eq!(head_of(&fixture.root.join("app")), in_lane);

    // R0: one command, no waiver.
    let response = local(&fixture.root, delete_request("A", &[]));
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("deleted local clone `A`"), "{message}");
    assert!(
        !message.contains("forced past"),
        "112 entries stood here once, and every one of them was the copy's: {message}"
    );
    assert!(!a.exists());
    assert_eq!(listed_names(&fixture.root), ["root"]);

    // The family kept every one of the things the disposal was cleared
    // over: the ignored data, the caches, and each abandoned or stashed
    // commit in the repository that made it.
    for repository in [fixture.root.clone(), fixture.root.join("app")] {
        assert!(repository.join("notes.txt").is_file());
        assert!(repository.join("target/debug/build.bin").is_file());
        assert!(
            repository
                .join("__pycache__/module.cpython-313.pyc")
                .is_file()
        );
    }
    let mut survived = 0;
    for repository in [fixture.root.clone(), fixture.root.join("app")] {
        let git = git2::Repository::open(&repository).unwrap();
        survived += carried
            .iter()
            .filter_map(|oid| git2::Oid::from_str(oid).ok())
            .filter(|oid| git.find_commit(*oid).is_ok())
            .count();
    }
    assert_eq!(
        survived,
        carried.len(),
        "every abandoned and stashed commit the lane copied is still the family's"
    );
}

/// R0.1, R19: the control. A lane holding a commit the family does not
/// hold, and a lane holding ignored data the family does not hold, each
/// still refuse -- named under `unique to the lane` -- and nothing is
/// removed. The same fixture that disposes in one command above.
#[test]
fn a_lane_holding_a_unique_commit_or_unique_ignored_data_still_refuses() {
    for (label, change, expected) in [
        (
            "a commit no family repository holds",
            &(|lane: &Path| {
                commit_in(
                    &lane.join("app"),
                    "feature.txt",
                    "never integrated\n",
                    "only in the lane",
                );
            }) as &dyn Fn(&Path),
            "unpreserved-history",
        ),
        (
            "ignored data no family repository holds",
            &(|lane: &Path| {
                fs::write(lane.join("app/notes.txt"), b"the lane's own note\n").unwrap();
            }) as &dyn Fn(&Path),
            "dirty",
        ),
    ] {
        let (fixture, _) = workspace_like_the_register_measured("dispose-phase1-unique");
        clone(&fixture.root, "A");
        let a = fixture.sibling("A");
        change(&a);
        let before = tree_bytes(&a);

        let error = refuse(&fixture.root, delete_request("A", &[]));
        assert_refused_without_effect(
            &fixture,
            &a,
            &before,
            &error,
            ErrorCode::UnwaivedHazard,
            &["unique to the lane", "nothing was removed"],
        );
        assert_eq!(
            printed_waivers(&error.message),
            [expected],
            "{label}: {}",
            error.message
        );
        // What the lane inherited is still not what refused.
        assert!(
            error
                .message
                .split("; changed copy")
                .next()
                .is_some_and(|unchanged| unchanged.contains("unchanged copy")),
            "{label}: {}",
            error.message
        );
        assert!(a.is_dir(), "{label}");
    }
}

/// S1.8's second question: **what is left after Phase 1**, stated exactly.
///
/// A real lane is built in, so its caches are not the bytes the copy
/// brought. Phase 1 has no recogniser for regenerable data (that is R5, R6
/// and plan S2.1/S2.2), so a rebuilt cache falls into `changed copy` when
/// the family has the same path and into `unique to the lane` when it does
/// not, and it refuses under the `dirty` waiver. Nothing else remains: the
/// history, the stash, the untouched ignored user data and the untouched
/// caches are all cleared. Plan S2.3 moves exactly these entries into the
/// `regenerable` category, which Phase 1 already reports and leaves empty.
///
/// Note what the fixture also shows about R1's **cheap** fingerprint: the
/// lane rewrote `target/debug/build.bin`, two levels below the recorded
/// directory entry `target/`, and one `stat` of `target/` does not see it,
/// so that entry reads as an unchanged copy. The register's own caches are
/// recorded as whole directories, so this is the common case, and it is
/// what R1 asks for (size, mtime and inode). Plan §8 records it as a known
/// limitation for S3.3 to price.
#[test]
fn what_remains_after_phase_one_is_the_caches_the_lane_rebuilt() {
    let (fixture, _) = workspace_like_the_register_measured("dispose-phase1-remainder");
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    // The lane builds, as a lane exists to do.
    fs::write(a.join("target/debug/build.bin"), b"built again\n").unwrap();
    fs::write(a.join("app/__pycache__/lane.cpython-313.pyc"), b"\x00new\n").unwrap();
    let before = tree_bytes(&a);

    let error = refuse(&fixture.root, delete_request("A", &[]));
    assert_refused_without_effect(
        &fixture,
        &a,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &["nothing was removed"],
    );
    // Exactly one waiver, and it is the one Phase 2 removes the need for.
    assert_eq!(printed_waivers(&error.message), ["dirty"]);
    assert!(
        error.message.contains("regenerable 0"),
        "the category Phase 2 fills is reported and empty: {}",
        error.message
    );
    // Every remaining entry is a rebuilt cache, and nothing else.
    for category in ["; changed copy ", "; unique to the lane "] {
        let Some((_, rest)) = error.message.split_once(category) else {
            panic!("no `{category}` category: {}", error.message);
        };
        let listed = rest.split(';').next().unwrap_or_default();
        for entry in listed.split(", `") {
            assert!(
                !entry.contains('(')
                    || entry.contains("(target/")
                    || entry.contains("(app/__pycache__/")
                    || entry.contains("(__pycache__/"),
                "only a rebuilt cache may remain, found `{entry}` in {category}: {}",
                error.message
            );
        }
    }
    // And the operator's own data, untouched by the lane, is not among them.
    assert!(
        error
            .message
            .split("; changed copy")
            .next()
            .is_some_and(|unchanged| unchanged.contains("(notes.txt)")),
        "{}",
        error.message
    );
}
