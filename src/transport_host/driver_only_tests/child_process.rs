use super::*;

/// The environment variable that tells a row it is the child, and the directory it is to use as its home.
const CHILD_HOME: &str = "GWZ_P1_7_CHILD_HOME";

/// The home directory this process was started with by `rerun_in_child`, if it is such a child.
pub(crate) fn child_home() -> Option<PathBuf> {
    std::env::var_os(CHILD_HOME).map(PathBuf::from)
}

/// The environment of a child whose global configuration is `home/.gitconfig` and nothing else: libgit2 reads its
/// global search path from the environment when it first needs it, so a child is how a row gets a global configuration
/// of its own without moving the process-wide search path of the shared test binary (Code round-2 P3-1).
pub(crate) fn child_environment(home: &Path) -> Vec<(&'static str, OsString)> {
    vec![
        (CHILD_HOME, home.as_os_str().to_owned()),
        ("HOME", home.as_os_str().to_owned()),
        ("USERPROFILE", home.as_os_str().to_owned()),
        ("XDG_CONFIG_HOME", home.join("no-xdg").into_os_string()),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
    ]
}

/// Runs the row `row` of the module `module` (`module_path!()` of the caller) again, alone, in a child process of this
/// test binary with `environment`, and fails unless the child ran that one row and it passed. Returns the child's output.
pub(crate) fn rerun_in_child(module: &str, row: &str, environment: &[(&str, OsString)]) -> String {
    let name = format!(
        "{}::{row}",
        module.split_once("::").map_or(module, |(_, rest)| rest)
    );
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"]);
    for (variable, value) in environment {
        command.env(variable, value);
    }
    let output = command.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "the child failed:\n{text}");
    assert!(text.contains("1 passed"), "the child ran no row:\n{text}");
    text
}
