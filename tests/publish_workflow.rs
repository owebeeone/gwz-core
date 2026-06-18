const RELEASE_WORKFLOW: &str = include_str!("../.github/workflows/release.yml");

#[test]
fn release_workflow_tests_linux_and_windows() {
    assert!(RELEASE_WORKFLOW.contains("ubuntu-22.04"));
    assert!(RELEASE_WORKFLOW.contains("windows-2022"));
}

#[test]
fn release_workflow_runs_full_rust_verification() {
    assert!(RELEASE_WORKFLOW.contains("cargo fmt --check"));
    assert!(RELEASE_WORKFLOW.contains("cargo test --locked"));
    assert!(RELEASE_WORKFLOW.contains("cargo clippy --all-targets -- -D warnings"));
}

#[test]
fn release_workflow_installs_release_taut_proto_for_protocol_tests() {
    assert!(RELEASE_WORKFLOW.contains("actions/setup-python"));
    assert!(RELEASE_WORKFLOW.contains("TAUT_PYTHON: python"));
    assert!(RELEASE_WORKFLOW.contains("python -m pip install --upgrade pip taut-proto"));
}

#[test]
fn release_workflow_only_runs_for_explicit_releases() {
    assert!(RELEASE_WORKFLOW.contains("release:"));
    assert!(RELEASE_WORKFLOW.contains("types: [published]"));
    assert!(RELEASE_WORKFLOW.contains("workflow_dispatch"));
    assert!(!RELEASE_WORKFLOW.contains("pull_request:"));
    assert!(!RELEASE_WORKFLOW.contains("branches:"));
}
