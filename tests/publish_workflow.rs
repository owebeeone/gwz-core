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
    assert!(RELEASE_WORKFLOW.contains(
        "CLIPPY_CONF_DIR=\"$PWD\" cargo clippy --all-targets --all-features -- -D warnings"
    ));
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
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("test_release_boundary.py"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("python-version: \"3.11\""));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains(
        "CLIPPY_CONF_DIR=\"$PWD\" cargo clippy --all-targets --all-features -- -D warnings"
    ));
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
    assert!(release.contains("RELEASE_BOUNDARY_TEST"));
    assert!(release.contains("cargo\", \"clippy\", \"--all-targets\", \"--all-features"));
    assert!(release.contains("test_env[\"CLIPPY_CONF_DIR\"] = str(cargo_root)"));
}

#[test]
fn local_release_cannot_skip_or_tag_a_commit_before_the_boundary_gate() {
    let release = include_str!("../scripts/release.py");
    assert!(!release.contains("--no-clippy"));
    assert!(!release.contains("no_clippy"));
    let commit = release
        .find("git([\"commit\", \"-m\", message])")
        .expect("release script creates its version commit");
    let finalizer = release
        .find("def finalize_new_release(")
        .expect("release script has one new-tag finalizer");
    let exact_gate = release
        .find("gate_exact_release_commit(cargo_root=cargo_root, expected_head=expected_head)")
        .expect("new-tag finalizer gates its exact target");
    let tag = release
        .find("ensure_tag(tag, expected_head)")
        .expect("release script creates the tag");
    let finalizer_call = release
        .rfind("finalize_new_release(")
        .expect("main routes a new tag through the finalizer");
    assert!(finalizer < exact_gate);
    assert!(exact_gate < tag);
    assert!(commit < finalizer_call);
}
