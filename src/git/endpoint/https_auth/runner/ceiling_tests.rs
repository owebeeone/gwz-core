//! Whether the Git on this machine honors the ceiling that the owner sets above the helper's working directory
//! (the WH2 contract, OQ-E and spike rows S4 and S5; plan step 4.3 records the version this holds for). The
//! working directory on Windows is the Windows directory, whose parent is a drive root where any user may create
//! a folder, so a repository planted there must not be found.
use super::environment::{Environment, confine};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};

fn captured() -> Vec<(OsString, OsString)> {
    ["PATH", "SystemRoot"]
        .iter()
        .map(|name| {
            (
                (*name).into(),
                std::env::var_os(name).expect("the test process has this variable"),
            )
        })
        .collect()
}

fn git(directory: &Path, environment: Vec<(OsString, OsString)>, arguments: &[&str]) -> Output {
    Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env_clear()
        .envs(environment)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("the Windows tests need Git for Windows on PATH")
}

#[test]
fn git_does_not_search_above_the_ceiling_the_owner_sets() {
    let parent = tempfile::tempdir().unwrap();
    let child = parent.path().join("child dir");
    fs::create_dir(&child).unwrap();
    assert!(git(parent.path(), captured(), &["init", "-q"]).status.success());
    let version = String::from_utf8_lossy(&git(&child, captured(), &["--version"]).stdout)
        .trim()
        .to_owned();
    // The control: without a ceiling Git finds the repository one level above, so the test can tell.
    assert!(
        git(&child, captured(), &["rev-parse", "--git-dir"]).status.success(),
        "{version} did not find the planted repository"
    );
    let mut confined = Environment::snapshot(&captured());
    confine(&mut confined, &child);
    let found = git(&child, confined.into_pairs(), &["rev-parse", "--git-dir"]);
    assert!(
        !found.status.success(),
        "{version} searched above the ceiling: {}",
        String::from_utf8_lossy(&found.stdout)
    );
}
