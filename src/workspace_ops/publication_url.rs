//! The one URL root publication reads to prove a lock dependency.
//!
//! A member's remote URL counts only when it reaches the committed repository
//! ([`same_repository`]), so a deliberately unusable push URL (`DISABLE`, a
//! fork) falls through to the next rule. Selection is pure: the caller supplies
//! the member's remote or the effective scheme.
//! Design: gwz-dev dev-docs/GwzUrlSchemePushPlan.md §3.1 and §3.2. Nothing calls
//! this until that plan's step 2.1, hence the `dead_code` allowances.

use crate::git::{GitRemote, UrlScheme, same_repository};

/// What the workspace holds for one dependency's member.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(super) enum DependencyMember<'a> {
    /// Materialized, with the checkout's remote named by the committed fetch
    /// remote; `None` when no remote has that name.
    Materialized(Option<&'a GitRemote>),
    /// Not materialized, with the operation's effective URL scheme.
    Unmaterialized(UrlScheme),
}

/// The rule of the read-URL order that chose a URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadUrlRule {
    /// 1: the remote's push destination, its `pushurl` or else its `url`.
    PushDestination,
    /// 2: the remote's fetch `url`.
    FetchUrl,
    /// 3: the committed URL derived into the effective scheme, or as committed
    /// when the derivation is refused.
    EffectiveScheme,
    /// 4: the committed URL.
    Committed,
}

/// The URL every read of one dependency uses, and the rule that chose it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) struct ReadUrl {
    pub url: String,
    pub rule: ReadUrlRule,
}

/// Chooses a dependency's read URL; the first rule that applies wins.
#[allow(dead_code)]
pub(super) fn select_read_url(committed: &str, member: DependencyMember<'_>) -> ReadUrl {
    let remote = match member {
        DependencyMember::Materialized(remote) => remote,
        DependencyMember::Unmaterialized(scheme) => {
            // A refused derivation falls back to the committed URL.
            let url = crate::git::derive(committed, scheme).map_or_else(
                |_| committed.to_owned(),
                |resolution| resolution.effective_url,
            );
            return ReadUrl {
                url,
                rule: ReadUrlRule::EffectiveScheme,
            };
        }
    };
    let fetch_url = remote.and_then(|remote| remote.url.as_deref());
    let push_destination = remote
        .and_then(|remote| remote.push_url.as_deref())
        .or(fetch_url);
    for (candidate, rule) in [
        (push_destination, ReadUrlRule::PushDestination),
        (fetch_url, ReadUrlRule::FetchUrl),
    ] {
        if let Some(url) = candidate
            && same_repository(committed, url)
        {
            return ReadUrl {
                url: url.to_owned(),
                rule,
            };
        }
    }
    ReadUrl {
        url: committed.to_owned(),
        rule: ReadUrlRule::Committed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReadUrlRule::{Committed, EffectiveScheme, FetchUrl, PushDestination};

    /// A committed URL as a manifest records it.
    const SSH: &str = "git@github.com:o/r.git";
    /// Its https form: the same repository.
    const HTTPS: &str = "https://github.com/o/r.git";
    /// Another repository on the same host.
    const FORK: &str = "git@github.com:fork/r.git";
    /// A committed URL on a host with no derivable forms.
    const UNKNOWN: &str = "git@example.com:o/r.git";
    /// A known-host URL whose https form is refused.
    const REFUSED: &str = "ssh://git@github.com:2222/o/r.git";

    /// The committed URL, the remote's `url` and `pushurl`, then the read URL
    /// and the rule that must choose it.
    type MaterializedRow = (
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        &'static str,
        ReadUrlRule,
    );

    /// The read order applied to a materialized member. Kept one row per line.
    #[rustfmt::skip]
    const MATERIALIZED: [MaterializedRow; 19] = [
        // Rule 1, including an https fetch URL with an SSH push URL.
        (SSH,     Some(SSH),   None,            SSH,     PushDestination),
        (SSH,     Some(HTTPS), None,            HTTPS,   PushDestination),
        (HTTPS,   Some(SSH),   None,            SSH,     PushDestination),
        (SSH,     Some(HTTPS), Some(SSH),       SSH,     PushDestination),
        (SSH,     Some(FORK),  Some(HTTPS),     HTTPS,   PushDestination),
        (SSH,     None,        Some(HTTPS),     HTTPS,   PushDestination),
        // Rule 2: a fork or `DISABLE` push URL falls through to the fetch URL.
        (SSH,     Some(SSH),   Some(FORK),      SSH,     FetchUrl),
        (SSH,     Some(HTTPS), Some(FORK),      HTTPS,   FetchUrl),
        (SSH,     Some(HTTPS), Some("DISABLE"), HTTPS,   FetchUrl),
        // Rule 4: no URL, or only different repositories.
        (SSH,     None,        None,            SSH,     Committed),
        (SSH,     Some(FORK),  Some("DISABLE"), SSH,     Committed),
        (UNKNOWN, Some("https://example.com/o/r.git"),   None, UNKNOWN, Committed),
        (SSH,     Some("https://github.com/o/r"),        None, SSH,     Committed),
        (SSH,     Some("https://GitHub.com/o/r.git"),    None, SSH,     Committed),
        (SSH,     Some("https://user@github.com/o/r.git"), None, SSH,   Committed),
        (SSH,     Some("https://github.com:443/o/r.git"), None, SSH,    Committed),
        (SSH,     Some("ssh://git@github.com/o/r.git"),  None, SSH,     Committed),
        (SSH,     Some("https://github.com:8443/o/r.git"), None, SSH,   Committed),
        (HTTPS,   Some(REFUSED),                         None, HTTPS,   Committed),
    ];

    /// The committed URL and the effective scheme, then the read URL rule 3
    /// chooses.
    #[rustfmt::skip]
    const UNMATERIALIZED: [(&str, UrlScheme, &str); 8] = [
        (SSH,     UrlScheme::Ssh,      SSH),
        (SSH,     UrlScheme::Https,    HTTPS),
        (SSH,     UrlScheme::Manifest, SSH),
        (HTTPS,   UrlScheme::Ssh,      SSH),
        (HTTPS,   UrlScheme::Https,    HTTPS),
        (HTTPS,   UrlScheme::Manifest, HTTPS),
        (UNKNOWN, UrlScheme::Https,    UNKNOWN),
        (REFUSED, UrlScheme::Https,    REFUSED),
    ];

    #[test]
    fn a_materialized_member_reads_a_remote_url_only_when_it_is_the_same_repository() {
        for (committed, url, push_url, want, rule) in MATERIALIZED {
            let remote = GitRemote {
                name: "origin".to_owned(),
                url: url.map(str::to_owned),
                push_url: push_url.map(str::to_owned),
            };
            let got = select_read_url(committed, DependencyMember::Materialized(Some(&remote)));
            let want = ReadUrl {
                url: want.to_owned(),
                rule,
            };
            assert_eq!(got, want, "{committed} with {remote:?}");
        }
        // The checkout has no remote of the committed fetch remote's name.
        let got = select_read_url(SSH, DependencyMember::Materialized(None));
        let want = ReadUrl {
            url: SSH.to_owned(),
            rule: Committed,
        };
        assert_eq!(got, want);
    }

    #[test]
    fn an_unmaterialized_member_reads_the_committed_url_in_the_effective_scheme() {
        // The last row's derivation is refused, so it reads the committed URL.
        assert!(crate::git::derive(REFUSED, UrlScheme::Https).is_err());
        for (committed, scheme, want) in UNMATERIALIZED {
            let got = select_read_url(committed, DependencyMember::Unmaterialized(scheme));
            let want = ReadUrl {
                url: want.to_owned(),
                rule: EffectiveScheme,
            };
            assert_eq!(got, want, "{committed} @ {scheme:?}");
        }
    }
}
