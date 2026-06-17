    
    
    

    

    
use super::*;

#[test]
    pub(crate) fn new_backend_allows_configured_credential_helpers() {
        assert_eq!(
            Git2Backend::new().credential_helpers,
            CredentialHelperPolicy::AllowConfigured
        );
        assert_eq!(
            Git2Backend::without_credential_helpers().credential_helpers,
            CredentialHelperPolicy::Disabled
        );
    }

    