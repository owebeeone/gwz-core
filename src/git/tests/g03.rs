    
    
    

    

    
use super::*;

#[test]
    pub(crate) fn remote_credentials_support_ssh_agent_username_and_default_auth() {
        let ssh = remote_credential(
            "ssh://github.com/example/repo.git",
            Some("git"),
            git2::CredentialType::SSH_KEY,
            CredentialHelperPolicy::Disabled,
            &mut 0u32,
        )
        .unwrap();
        assert!(ssh.has_username());

        let username = remote_credential(
            "ssh://github.com/example/repo.git",
            None,
            git2::CredentialType::USERNAME,
            CredentialHelperPolicy::Disabled,
            &mut 0u32,
        )
        .unwrap();
        assert!(username.has_username());

        remote_credential(
            "https://github.com/example/repo.git",
            None,
            git2::CredentialType::DEFAULT,
            CredentialHelperPolicy::Disabled,
            &mut 0u32,
        )
        .unwrap();
    }

    