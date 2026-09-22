use super::*;

#[test]
fn candidate_binding_recognizes_only_https_as_the_host_http_route() {
    assert!(is_https_remote("https://example.invalid/owner/repo"));
    assert!(is_https_remote("HTTPS://example.invalid/owner/repo"));
    assert!(!is_https_remote("ssh://git@example.invalid/owner/repo"));
    assert!(!is_https_remote("git@example.invalid:owner/repo"));
    assert!(!is_https_remote("file:///tmp/repo"));
    assert!(
        is_https_remote("https://"),
        "invalid HTTPS must reach host validation, never native fallback"
    );
}

#[test]
fn candidate_binding_maps_helper_availability_to_https_policy() {
    assert_eq!(
        https_policy_for(crate::git::gitbackend::CredentialHelperPolicy::AllowConfigured),
        None
    );
    assert_eq!(
        https_policy_for(crate::git::gitbackend::CredentialHelperPolicy::Disabled),
        Some(gwz_transport::protocol::AuthPolicy::Anonymous)
    );
}
