//! The hook-path preflight the LCM1.0c checkpoint (§6, §8, "Code P3-5")
//! deferred to lane I.
//!
//! Three outcomes — `EscapingConfig`, an admitted layout, `UnresolvableConfig`
//! — in each of three contexts: an ordinary checkout (the hook working
//! directory is the worktree root), a bare repository (the Git directory), and
//! a **push-triggered** hook in a checkout (also the Git directory, which in a
//! checkout is a *different* base from the worktree root).
//!
//! The push-hook cases are the ones a worktree-only implementation passes by
//! accident: each is arranged so the escape or the unresolvable resolution is
//! visible **only** from the Git-directory base, while the worktree base
//! resolves to a perfectly ordinary internal directory.

use git2::ObjectFormat;
use gwz_repo_contract::LayoutHazard;

use super::{admitted, assert_has, hazards_of};
use crate::LocalRepoInspector;
use crate::fixtures::{Fixture, symlink, symlink_loop};

const KEY: &str = "core.hooksPath";

fn escaping(hazards: &[LayoutHazard]) {
    assert_has(
        hazards,
        |hazard| matches!(hazard, LayoutHazard::EscapingConfig { key, .. } if key == KEY),
    );
}

fn unresolvable(hazards: &[LayoutHazard]) {
    assert_has(
        hazards,
        |hazard| matches!(hazard, LayoutHazard::UnresolvableConfig { key, .. } if key == KEY),
    );
}

// ---------------------------------------------------------------- checkout --

#[test]
fn checkout_an_escaping_relative_hook_path_is_escaping_config() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    std::fs::create_dir_all(fixture.outside().join("escaping-hooks")).expect("outside hooks");
    fixture.append_config("[core]\n\thooksPath = ../escaping-hooks\n");
    escaping(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

#[test]
fn checkout_an_absolute_hook_path_is_escaping_config() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let outside = fixture.outside().join("absolute-hooks");
    std::fs::create_dir_all(&outside).expect("outside hooks");
    fixture.append_config(&format!("[core]\n\thooksPath = {}\n", outside.display()));
    escaping(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

#[test]
fn checkout_a_valid_internal_relative_hook_path_is_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.mkdir(".githooks");
    fixture.append_config("[core]\n\thooksPath = .githooks\n");
    admitted(fixture.root());

    // A path that has not been created yet is still knowably internal.
    let pending = Fixture::checkout(ObjectFormat::Sha1);
    pending.append_config("[core]\n\thooksPath = .githooks\n");
    admitted(pending.root());
}

#[cfg(unix)]
#[test]
fn checkout_an_unresolvable_hook_path_is_unresolvable_config() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    symlink_loop(&fixture.real_root().join("loop-hooks"));
    fixture.append_config("[core]\n\thooksPath = loop-hooks\n");
    unresolvable(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

#[test]
fn checkout_a_hook_path_with_no_value_is_unresolvable_config() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.append_config("[core]\n\thooksPath\n");
    unresolvable(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

// ------------------------------------------------------------------- bare --

#[test]
fn bare_an_escaping_relative_hook_path_is_escaping_config() {
    let fixture = Fixture::bare(ObjectFormat::Sha1);
    std::fs::create_dir_all(fixture.outside().join("escaping-hooks")).expect("outside hooks");
    fixture.append_config("[core]\n\thooksPath = ../escaping-hooks\n");
    escaping(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

#[test]
fn bare_a_valid_internal_relative_hook_path_is_admitted() {
    let fixture = Fixture::bare(ObjectFormat::Sha1);
    fixture.mkdir("custom-hooks");
    fixture.append_config("[core]\n\thooksPath = custom-hooks\n");
    admitted(fixture.root());
}

#[cfg(unix)]
#[test]
fn bare_an_unresolvable_hook_path_is_unresolvable_config() {
    let fixture = Fixture::bare(ObjectFormat::Sha1);
    symlink_loop(&fixture.real_root().join("loop-hooks"));
    fixture.append_config("[core]\n\thooksPath = loop-hooks\n");
    unresolvable(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

// ------------------------------------------- push-hook working directory --
//
// A push-triggered hook (`pre-receive`, `update`, `post-receive`,
// `post-update`) runs with the **Git directory** as its working directory. In
// a checkout that is `<worktree>/.git`, one level deeper than the base an
// ordinary hook uses, so each of these fixtures is inert from the worktree
// base and decisive from the Git-directory base.

#[cfg(unix)]
#[test]
fn push_hook_working_directory_an_escaping_relative_hook_path_is_escaping_config() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    // Ordinary hooks would find a perfectly internal directory here.
    fixture.mkdir("ph-hooks");
    // Push hooks resolve the same value against `.git`, where it is a link
    // out of the tree.
    let outside = fixture.outside().join("outside-hooks");
    std::fs::create_dir_all(&outside).expect("outside hooks");
    symlink(&outside, &fixture.git_dir().join("ph-hooks"));
    fixture.append_config("[core]\n\thooksPath = ph-hooks\n");

    let hazards = hazards_of(&LocalRepoInspector::new(), fixture.root());
    escaping(&hazards);
    assert_has(&hazards, |hazard| {
        matches!(hazard, LayoutHazard::EscapingConfig { value, .. }
            if value.contains("ph-hooks") && value.contains("outside-hooks"))
    });
}

#[test]
fn push_hook_working_directory_a_valid_internal_relative_hook_path_is_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.mkdir("ph-ok");
    std::fs::create_dir_all(fixture.git_dir().join("ph-ok")).expect("git-dir hooks");
    fixture.append_config("[core]\n\thooksPath = ph-ok\n");
    admitted(fixture.root());
}

#[cfg(unix)]
#[test]
fn push_hook_working_directory_an_unresolvable_hook_path_is_unresolvable_config() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.mkdir("ph-loop");
    symlink_loop(&fixture.git_dir().join("ph-loop"));
    fixture.append_config("[core]\n\thooksPath = ph-loop\n");
    unresolvable(&hazards_of(&LocalRepoInspector::new(), fixture.root()));
}

#[cfg(unix)]
#[test]
fn a_hook_path_reached_through_an_internal_symlink_is_admitted() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let real = fixture.mkdir("hooks-real");
    symlink(&real, &fixture.real_root().join("hooks-link"));
    fixture.append_config("[core]\n\thooksPath = hooks-link\n");
    admitted(fixture.root());
}
