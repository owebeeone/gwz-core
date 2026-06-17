    
    
    

    

    
use super::*;

#[test]
    pub(crate) fn remote_credentials_reject_plaintext_auth_when_helpers_are_disabled() {
        let result = remote_credential(
            "https://github.com/example/repo.git",
            None,
            git2::CredentialType::USER_PASS_PLAINTEXT,
            CredentialHelperPolicy::Disabled,
        );
        let err = match result {
            Ok(_) => panic!("expected disabled credential helpers to reject plaintext auth"),
            Err(err) => err,
        };

        assert!(err.message().contains("could not acquire credentials"));
    }

    