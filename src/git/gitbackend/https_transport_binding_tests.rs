use super::*;

#[test]
fn ssh_network_timeout_text_cannot_infer_a_helper_outcome() {
    for message in [
        "No credential helper answered within 0.5 seconds, a fixed bound",
        "SSH authentication needs `git` on PATH:",
    ] {
        let error = git2::Error::new(
            git2::ErrorCode::GenericError,
            git2::ErrorClass::Net,
            message,
        );
        assert!(!credential_helper_timeout(&error));
        assert!(!credential_helper_unavailable(&error));
        assert_eq!(
            crate::git::git_error(error).code,
            crate::model::ErrorCode::GitCommandFailed
        );
    }
}

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
    // Windows asks for its own policies where the others leave the choice to the helper (`None`).
    let (allowed, disabled) = if cfg!(windows) {
        (
            Some(gwz_transport::protocol::AuthPolicy::WindowsConfigured),
            Some(gwz_transport::protocol::AuthPolicy::WindowsDefault),
        )
    } else {
        (None, Some(gwz_transport::protocol::AuthPolicy::Anonymous))
    };
    assert_eq!(
        https_policy_for(crate::git::gitbackend::CredentialHelperPolicy::AllowConfigured),
        allowed
    );
    assert_eq!(
        https_policy_for(crate::git::gitbackend::CredentialHelperPolicy::Disabled),
        disabled
    );
}
