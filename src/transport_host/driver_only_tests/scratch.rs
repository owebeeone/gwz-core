use super::*;

// The scratch directory of `read_remote_file` (State P3-1).

pub(crate) const SCRATCH_CHILD: &str = "GWZ_P1_7_SCRATCH_CHILD";

#[test]
pub(crate) fn a_remote_file_read_refused_by_route_makes_no_scratch_directory() {
    if let Some(root) = std::env::var_os(SCRATCH_CHILD) {
        scratch_child(PathBuf::from(root));
        return;
    }
    // The process's temporary directory is the environment's, and the other rows make theirs in it: this row runs the
    // refusals in a process of its own with that directory missing.
    let root = TempDir::new().unwrap();
    let missing = root.path().join("no-scratch-root");
    let module = module_path!();
    let name = format!(
        "{}::a_remote_file_read_refused_by_route_makes_no_scratch_directory",
        module.split_once("::").map_or(module, |(_, rest)| rest)
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"])
        .env(SCRATCH_CHILD, root.path())
        .env("TMPDIR", &missing)
        .env("TMP", &missing)
        .env("TEMP", &missing)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "the child failed:\n{text}");
    assert!(text.contains("1 passed"), "the child ran no row:\n{text}");
    assert!(!missing.exists());
}
