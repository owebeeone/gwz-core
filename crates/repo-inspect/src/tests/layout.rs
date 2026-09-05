//! Design §4.0, one test per refusal: "Inventory every included Git
//! repository before copying … Apply these checks and exclusions to each, or
//! refuse an unsupported nested layout before reservation."

use std::path::Path;

use git2::ObjectFormat;
use gwz_repo_contract::{
    HeadState, LayoutError, LayoutHazard, ObjectFormat as ContractFormat, RepoInspector,
};

use super::{admitted, assert_has, assert_none, hazards_of};
use crate::fixtures::Fixture;
use crate::{Environment, LocalRepoInspector};

#[test]
fn a_plain_checkout_is_admitted_with_its_resolved_paths_and_attached_head() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let info = admitted(fixture.root());
    assert_eq!(info.path, fixture.real_root());
    assert_eq!(info.git_dir, fixture.real_root().join(".git"));
    assert_eq!(info.common_dir, info.git_dir);
    assert!(!info.bare);
    assert_eq!(info.object_format, ContractFormat::Sha1);
    let HeadState::Attached { branch, .. } = &info.head else {
        panic!("expected an attached HEAD, got {:?}", info.head);
    };
    assert_eq!(branch, "refs/heads/main");
}

#[test]
fn a_bare_repository_is_admitted_and_reports_its_git_directory_as_the_path() {
    let fixture = Fixture::bare(ObjectFormat::Sha1);
    let info = admitted(fixture.root());
    assert!(info.bare);
    assert_eq!(info.path, fixture.real_root());
    assert_eq!(info.git_dir, fixture.real_root());
    assert_eq!(
        info.head,
        HeadState::Unborn {
            branch: "refs/heads/main".to_owned()
        }
    );
}

#[test]
fn a_sha256_checkout_reports_its_object_format_and_a_thirty_two_byte_head() {
    let fixture = Fixture::checkout(ObjectFormat::Sha256);
    let info = admitted(fixture.root());
    assert_eq!(info.object_format, ContractFormat::Sha256);
    let HeadState::Attached { target, .. } = &info.head else {
        panic!("expected an attached HEAD, got {:?}", info.head);
    };
    assert_eq!(target.format(), ContractFormat::Sha256);
    assert_eq!(target.as_bytes().len(), 32);
    assert_eq!(target.to_hex().len(), 64);
}

#[test]
fn a_detached_head_is_reported_as_detached() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let commit = fixture.head_commit();
    fixture.detach_head();
    let info = admitted(fixture.root());
    let HeadState::Detached { target } = &info.head else {
        panic!("expected a detached HEAD, got {:?}", info.head);
    };
    assert_eq!(target.to_hex(), commit.to_string());
}

#[test]
fn an_unborn_head_is_reported_as_unborn_not_as_a_read_failure() {
    let fixture = Fixture::empty_checkout(ObjectFormat::Sha1);
    assert_eq!(
        admitted(fixture.root()).head,
        HeadState::Unborn {
            branch: "refs/heads/main".to_owned()
        }
    );
}

#[test]
fn a_path_that_is_not_a_repository_refuses_typed_without_searching_upwards() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let inspector = LocalRepoInspector::new();
    // A subdirectory of a repository is not itself a repository: `NO_SEARCH`
    // stops the answer from being the parent's layout.
    let subdirectory = fixture.mkdir("nested/deeper");
    match inspector.inspect_layout(&subdirectory) {
        Err(LayoutError::NotARepository { path }) => assert_eq!(path, subdirectory),
        other => panic!("expected NotARepository, got {other:?}"),
    }
    assert!(matches!(
        inspector.inspect_layout(Path::new("/nowhere-at-all")),
        Err(LayoutError::NotARepository { .. })
    ));
}

#[test]
fn a_gitfile_is_refused_and_its_external_common_directory_named() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let linked = fixture.linked_gitfile("linked");
    let hazards = hazards_of(&LocalRepoInspector::new(), &linked);
    assert_has(
        &hazards,
        |hazard| matches!(hazard, LayoutHazard::GitFile { path } if path == &linked.join(".git")),
    );
    assert_has(&hazards, |hazard| {
        matches!(hazard, LayoutHazard::ExternalCommonDir { .. })
    });
}

#[test]
fn an_alternates_file_is_refused_including_the_http_spelling() {
    for name in ["alternates", "http-alternates"] {
        let fixture = Fixture::checkout(ObjectFormat::Sha1);
        let donor = fixture.outside().join("donor/objects");
        std::fs::create_dir_all(&donor).expect("donor");
        let alternates = fixture.git_dir().join("objects/info").join(name);
        std::fs::write(&alternates, format!("{}\n", donor.display())).expect("alternates");
        let hazards = hazards_of(&LocalRepoInspector::new(), fixture.root());
        assert_has(
            &hazards,
            |hazard| matches!(hazard, LayoutHazard::Alternates { path } if path == &alternates),
        );
    }
}

#[cfg(unix)]
#[test]
fn an_object_store_reached_through_an_escaping_symlink_is_refused() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let objects = fixture.git_dir().join("objects");
    let moved = fixture.outside().join("moved-objects");
    std::fs::rename(&objects, &moved).expect("move the object store out");
    crate::fixtures::symlink(&moved, &objects);
    let hazards = hazards_of(&LocalRepoInspector::new(), fixture.root());
    assert_has(&hazards, |hazard| {
        matches!(
            hazard,
            LayoutHazard::EscapingMetadataLink { path, target }
                if path == &objects && target == &std::fs::canonicalize(&moved).expect("real")
        )
    });
}

#[cfg(unix)]
#[test]
fn metadata_reached_through_an_internal_symlink_is_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let logs = fixture.git_dir().join("logs");
    let inside = fixture.real_root().join(".git/relocated-logs");
    std::fs::create_dir_all(&inside).expect("inside");
    if logs.exists() {
        std::fs::remove_dir_all(&logs).expect("clear logs");
    }
    crate::fixtures::symlink(&inside, &logs);
    admitted(fixture.root());
}

#[test]
fn a_core_worktree_naming_a_path_outside_the_boundary_is_refused() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let outside = fixture.outside().join("elsewhere-tree");
    std::fs::create_dir_all(&outside).expect("outside tree");
    fixture.append_config(&format!("[core]\n\tworktree = {}\n", outside.display()));
    let hazards = hazards_of(&LocalRepoInspector::new(), fixture.root());
    assert_has(
        &hazards,
        |hazard| matches!(hazard, LayoutHazard::EscapingConfig { key, .. } if key == "core.worktree"),
    );
}

#[test]
fn a_core_worktree_inside_the_boundary_is_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    // `..` from the Git directory is the worktree root: valid and internal.
    fixture.append_config("[core]\n\tworktree = ..\n");
    admitted(fixture.root());
}

#[test]
fn an_include_path_outside_the_boundary_is_refused_and_an_internal_one_is_not() {
    let escaping = Fixture::checkout(ObjectFormat::Sha1);
    escaping.append_config("[include]\n\tpath = ../../outside.config\n");
    assert_has(
        &hazards_of(&LocalRepoInspector::new(), escaping.root()),
        |hazard| matches!(hazard, LayoutHazard::EscapingConfig { key, .. } if key == "include.path"),
    );

    let internal = Fixture::checkout(ObjectFormat::Sha1);
    std::fs::write(
        internal.git_dir().join("extra.config"),
        "[core]\n\tquotepath = false\n",
    )
    .expect("extra config");
    internal.append_config("[include]\n\tpath = extra.config\n");
    admitted(internal.root());
}

#[test]
fn an_include_if_path_outside_the_boundary_is_refused() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.append_config("[includeIf \"gitdir:**\"]\n\tpath = /etc/gwz-nonexistent.config\n");
    assert_has(
        &hazards_of(&LocalRepoInspector::new(), fixture.root()),
        |hazard| {
            matches!(hazard, LayoutHazard::EscapingConfig { key, .. }
                if key.starts_with("includeIf.") && key.ends_with(".path"))
        },
    );
}

#[test]
fn a_url_insteadof_naming_a_path_outside_the_boundary_is_refused() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.append_config("[url \"../peer-mirror\"]\n\tinsteadOf = peer:\n");
    assert_has(
        &hazards_of(&LocalRepoInspector::new(), fixture.root()),
        |hazard| {
            matches!(hazard, LayoutHazard::EscapingConfig { key, .. }
                if key.starts_with("url.") && key.ends_with(".insteadOf"))
        },
    );
}

#[test]
fn an_ordinary_https_remote_and_insteadof_are_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.append_config(
        "[remote \"origin\"]\n\turl = https://example.invalid/repo.git\n\
         [url \"https://example.invalid/\"]\n\tinsteadOf = up:\n",
    );
    admitted(fixture.root());
}

#[test]
fn a_promisor_remote_and_a_promisor_pack_are_refused_as_partial_clones() {
    let configured = Fixture::checkout(ObjectFormat::Sha1);
    configured.append_config("[remote \"origin\"]\n\tpromisor = true\n");
    assert_has(
        &hazards_of(&LocalRepoInspector::new(), configured.root()),
        |hazard| matches!(hazard, LayoutHazard::PartialClone { .. }),
    );

    let packed = Fixture::checkout(ObjectFormat::Sha1);
    let pack_dir = packed.git_dir().join("objects/pack");
    std::fs::create_dir_all(&pack_dir).expect("pack dir");
    std::fs::write(pack_dir.join("pack-abc.promisor"), b"").expect("promisor marker");
    assert_has(
        &hazards_of(&LocalRepoInspector::new(), packed.root()),
        |hazard| matches!(hazard, LayoutHazard::PartialClone { .. }),
    );
}

#[test]
fn every_environment_redirection_refuses_an_otherwise_admissible_repository() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    for variable in crate::environment::REDIRECTING_VARIABLES {
        let inspector = LocalRepoInspector::with_environment(Environment::from_vars([(
            *variable,
            "/elsewhere",
        )]));
        let hazards = hazards_of(&inspector, fixture.root());
        assert_has(&hazards, |hazard| {
            matches!(hazard, LayoutHazard::EnvironmentOverride { variable: reported }
                if reported == variable)
        });
    }
    // The same repository with an explicitly empty view is admitted, so the
    // refusal is the environment and nothing else.
    assert!(
        LocalRepoInspector::with_environment(Environment::none())
            .inspect_layout(fixture.root())
            .is_ok()
    );
}

#[test]
fn a_missing_configuration_file_is_unresolvable_rather_than_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    std::fs::remove_file(fixture.git_dir().join("config")).expect("remove config");
    let hazards = hazards_of(&LocalRepoInspector::new(), fixture.root());
    assert_has(
        &hazards,
        |hazard| matches!(hazard, LayoutHazard::UnresolvableConfig { key, .. } if key == "config"),
    );
    assert_none(&hazards, |hazard| {
        matches!(hazard, LayoutHazard::EscapingConfig { .. })
    });
}

#[test]
fn hazards_aggregate_so_one_call_reports_every_reason() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let donor = fixture.outside().join("donor/objects");
    std::fs::create_dir_all(&donor).expect("donor");
    std::fs::write(
        fixture.git_dir().join("objects/info/alternates"),
        format!("{}\n", donor.display()),
    )
    .expect("alternates");
    fixture.append_config("[core]\n\thooksPath = ../escaping-hooks\n");
    let inspector = LocalRepoInspector::with_environment(Environment::from_vars([(
        "GIT_WORK_TREE",
        "/elsewhere",
    )]));
    let hazards = hazards_of(&inspector, fixture.root());
    assert_has(&hazards, |hazard| {
        matches!(hazard, LayoutHazard::Alternates { .. })
    });
    assert_has(
        &hazards,
        |hazard| matches!(hazard, LayoutHazard::EscapingConfig { key, .. } if key == "core.hooksPath"),
    );
    assert_has(&hazards, |hazard| {
        matches!(hazard, LayoutHazard::EnvironmentOverride { .. })
    });
    assert!(hazards.len() >= 3, "{hazards:?}");
}

#[test]
fn the_contract_conformance_suite_passes_against_a_real_repository() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let temp = tempfile::tempdir().expect("tempdir");
    gwz_repo_contract::contract_tests::inspector_conformance(
        &LocalRepoInspector::new(),
        temp.path(),
        Some(fixture.root()),
    );
}
