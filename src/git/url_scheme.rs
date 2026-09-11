//! URL-scheme preference for repository clones.
//!
//! A manifest records one URL per repository; a caller may still want the other
//! form of it — ssh where the manifest says https, or the reverse. Both forms
//! are derivable only for the hosts whose URL layout is fixed ([`KnownHost`]),
//! so every other host is passed through untouched, and a known-host URL that
//! cannot express the wanted form is refused rather than guessed at.

use super::git_host;

/// The remedy offered with every refusal.
const REMEDY: &str = "use --url-scheme manifest for this run, or record a remote in the wanted form with `gwz repo sync`";

/// The ssh port a derived URL leaves implicit.
const DEFAULT_SSH_PORT: u16 = 22;

/// The https port a derived URL leaves implicit.
const DEFAULT_HTTPS_PORT: u16 = 443;

/// Which URL form a caller wants for repositories on a known host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum UrlScheme {
    /// Clone exactly what the manifest records; never rewrite, never refuse.
    Manifest,
    /// Prefer the scp-like `git@host:path` form.
    Ssh,
    /// Prefer the `https://host/path` form.
    Https,
}

impl UrlScheme {
    /// The spelling used on the command line and in records.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Ssh => "ssh",
            Self::Https => "https",
        }
    }

    /// Reads a command-line spelling, ignoring surrounding space and case.
    /// `None` for anything that is not one of the three schemes.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        [Self::Manifest, Self::Ssh, Self::Https]
            .into_iter()
            .find(|scheme| scheme.as_str().eq_ignore_ascii_case(text))
    }
}

/// Hosts for which both URL forms are derivable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum KnownHost {
    /// `github.com` itself; subdomains such as `ssh.github.com` are not covered.
    GitHub,
    /// `gitlab.com` itself; a self-hosted GitLab is not covered.
    GitLab,
    /// `bitbucket.org` itself.
    Bitbucket,
}

impl KnownHost {
    /// The one host name this variant covers, lowercase.
    pub fn host(self) -> &'static str {
        match self {
            Self::GitHub => "github.com",
            Self::GitLab => "gitlab.com",
            Self::Bitbucket => "bitbucket.org",
        }
    }

    /// Recognises a host name, case-insensitively; subdomains are not known.
    fn from_host(host: &str) -> Option<Self> {
        [Self::GitHub, Self::GitLab, Self::Bitbucket]
            .into_iter()
            .find(|known| known.host().eq_ignore_ascii_case(host))
    }
}

/// Outcome of applying a scheme preference to one URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UrlResolution {
    /// The input, verbatim.
    pub manifest_url: String,
    /// What to clone from.
    pub effective_url: String,
    /// The scheme that was requested.
    pub scheme: UrlScheme,
    /// True only when `effective_url` differs from `manifest_url`.
    pub derived: bool,
    /// True when the host is one of [`KnownHost`].
    pub host_known: bool,
}

/// A requested scheme cannot be produced for a known-host URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UrlSchemeRefusal {
    /// The URL that was asked about, verbatim.
    pub url: String,
    /// The scheme that could not be produced.
    pub scheme: UrlScheme,
    /// The specific cause, naming the port, the URL scheme or the missing path.
    pub reason: String,
    /// What the operator can do instead.
    pub remedy: String,
}

impl std::fmt::Display for UrlSchemeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot derive {} form for {}: {}; {}",
            self.scheme.as_str(),
            self.url,
            self.reason,
            self.remedy
        )
    }
}

impl std::error::Error for UrlSchemeRefusal {}

/// Recognises the host of a git URL when both URL forms are derivable for it.
/// Accepts the same URL forms as `git_host`, and answers `None` for every other
/// host, for local paths and for strings with no host at all.
pub fn known_host(url: &str) -> Option<KnownHost> {
    parse_git_url(url).and_then(|parsed| KnownHost::from_host(&parsed.host))
}

/// Applies a scheme preference to one manifest URL.
///
/// [`UrlScheme::Manifest`], an unknown host, and a URL already in the requested
/// form all yield the input verbatim with `derived` false. A known-host URL that
/// cannot be expressed in the requested form — a nonstandard port, a transport
/// other than ssh/https, an empty repository path — is refused.
pub fn derive(url: &str, scheme: UrlScheme) -> Result<UrlResolution, UrlSchemeRefusal> {
    let parsed = parse_git_url(url);
    let host_known = parsed
        .as_ref()
        .is_some_and(|parsed| KnownHost::from_host(&parsed.host).is_some());
    let mut effective = url.to_string();
    if let Some(parsed) = &parsed
        && host_known
        && let Some(target) = Target::of(scheme)
    {
        effective = derive_known(url, parsed, target)?;
    }
    Ok(UrlResolution {
        manifest_url: url.to_string(),
        derived: effective != url,
        effective_url: effective,
        scheme,
        host_known,
    })
}

/// True when `observed_url` is exactly what `derive(manifest_url, <scheme of
/// observed>)` would produce and the two differ: the same repository on a known
/// host, reached through the other scheme. An unknown host, a refusal, a
/// different repository and two identical strings are all false.
pub fn scheme_only_difference(manifest_url: &str, observed_url: &str) -> bool {
    if manifest_url == observed_url || known_host(manifest_url).is_none() {
        return false;
    }
    let Some(observed) = parse_git_url(observed_url) else {
        return false;
    };
    let observed_form = match observed.form {
        Form::Scp | Form::Ssh => UrlScheme::Ssh,
        Form::Https => UrlScheme::Https,
        Form::Other(_) => {
            return false;
        }
    };
    derive(manifest_url, observed_form)
        .is_ok_and(|resolution| resolution.effective_url == observed_url)
}

/// True for the two ssh forms, scp-like and `ssh://`, on any host.
pub fn uses_ssh(url: &str) -> bool {
    parse_git_url(url).is_some_and(|parsed| matches!(parsed.form, Form::Scp | Form::Ssh))
}

/// The two derivable forms: [`UrlScheme`] without its no-op `Manifest`.
#[derive(Clone, Copy)]
enum Target {
    /// The scp form, `git@host:path`.
    Ssh,
    /// The https form, `https://host/path`.
    Https,
}

impl Target {
    /// The derivable counterpart of a requested scheme; `None` for `Manifest`.
    fn of(scheme: UrlScheme) -> Option<Self> {
        match scheme {
            UrlScheme::Manifest => None,
            UrlScheme::Ssh => Some(Self::Ssh),
            UrlScheme::Https => Some(Self::Https),
        }
    }

    /// The public scheme this target answers, for refusal records.
    fn scheme(self) -> UrlScheme {
        match self {
            Self::Ssh => UrlScheme::Ssh,
            Self::Https => UrlScheme::Https,
        }
    }
}

/// The recognised URL forms.
enum Form {
    /// scp-like `[user@]host:path`, which carries no port.
    Scp,
    /// `ssh://[user@]host[:port]/path`.
    Ssh,
    /// `https://[user[:password]@]host[:port]/path`.
    Https,
    /// Any other URL scheme, which is derivable in neither direction.
    Other(String),
}

/// One git URL split into the parts a derivation needs.
struct Parsed {
    /// The URL form.
    form: Form,
    /// Host name, lowercase.
    host: String,
    /// The explicit port, when the form carries one.
    port: Option<u16>,
    /// Repository path with no leading or trailing `/`.
    path: String,
}

/// Splits a git URL that has a host; `None` for local paths and for strings with
/// no host. The host comes from `git_host`, so the two agree exactly on which
/// strings have one.
fn parse_git_url(url: &str) -> Option<Parsed> {
    let url = url.trim();
    let host = git_host(url)?;
    if url.contains("://") {
        let parsed = url::Url::parse(url).ok()?;
        let form = match parsed.scheme() {
            "ssh" => Form::Ssh,
            "https" => Form::Https,
            other => Form::Other(other.to_string()),
        };
        return Some(Parsed {
            form,
            host,
            port: parsed.port(),
            path: repo_path(parsed.path()),
        });
    }
    // scp-like: everything after the first colon is the path, ports and all.
    let colon = url.find(':')?;
    Some(Parsed {
        form: Form::Scp,
        host,
        port: None,
        path: repo_path(&url[colon + 1..]),
    })
}

/// Normalises a repository path: one leading `/` and every trailing `/` are
/// dropped; case, the `.git` suffix, nested groups and percent-encoding are all
/// kept as written.
fn repo_path(raw: &str) -> String {
    raw.strip_prefix('/')
        .unwrap_or(raw)
        .trim_end_matches('/')
        .to_string()
}

/// Rewrites a known-host URL into `target`'s form, or refuses.
fn derive_known(url: &str, parsed: &Parsed, target: Target) -> Result<String, UrlSchemeRefusal> {
    if parsed.path.is_empty() {
        return Err(refuse(url, target, "empty repository path".to_string()));
    }
    match (&parsed.form, target) {
        (Form::Scp, Target::Ssh) | (Form::Https, Target::Https) => Ok(url.to_string()),
        (Form::Scp, Target::Https) => Ok(to_https(parsed)),
        (Form::Ssh, Target::Https) => {
            check_port(url, target, parsed.port, DEFAULT_SSH_PORT, "SSH")?;
            Ok(to_https(parsed))
        }
        (Form::Https, Target::Ssh) => {
            check_port(url, target, parsed.port, DEFAULT_HTTPS_PORT, "HTTPS")?;
            Ok(to_ssh(parsed))
        }
        // An `ssh://` URL on a nonstandard port cannot become the scp form
        // without silently losing the port, so it is left as written.
        (Form::Ssh, Target::Ssh) => match parsed.port {
            Some(port) if port != DEFAULT_SSH_PORT => Ok(url.to_string()),
            _ => Ok(to_ssh(parsed)),
        },
        (Form::Other(other), _) => Err(refuse(
            url,
            target,
            format!("unsupported URL scheme {other}"),
        )),
    }
}

/// Refuses when the source URL names a port other than its transport's default;
/// such a port has no place to go in the other form.
fn check_port(
    url: &str,
    target: Target,
    port: Option<u16>,
    default: u16,
    transport: &str,
) -> Result<(), UrlSchemeRefusal> {
    match port {
        Some(port) if port != default => Err(refuse(
            url,
            target,
            format!("nonstandard {transport} port {port}"),
        )),
        _ => Ok(()),
    }
}

/// The https form of a parsed known-host URL.
fn to_https(parsed: &Parsed) -> String {
    format!("https://{}/{}", parsed.host, parsed.path)
}

/// The scp form of a parsed known-host URL, always with the `git` user.
fn to_ssh(parsed: &Parsed) -> String {
    format!("git@{}:{}", parsed.host, parsed.path)
}

/// Builds a refusal for one URL, with the fixed remedy sentence.
fn refuse(url: &str, target: Target, reason: String) -> UrlSchemeRefusal {
    UrlSchemeRefusal {
        url: url.to_string(),
        scheme: target.scheme(),
        reason,
        remedy: REMEDY.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Expect::{Refuse, Same, To};

    /// Every host both URL forms are derivable for.
    const KNOWN: [&str; 3] = ["github.com", "gitlab.com", "bitbucket.org"];

    /// What one table row expects of one derivation.
    enum Expect {
        /// The input comes back verbatim, `derived` false.
        Same,
        /// The input derives to this template.
        To(&'static str),
        /// The derivation is refused, with this substring in `reason`.
        Refuse(&'static str),
    }

    /// Input template, then the expectation for `Https` and for `Ssh`, with
    /// `HOST` standing in for each known host. Kept one row per line: it is the
    /// derivation table the module is specified by.
    #[rustfmt::skip]
    const ROWS: [(&str, Expect, Expect); 16] = [
        ("git@HOST:owner/repo.git",     To("https://HOST/owner/repo.git"),     Same),
        ("git@HOST:owner/repo",         To("https://HOST/owner/repo"),         Same),
        ("git@HOST:group/sub/repo.git", To("https://HOST/group/sub/repo.git"), Same),
        ("ssh://git@HOST/o/r.git",      To("https://HOST/o/r.git"),            To("git@HOST:o/r.git")),
        ("ssh://git@HOST:22/o/r.git",   To("https://HOST/o/r.git"),            To("git@HOST:o/r.git")),
        ("https://HOST/o/r.git",        Same,                                  To("git@HOST:o/r.git")),
        ("https://HOST/o/r",            Same,                                  To("git@HOST:o/r")),
        ("https://user@HOST/o/r.git",   Same,                                  To("git@HOST:o/r.git")),
        ("https://HOST:443/o/r.git",    Same,                                  To("git@HOST:o/r.git")),
        ("ssh://git@HOST:2222/o/r.git", Refuse("2222"),                        Same),
        ("https://HOST:8443/o/r.git",   Same,                                  Refuse("8443")),
        ("http://HOST/o/r.git",         Refuse("http"),                        Refuse("http")),
        ("git://HOST/o/r.git",          Refuse("git"),                         Refuse("git")),
        ("git@HOST:",                   Refuse("empty"),                       Refuse("empty")),
        ("https://HOST/",               Refuse("empty"),                       Refuse("empty")),
        ("https://HOST",                Refuse("empty"),                       Refuse("empty")),
    ];

    /// Asserts one table cell.
    fn check(input: &str, scheme: UrlScheme, expect: &Expect, host: &str) {
        match expect {
            Same => {
                let got = derive(input, scheme).expect("a known-host URL in the wanted form");
                assert_eq!(got.effective_url, input, "{input} @ {scheme:?}");
                assert_eq!(got.manifest_url, input);
                assert_eq!(got.scheme, scheme);
                assert!(!got.derived, "{input} @ {scheme:?}");
                assert!(got.host_known, "{input}");
            }
            To(template) => {
                let got = derive(input, scheme).expect("a derivable known-host URL");
                let want = template.replace("HOST", host);
                assert_eq!(got.effective_url, want, "{input} @ {scheme:?}");
                assert_eq!(got.manifest_url, input);
                assert_eq!(got.scheme, scheme);
                assert!(got.derived, "{input} @ {scheme:?}");
                assert!(got.host_known, "{input}");
            }
            Refuse(needle) => {
                let got = derive(input, scheme).expect_err("a refusal");
                assert_eq!(got.scheme, scheme);
                assert_eq!(got.url, input);
                assert!(got.reason.contains(needle), "{} lacks {needle}", got.reason);
                let shown = got.to_string();
                assert!(shown.contains(input), "{shown}");
                assert!(shown.contains(REMEDY), "{shown}");
                assert!(shown.contains(scheme.as_str()), "{shown}");
            }
        }
    }

    /// Asserts that `input` derives to `want` under `scheme`, rewritten.
    fn derives(input: &str, scheme: UrlScheme, want: &str) {
        let got = derive(input, scheme).expect("a derivable known-host URL");
        assert_eq!(got.effective_url, want, "{input} @ {scheme:?}");
        assert!(got.derived, "{input} @ {scheme:?}");
    }

    /// Asserts that `input` derives to the https form `want`.
    fn https_of(input: &str, want: &str) {
        derives(input, UrlScheme::Https, want);
    }

    /// Asserts that `input` derives to the scp form `want`.
    fn ssh_of(input: &str, want: &str) {
        derives(input, UrlScheme::Ssh, want);
    }

    #[test]
    fn table_holds_for_every_known_host_both_ways() {
        for host in KNOWN {
            for (template, https, ssh) in &ROWS {
                let input = template.replace("HOST", host);
                let known = known_host(&input).map(KnownHost::host);
                assert_eq!(known, Some(host), "{input}");
                check(&input, UrlScheme::Https, https, host);
                check(&input, UrlScheme::Ssh, ssh, host);
            }
        }
    }

    #[test]
    fn known_host_agrees_with_git_host() {
        for host in KNOWN {
            for (template, _, _) in &ROWS {
                let input = template.replace("HOST", host);
                let upper = input.replace(host, &host.to_ascii_uppercase());
                for url in [input.as_str(), upper.as_str()] {
                    let known = known_host(url).map(KnownHost::host);
                    assert_eq!(known, git_host(url).as_deref(), "{url}");
                }
            }
        }
    }

    #[test]
    fn manifest_scheme_never_rewrites_and_never_refuses() {
        for (input, known) in [
            ("git@github.com:owner/repo.git", true),
            ("https://gitlab.com/owner/repo.git", true),
            ("https://gitlab.com/", true),
            ("http://bitbucket.org/o/r.git", true),
            ("ssh://git@github.com:2222/o/r.git", true),
            ("/Users/x/repo", false),
            ("git@example.com:o/r.git", false),
        ] {
            let got = derive(input, UrlScheme::Manifest).expect("manifest never refuses");
            assert_eq!(got.effective_url, input, "{input}");
            assert_eq!(got.manifest_url, input);
            assert_eq!(got.scheme, UrlScheme::Manifest);
            assert!(!got.derived, "{input}");
            assert_eq!(got.host_known, known, "{input}");
        }
    }

    #[test]
    fn unknown_and_hostless_urls_pass_through() {
        const PASSTHROUGH: [&str; 13] = [
            "/Users/x/repo",
            "../rel",
            "C:\\repo",
            "C:/repo",
            "file:///x/y",
            "git@example.com:o/r.git",
            "https://gitlab.example.com/o/r.git",
            "ssh://git@ssh.github.com:443/o/r.git",
            "https://www.github.com/o/r.git",
            "",
            "github.com",
            "git@github.com",
            "owner/repo",
        ];
        for input in PASSTHROUGH {
            assert_eq!(known_host(input), None, "{input}");
            for scheme in [UrlScheme::Manifest, UrlScheme::Ssh, UrlScheme::Https] {
                let got = derive(input, scheme).expect("passthrough never refuses");
                assert_eq!(got.effective_url, input, "{input} @ {scheme:?}");
                assert_eq!(got.manifest_url, input);
                assert!(!got.derived, "{input} @ {scheme:?}");
                assert!(!got.host_known, "{input}");
            }
        }
    }

    #[test]
    fn host_case_users_and_path_shape_are_normalised() {
        assert_eq!(known_host("git@GITHUB.COM:o/r"), Some(KnownHost::GitHub));
        // Already the scp form: returned exactly as written, uppercase kept.
        let same = derive("git@GitHub.com:O/R.git", UrlScheme::Ssh).expect("same");
        assert_eq!(same.effective_url, "git@GitHub.com:O/R.git");
        assert!(!same.derived);
        // An uppercase host derives lowercase; the path keeps its case.
        https_of("git@GitHub.com:O/R.git", "https://github.com/O/R.git");
        // One leading slash and every trailing slash go; the rest is verbatim.
        https_of("git@github.com:/o/r", "https://github.com/o/r");
        https_of("git@github.com:o/r/", "https://github.com/o/r");
        ssh_of("https://github.com/o/r/", "git@github.com:o/r");
        ssh_of("https://github.com/o/r%20x", "git@github.com:o/r%20x");
        // Any user, and a default port, are dropped; ssh output is always `git`.
        ssh_of("https://u:pw@github.com/o/r.git", "git@github.com:o/r.git");
        ssh_of("ssh://deploy@github.com/o/r.git", "git@github.com:o/r.git");
        https_of("ssh://d@gitlab.com:22/g/s/r", "https://gitlab.com/g/s/r");
    }

    #[test]
    fn scheme_spellings_parse_and_round_trip() {
        assert_eq!(UrlScheme::parse("ssh"), Some(UrlScheme::Ssh));
        assert_eq!(UrlScheme::parse(" HTTPS "), Some(UrlScheme::Https));
        assert_eq!(UrlScheme::parse("Manifest"), Some(UrlScheme::Manifest));
        assert_eq!(UrlScheme::parse("auto"), None);
        assert_eq!(UrlScheme::parse(""), None);
        for scheme in [UrlScheme::Manifest, UrlScheme::Ssh, UrlScheme::Https] {
            assert_eq!(UrlScheme::parse(scheme.as_str()), Some(scheme));
        }
        assert_eq!(UrlScheme::Manifest.as_str(), "manifest");
        assert_eq!(UrlScheme::Ssh.as_str(), "ssh");
        assert_eq!(UrlScheme::Https.as_str(), "https");
    }

    #[test]
    fn scheme_only_difference_spots_the_other_form() {
        for host in KNOWN {
            for (ssh, https) in [
                (
                    format!("git@{host}:o/r.git"),
                    format!("https://{host}/o/r.git"),
                ),
                (
                    format!("git@{host}:owner/repo"),
                    format!("https://{host}/owner/repo"),
                ),
            ] {
                assert!(scheme_only_difference(&ssh, &https), "{ssh} -> {https}");
                assert!(scheme_only_difference(&https, &ssh), "{https} -> {ssh}");
                assert!(!scheme_only_difference(&ssh, &ssh), "{ssh}");
                assert!(!scheme_only_difference(&https, &https), "{https}");
            }
            // An `ssh://` manifest URL reaches the same repository as the scp form.
            let scheme_url = format!("ssh://git@{host}/o/r.git");
            let scp = format!("git@{host}:o/r.git");
            assert!(scheme_only_difference(&scheme_url, &scp), "{scheme_url}");
        }
        for (manifest, observed) in [
            // Unknown host.
            ("git@example.com:o/r.git", "https://example.com/o/r.git"),
            // A different repository.
            ("git@github.com:o/r.git", "https://github.com/o/other.git"),
            // The `.git` suffix differs.
            ("git@github.com:o/r.git", "https://github.com/o/r"),
            // The observed form is one that is refused outright.
            ("https://github.com/o/r.git", "http://github.com/o/r.git"),
            // A nonstandard port is not a scheme-only difference.
            (
                "https://github.com/o/r.git",
                "ssh://git@github.com:2222/o/r.git",
            ),
            // No host at all.
            ("https://github.com/o/r.git", "/Users/x/repo"),
            // `ssh://` is not what the ssh derivation produces.
            ("https://github.com/o/r.git", "ssh://git@github.com/o/r.git"),
        ] {
            let shown = format!("{manifest} vs {observed}");
            assert!(!scheme_only_difference(manifest, observed), "{shown}");
        }
    }
}
