const RELEASE_WORKFLOW: &str = include_str!("../.github/workflows/release.yml");
const CHECKED_ARTIFACT_WORKFLOW: &str =
    include_str!("../.github/workflows/checked-artifact-boundary.yml");

#[test]
fn release_workflow_tests_linux_and_windows() {
    assert!(RELEASE_WORKFLOW.contains("ubuntu-22.04"));
    assert!(RELEASE_WORKFLOW.contains("windows-2022"));
}

#[test]
fn release_workflow_runs_full_rust_verification() {
    assert!(RELEASE_WORKFLOW.contains("cargo fmt --check"));
    assert!(RELEASE_WORKFLOW.contains("Run 'cargo fmt' from the gwz-core repo root"));
    assert!(RELEASE_WORKFLOW.contains("cargo test --locked"));
    assert!(RELEASE_WORKFLOW.contains("cargo clippy --all-targets -- -D warnings"));
}

#[test]
fn release_workflow_installs_release_taut_proto_for_protocol_tests() {
    assert!(RELEASE_WORKFLOW.contains("actions/setup-python"));
    assert!(RELEASE_WORKFLOW.contains("TAUT_PYTHON: python"));
    assert!(RELEASE_WORKFLOW.contains("python -m pip install --upgrade pip \"taut-proto==0.8.1\""));
}

#[test]
fn release_workflow_only_runs_for_explicit_releases() {
    assert!(RELEASE_WORKFLOW.contains("release:"));
    assert!(RELEASE_WORKFLOW.contains("types: [published]"));
    assert!(RELEASE_WORKFLOW.contains("workflow_dispatch"));
    assert!(!RELEASE_WORKFLOW.contains("pull_request:"));
    assert!(!RELEASE_WORKFLOW.contains("branches:"));
}

#[test]
fn checked_artifact_boundary_runs_before_merge_and_on_main_push() {
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("pull_request:"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("push:"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("branches: [main]"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("check_checked_artifact_boundaries.py"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("test_check_checked_artifact_boundaries.py"));
}

#[test]
fn local_release_runs_checked_artifact_boundary_before_rust_tests() {
    let release = include_str!("../scripts/release.py");
    let boundary = release
        .find("[sys.executable, CHECKED_ARTIFACT_BOUNDARY]")
        .expect("release gate invokes the boundary checker");
    let tests = release
        .find("[\"cargo\", \"test\", \"--locked\"]")
        .expect("release gate invokes Rust tests");
    assert!(boundary < tests);
    assert!(release.contains("CHECKED_ARTIFACT_BOUNDARY_TEST"));
}
