const RELEASE_WORKFLOW: &str = include_str!("../.github/workflows/release.yml");
const CHECKED_ARTIFACT_WORKFLOW: &str =
    include_str!("../.github/workflows/checked-artifact-boundary.yml");

#[test]
fn release_workflow_tests_linux_and_windows() {
    assert!(RELEASE_WORKFLOW.contains("ubuntu-24.04"));
    assert!(RELEASE_WORKFLOW.contains("windows-2022"));
}

#[test]
fn release_workflow_runs_full_rust_verification() {
    assert!(RELEASE_WORKFLOW.contains("cargo fmt --check"));
    assert!(RELEASE_WORKFLOW.contains("Run 'cargo fmt' from the gwz-core repo root"));
    assert!(RELEASE_WORKFLOW.contains("python scripts/run_tests.py"));
    assert!(RELEASE_WORKFLOW.contains(
        "CLIPPY_CONF_DIR=\"$PWD\" cargo clippy --all-targets --all-features -- -D warnings"
    ));
}

#[test]
fn release_workflow_installs_release_taut_proto_for_protocol_tests() {
    assert!(RELEASE_WORKFLOW.contains("actions/setup-python"));
    assert!(RELEASE_WORKFLOW.contains("TAUT_PYTHON: python"));
    assert!(RELEASE_WORKFLOW.contains("python -m pip install --upgrade pip \"taut-proto==0.9.1\""));
}

#[test]
fn release_workflow_has_an_exact_v0105_boundary_compatibility_exception() {
    assert!(RELEASE_WORKFLOW.contains("if [ \"$tag\" = \"v0.10.5\" ]; then"));
    assert!(RELEASE_WORKFLOW.contains(
        "v0.10.5 predates the checked-artifact boundary; running the remaining release gates."
    ));
    assert!(
        !RELEASE_WORKFLOW.contains("if [ -f scripts/checks/check_checked_artifact_boundaries.py")
    );
}

#[test]
fn release_workflow_can_verify_an_explicit_dispatch_ref_without_a_fake_tag() {
    assert!(RELEASE_WORKFLOW.contains("required: false"));
    assert!(
        RELEASE_WORKFLOW
            .contains("ref: ${{ github.event.release.tag_name || inputs.tag || github.sha }}")
    );
    assert!(RELEASE_WORKFLOW.contains("if [ -z \"$tag\" ]; then"));
    assert!(RELEASE_WORKFLOW.contains("git rev-parse --verify \"$GITHUB_SHA\""));
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
fn release_workflow_gates_publication_on_the_linux_verification_alone() {
    // Plan D5 / S2.1 (dev-docs/GwzCratesIoPlan.md): publishing waits on the
    // Linux verification and must not wait on Windows. `needs:` names a job,
    // and a matrix job has no per-leg signal to name (its outputs are shared),
    // so the two legs are separate jobs here and `publish` needs the Linux one.
    assert!(RELEASE_WORKFLOW.contains("name: Verify (ubuntu-24.04)"));
    assert!(RELEASE_WORKFLOW.contains("name: Verify (windows-2022)"));
    assert!(!RELEASE_WORKFLOW.contains("strategy:"));
    assert!(!RELEASE_WORKFLOW.contains("matrix.os"));
    assert!(RELEASE_WORKFLOW.contains("needs: verify\n"));
    assert!(!RELEASE_WORKFLOW.contains("needs: verify-windows"));
}

#[test]
fn release_workflow_publishes_the_crates_through_the_one_publisher_script() {
    // S2.1: one script owns the order, the skip, the rate-limit wait and the
    // index poll, so the workflow holds no per-crate list of its own.
    assert!(RELEASE_WORKFLOW.contains("python scripts/publish_crates.py --tag \"$TAG\""));
    assert!(RELEASE_WORKFLOW.contains("environment: crates-io"));
    assert!(RELEASE_WORKFLOW.contains("id-token: write"));
    assert!(RELEASE_WORKFLOW.contains("timeout-minutes: 240"));
    assert!(!RELEASE_WORKFLOW.contains("cargo publish -p"));
}

#[test]
fn release_workflow_prefers_the_first_publication_token_over_trusted_publishing() {
    // D5, S2.2 and S2.3: crates.io accepts the first publication of a new name
    // only with a token, so the auth action is allowed to fail while no
    // trusted publisher exists, and the environment secret wins while it is
    // set. A secret cannot be tested in an `if:`, hence the HAVE_TOKEN string.
    assert!(RELEASE_WORKFLOW.contains("rust-lang/crates-io-auth-action@v1"));
    assert!(RELEASE_WORKFLOW.contains("continue-on-error: true"));
    assert!(RELEASE_WORKFLOW.contains("HAVE_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN != '' }}"));
    assert!(RELEASE_WORKFLOW.contains(
        "CARGO_REGISTRY_TOKEN: ${{ env.HAVE_TOKEN == 'true' && secrets.CARGO_REGISTRY_TOKEN || steps.auth.outputs.token }}"
    ));
}

#[test]
fn checked_artifact_boundary_runs_the_release_and_publish_unit_tests() {
    // S1.5 and S2.1 wiring: this job names its unittest modules one by one, so
    // each new module has to be named here or CI never runs it.
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("scripts/test_release_bump.py"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("scripts/test_publish_crates.py"));
}

#[test]
fn checked_artifact_boundary_runs_before_merge_and_on_main_push() {
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("pull_request:"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("push:"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("branches: [main]"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("check_checked_artifact_boundaries.py"));
    assert!(!CHECKED_ARTIFACT_WORKFLOW.contains("test_check_checked_artifact_boundaries.py"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("test_release_boundary.py"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("check_filesystem_boundary.py"));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains("python-version: \"3.11\""));
    assert!(CHECKED_ARTIFACT_WORKFLOW.contains(
        "CLIPPY_CONF_DIR=\"$PWD/scripts/checks/filesystem_lints\" cargo clippy --no-deps --all-targets --all-features -- -D warnings"
    ));
}

#[test]
fn local_release_runs_checked_artifact_boundary_before_rust_tests() {
    let release = include_str!("../scripts/release.py");
    let boundary = release
        .find("[sys.executable, CHECKED_ARTIFACT_BOUNDARY]")
        .expect("release gate invokes the boundary checker");
    let tests = release
        .find("str(cargo_root / \"scripts\" / \"run_tests.py\")")
        .expect("release gate invokes Rust tests");
    assert!(boundary < tests);
    assert!(!release.contains("CHECKED_ARTIFACT_BOUNDARY_TEST"));
    assert!(!release.contains("RELEASE_BOUNDARY_TEST"));
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
