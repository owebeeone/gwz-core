//! Phase 2 traceability (`GwzLaneCleanFixesPlan.md` S2.4; requirements
//! R17, R18, and R19's ignored-data half): what recognising regenerable
//! data clears, and what it must still refuse.
//!
//! The lane issues register (gwz-dev `dev-docs/GwzLaneIssues.md`, L1)
//! counted 112 hazard entries, 48 of them build output: 25 `__pycache__/`
//! directories, 13 `CACHEDIR.TAG` caches, 7 bazel and razel output links,
//! gwz-py's `_gwz_core.abi3.so` and `src/gwz_py.egg-info/`, and one
//! `target/` an older cargo left untagged. Phase 1's fixture is that
//! workspace in miniature; this module extends it with the two shapes
//! Phase 1's did not carry -- an **untagged** build directory and an
//! egg-info -- and then does to the lane what a lane is for: it builds in
//! it.
//!
//! The question S2.4 answers is R0 for a lane that was **built in**, which
//! is every real lane. Phase 1 left exactly that case refusing.

use super::phase1::{CACHEDIR_TAG, ignore, workspace_like_the_register_measured, write};
use super::*;

/// Phase 1's workspace plus the two regenerable shapes it did not carry,
/// and the two ignore rules the lane's own data will arrive under.
///
/// Every file here is the **family's**, made before any lane exists, so a
/// verbatim lane inherits the lot and owns none of it.
fn workspace_with_every_regenerable_shape(label: &str) -> FamilyFixture {
    let (fixture, _) = workspace_like_the_register_measured(label);
    for repository in [fixture.root.clone(), fixture.root.join("app")] {
        for rule in [
            "/untagged-target/",
            "/pkg.egg-info/",
            "/lane-cache/",
            "/lane-only.txt",
        ] {
            ignore(&repository, rule);
        }
        // A build directory whose tool wrote no `CACHEDIR.TAG`, exactly
        // the register's `gwz-cli/target` case (R6): cargo's own metadata
        // file and cargo's profile layout, and no tag anywhere.
        write(&repository, "untagged-target/.rustc_info.json", b"{}\n");
        write(
            &repository,
            "untagged-target/debug/.fingerprint/app/lib",
            b"stamp\n",
        );
        write(
            &repository,
            "untagged-target/debug/deps/libapp.rlib",
            b"\x00\n",
        );
        // setuptools' own metadata directory (R5).
        write(
            &repository,
            "pkg.egg-info/PKG-INFO",
            b"Metadata-Version: 2.1\nName: pkg\n",
        );
        write(
            &repository,
            "pkg.egg-info/SOURCES.txt",
            b"pkg/__init__.py\n",
        );
    }
    fixture
}

/// Everything a build does to a lane, in one place: each of the five
/// regenerable shapes is rebuilt, and the lane makes a cache of its own
/// that the family never had.
fn build_in(lane: &Path) {
    for repository in [lane.to_path_buf(), lane.join("app")] {
        // A tagged cache, rebuilt.
        fs::write(repository.join("target/debug/build.bin"), b"rebuilt\n").unwrap();
        // A bytecode cache, with a module the family never compiled.
        fs::write(
            repository.join("__pycache__/lane.cpython-313.pyc"),
            b"\x00lane\n",
        )
        .unwrap();
        // An untagged build directory, rebuilt.
        fs::write(
            repository.join("untagged-target/debug/deps/libapp.rlib"),
            b"\x00rebuilt\n",
        )
        .unwrap();
        // Package metadata, rewritten by the build.
        fs::write(
            repository.join("pkg.egg-info/SOURCES.txt"),
            b"pkg/__init__.py\npkg/lane.py\n",
        )
        .unwrap();
        // A compiled extension module, relinked.
        fs::write(repository.join("_ext.abi3.so"), b"\x7fELF relinked\n").unwrap();
        // And a cache the lane made from nothing (R7's second half).
        fs::create_dir_all(repository.join("lane-cache")).unwrap();
        fs::write(repository.join("lane-cache/CACHEDIR.TAG"), CACHEDIR_TAG).unwrap();
        fs::write(repository.join("lane-cache/object.bin"), b"only here\n").unwrap();
    }
}

/// **R0, R17, R18.** The milestone: a verbatim lane of a workspace
/// carrying every hazard the register counted, whose work has been merged
/// back, and which was **built in** -- a rebuilt tagged cache, a rebuilt
/// untagged build directory, a `__pycache__` with a new module in it, a
/// rewritten egg-info, a relinked `.so`, a build tool's convenience link
/// and a cache the lane invented -- disposes in **one command**.
///
/// This is precisely the case Phase 1 left refusing.
#[test]
fn a_merged_verbatim_lane_that_was_built_in_disposes_in_one_command() {
    let fixture = workspace_with_every_regenerable_shape("dispose-phase2-built-in");
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    assert!(
        a.join("untagged-target/.rustc_info.json").is_file(),
        "the copy is whole"
    );
    assert!(a.join("app/pkg.egg-info/PKG-INFO").is_file());

    // The lane does its work, and it is integrated, as an agent's lane is.
    let in_lane = commit_in(
        &a.join("app"),
        "feature.txt",
        "from the lane\n",
        "work done in the lane",
    );
    build_in(&a);
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
        "a built-in lane needs no waiver: {message}"
    );
    assert!(!a.exists());
    assert_eq!(listed_names(&fixture.root), ["root"]);

    // The family kept its own build output, untouched by the deletion.
    for repository in [fixture.root.clone(), fixture.root.join("app")] {
        assert!(repository.join("target/debug/build.bin").is_file());
        assert!(
            repository
                .join("untagged-target/.rustc_info.json")
                .is_file()
        );
        assert!(repository.join("pkg.egg-info/PKG-INFO").is_file());
        assert!(repository.join("notes.txt").is_file());
    }
}

/// **R19, R0.1.** The control, and the reason the recogniser is narrow: a
/// lane holding ignored data that is *not* regenerable, and that no family
/// repository holds, still refuses -- named under `unique to the lane`,
/// with its path -- while the caches it made in the same run sit in
/// `regenerable` and refuse nothing.
#[test]
fn unique_ignored_user_data_still_refuses_beside_the_lanes_own_caches() {
    let fixture = workspace_with_every_regenerable_shape("dispose-phase2-unique");
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    build_in(&a);
    // Not a cache: a file a person made, that the family has never seen.
    fs::write(a.join("app/lane-only.txt"), b"the lane's own note\n").unwrap();
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
    assert_eq!(printed_waivers(&error.message), ["dirty"]);

    let unique = category_of(&error.message, "unique to the lane");
    assert!(
        unique.contains("(lane-only.txt)"),
        "the lane's own file is what refused: {unique}"
    );
    let regenerable = category_of(&error.message, "regenerable");
    for made in ["lane-cache/", "__pycache__/", "untagged-target/"] {
        assert!(
            regenerable.contains(made),
            "`{made}` belongs in `regenerable`, which reads: {regenerable}"
        );
    }
    assert!(
        !unique.contains("lane-cache/") && !unique.contains("untagged-target/"),
        "and not beside it: {unique}"
    );
}

/// **R7, the half a comparison can never answer.** A cache the lane
/// created that the family never had is unique by every comparison and
/// regenerable all the same, so it does not refuse. Nothing else is
/// changed in the lane, so this is the whole of what is asked.
#[test]
fn a_cache_the_lane_invented_is_regenerable_and_does_not_refuse() {
    let fixture = workspace_with_every_regenerable_shape("dispose-phase2-invented");
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    fs::create_dir_all(a.join("app/lane-cache")).unwrap();
    fs::write(a.join("app/lane-cache/CACHEDIR.TAG"), CACHEDIR_TAG).unwrap();
    fs::write(a.join("app/lane-cache/object.bin"), b"only ever here\n").unwrap();

    let response = local(&fixture.root, delete_request("A", &[]));
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("deleted local clone `A`"), "{message}");
    assert!(!message.contains("forced past"), "{message}");
    assert!(!a.exists());
    assert!(
        !fixture.root.join("app/lane-cache").exists(),
        "and the family never gains it"
    );
}

/// **R6, end to end.** An untagged build directory is recognised by its
/// tool's markers, and a directory of the same name carrying none of them
/// is not: the lane's copy of it refuses, naming its path.
#[test]
fn an_untagged_directory_without_its_tools_markers_is_not_regenerable() {
    let fixture = workspace_with_every_regenerable_shape("dispose-phase2-untagged");
    // The family ignores a second directory of the same shape of name.
    // The lane then makes one, holding a person's file and nothing any
    // build tool writes.
    for repository in [fixture.root.clone(), fixture.root.join("app")] {
        ignore(&repository, "/not-a-target/");
    }
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    build_in(&a);
    fs::create_dir_all(a.join("app/not-a-target")).unwrap();
    fs::write(a.join("app/not-a-target/keep.txt"), b"a person's file\n").unwrap();
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
    let unique = category_of(&error.message, "unique to the lane");
    assert!(
        unique.contains("(not-a-target/)"),
        "a directory with no tool's marker is the lane's: {unique}"
    );
    let regenerable = category_of(&error.message, "regenerable");
    assert!(
        regenerable.contains("untagged-target/") && !regenerable.contains("not-a-target/"),
        "only the one with cargo's markers is regenerable: {regenerable}"
    );
}

/// The refusal's rendering of one category: everything between its name
/// and the next `;`. The message is `` <category> <count>: <items> ``,
/// with every category present even when empty (R9).
fn category_of<'a>(message: &'a str, category: &str) -> &'a str {
    let Some((_, rest)) = message.split_once(&format!("{category} ")) else {
        panic!("no `{category}` category in: {message}");
    };
    rest.split(';').next().unwrap_or_default()
}
