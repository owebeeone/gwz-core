    
    
    

    

    
use super::*;

#[test]
    pub(crate) fn git_host_parses_scheme_scp_and_local_forms() {
        assert_eq!(
            git_host("https://github.com/o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            git_host("https://github.com:443/o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            git_host("ssh://git@example.org/o/r.git").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            git_host("git@github.com:o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            git_host("github.com:o/r.git").as_deref(),
            Some("github.com")
        );
        // Host is case-insensitive.
        assert_eq!(
            git_host("GitHub.COM:o/r.git").as_deref(),
            Some("github.com")
        );
        // Local / hostless forms.
        assert_eq!(git_host("/tmp/repo.git"), None);
        assert_eq!(git_host("file:///tmp/repo.git"), None);
        assert_eq!(git_host("./relative.git"), None);
        assert_eq!(git_host("C:/work/repo.git"), None);
        assert_eq!(git_host(""), None);
    }

    