//! The words for a Windows sign-in that the Windows provider refused (decision D48; finding S2 of
//! `GwzV110AmendmentS56Scope-ReviewSafety.md`): the first `InitializeSecurityContextW` failing, or a token larger than
//! the limit. The failure is `Protocol` like a worker that broke its contract, so the facts tell them apart: the
//! sign-in began (`Unresolved`, which only Negotiate can be) and nothing was sent. Those words used to say the worker
//! may have been replaced and to reinstall gwz, which points at the wrong cause.
use super::HttpsOpenFailure;

impl HttpsOpenFailure {
    /// The words for a Negotiate sign-in the provider refused before it sent anything, or `None` for any other failure.
    pub(super) fn provider_refusal_words(&self) -> Option<String> {
        use gwz_transport::protocol::{ErrorCode, NativeObservation, NativeScheme};
        let facts = self.failure.facts.as_ref()?;
        let native = facts.native.as_ref()?;
        if self.failure.code != ErrorCode::Protocol
            || native.scheme != NativeScheme::Negotiate
            || native.observation != NativeObservation::Unresolved
            || facts.credential_offered
            || facts.http_status.is_some()
        {
            return None;
        }
        let to = self.host.as_deref().unwrap_or("the server");
        let hint = crate::transport_host::session::native_switch_hint(self.cli_hint, self.windows);
        Some(format!(
            "Windows refused GWZ's sign-in (Negotiate) to {to}, and no credential was sent. This is Windows' answer, not \
             a fault in the files installed with gwz. On a domain-joined computer it can mean Kerberos or NTLM policy \
             (Negotiate has no other mechanism to fall back to when NTLM is blocked and Kerberos has no service \
             principal name for the server or no domain controller to ask), or a sign-in token too large for the header \
             GWZ can send (an account in very many groups). Ask your administrator, or {hint}."
        ))
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests {
    use crate::transport_host::request::https_failure::HttpsOpenFailure;
    use gwz_transport::protocol::{
        AuthMethod, ErrorCode, Facts, Failure, NativeFacts, NativeObservation, NativeScheme,
    };

    fn open(failure: Failure, host: Option<&str>, cli_hint: bool) -> HttpsOpenFailure {
        HttpsOpenFailure {
            ended: None,
            failure,
            anonymous: None,
            attempts: None,
            service: Some(gwz_transport::protocol::GitService::UploadPackAdvertisement),
            helpers_disabled: false,
            cli_hint,
            windows: true,
            host: host.map(str::to_owned),
            redirect: None,
            machine: Default::default(),
        }
    }

    /// The facts of a Negotiate sign-in the provider refused: begun, no mechanism seen, nothing sent.
    fn refused(code: ErrorCode) -> Failure {
        Failure {
            code,
            facts: Some(Facts {
                method: AuthMethod::Sspi,
                native: Some(NativeFacts {
                    scheme: NativeScheme::Negotiate,
                    observation: NativeObservation::Unresolved,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_refused_negotiate_sign_in_names_the_cause_and_the_fallback_not_a_replaced_worker() {
        let said = open(refused(ErrorCode::Protocol), Some("git.example.test"), true).reason();
        assert_eq!(
            said,
            "Windows refused GWZ's sign-in (Negotiate) to git.example.test, and no credential was sent. This is Windows' \
             answer, not a fault in the files installed with gwz. On a domain-joined computer it can mean Kerberos or \
             NTLM policy (Negotiate has no other mechanism to fall back to when NTLM is blocked and Kerberos has no \
             service principal name for the server or no domain controller to ask), or a sign-in token too large for \
             the header GWZ can send (an account in very many groups). Ask your administrator, or run with \
             --transport native to use libgit2's native transport, as gwz 1.0 did."
        );
        assert!(!said.contains("Reinstall"), "{said}");
        assert!(!said.contains("replaced by another program"), "{said}");
        // A host the failure does not know is "the server"; the environment switch where the caller has no flag.
        let said = open(refused(ErrorCode::Protocol), None, false).reason();
        assert!(said.starts_with("Windows refused GWZ's sign-in (Negotiate) to the server, and"), "{said}");
        assert!(said.contains("GWZ_TRANSPORT=native"), "{said}");
    }

    #[test]
    fn the_reinstall_words_stay_for_a_worker_that_never_began() {
        let mut mismatch = refused(ErrorCode::Protocol);
        mismatch.facts.as_mut().unwrap().native.as_mut().unwrap().observation = NativeObservation::NotStarted;
        let said = open(mismatch, Some("git.example.test"), true).reason();
        assert!(said.starts_with("GWZ's Windows sign-in worker did not answer as the one installed with gwz does"), "{said}");
        assert!(said.contains("Reinstall gwz"), "{said}");
    }

    #[test]
    fn a_failure_after_a_token_or_a_status_or_of_another_code_is_not_a_provider_refusal() {
        let mut sent = refused(ErrorCode::Protocol);
        sent.facts.as_mut().unwrap().credential_offered = true;
        assert_eq!(open(sent, None, true).reason(), "HTTPS endpoint request failed: Protocol");
        let mut answered = refused(ErrorCode::Protocol);
        answered.facts.as_mut().unwrap().http_status = Some(401);
        assert_eq!(open(answered, None, true).reason(), "HTTPS endpoint request failed: Protocol");
        for code in [ErrorCode::Timeout, ErrorCode::Unavailable, ErrorCode::Io] {
            let said = open(refused(code), None, true).reason();
            assert!(!said.starts_with("Windows refused"), "{code:?}: {said}");
        }
        // NTLM never carries the mark (the wire refuses it).
        let mut ntlm = refused(ErrorCode::Protocol);
        ntlm.facts.as_mut().unwrap().native.as_mut().unwrap().scheme = NativeScheme::Ntlm;
        assert!(!open(ntlm, None, true).reason().starts_with("Windows refused"));
    }
} } }
