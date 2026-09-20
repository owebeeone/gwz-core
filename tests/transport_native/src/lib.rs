//! Qualification fixtures for a pinned per-remote git2 smart transport extension.
//! This package is never linked into the production GWZ runtime.

pub fn file_url(repo: &git2::Repository) -> String {
    let path = repo.path().to_str().unwrap();
    let encoded = path
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"/:-_.~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    format!("file:///{}", encoded.trim_start_matches('/'))
}
