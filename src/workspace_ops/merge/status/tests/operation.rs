use super::*;

#[test]
fn digest_comparison_reports_change_without_rewriting_the_file() {
    let root = std::env::temp_dir().join(format!("gwz-status-{}", std::process::id()));
    let path = root.join("baseline");
    fs::create_dir_all(&root).unwrap();
    fs::write(&path, b"live").unwrap();
    let mut drift = Vec::new();
    compare_digest(
        &root,
        "baseline",
        "recorded",
        OperationDriftKind::BaselineLockChanged,
        &mut drift,
    );
    assert_eq!(drift[0].kind, OperationDriftKind::BaselineLockChanged);
    assert_eq!(fs::read(&path).unwrap(), b"live");
    fs::remove_dir_all(root).unwrap();
}
