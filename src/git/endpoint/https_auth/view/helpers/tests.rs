//! The helper view against Git's own selection of `credential.<url>.*` sections.
use super::*;
use crate::git::endpoint::https_auth::SecretBuffer;

fn entry(name: &str, value: Option<&str>) -> Entry {
    Entry {
        name: SecretBuffer(name.as_bytes().to_vec()),
        value: value.map(|value| SecretBuffer(value.as_bytes().to_vec())),
    }
}
fn held(entries: &[(&str, Option<&str>)], url: &str) -> bool {
    let entries: Vec<_> = entries
        .iter()
        .map(|(name, value)| entry(name, *value))
        .collect();
    in_effect(&entries, url.as_bytes())
}
/// Whether a helper written under `credential.<pattern>` applies to `url`.
fn scoped(pattern: &str, url: &str) -> bool {
    held(
        &[(&format!("credential.{pattern}.helper"), Some("store"))],
        url,
    )
}
const URL: &str = "https://example.test/repo";

#[test]
fn no_helper_entry_means_no_helper() {
    assert!(!held(&[], URL));
    assert!(!held(
        &[
            ("core.askpass", Some("")),
            ("credential.https://example.test.username", Some("me")),
            ("credential.usehttppath", Some("true")),
        ],
        URL
    ));
}

#[test]
fn a_global_helper_applies_and_an_empty_value_clears_what_came_before_it() {
    assert!(held(&[("credential.helper", Some("store"))], URL));
    assert!(!held(
        &[
            ("credential.helper", Some("store")),
            ("credential.helper", Some(""))
        ],
        URL
    ));
    assert!(held(
        &[
            ("credential.helper", Some("store")),
            ("credential.helper", Some("")),
            ("credential.helper", Some("manager")),
        ],
        URL
    ));
}

#[test]
fn a_helper_scoped_to_another_server_is_not_a_helper_for_this_one() {
    // What `gh auth setup-git` writes: a reset, then a helper for github.com alone.
    let github = [
        ("credential.https://github.com.helper", Some("")),
        (
            "credential.https://github.com.helper",
            Some("!gh auth git-credential"),
        ),
    ];
    assert!(!held(&github, URL));
    assert!(held(&github, "https://github.com/team/repo"));
    assert!(!scoped("https://example.test", "http://example.test/repo"));
}

#[test]
fn a_scoped_reset_clears_the_global_helper_for_its_server_only() {
    let entries = [
        ("credential.helper", Some("store")),
        ("credential.https://example.test.helper", Some("")),
    ];
    assert!(!held(&entries, URL));
    assert!(held(&entries, "https://elsewhere.test/repo"));
}

#[test]
fn a_pattern_matches_on_every_part_it_names() {
    assert!(scoped("https://example.test", URL));
    assert!(scoped("https://example.test/repo", URL));
    assert!(!scoped("https://example.test/other", URL));
    assert!(!scoped("https://example.test:8443", URL));
    assert!(scoped(
        "https://example.test:8443",
        "https://example.test:8443/repo"
    ));
    assert!(!scoped("https://me@example.test", URL));
    assert!(scoped(
        "https://me@example.test",
        "https://me@example.test/repo"
    ));
    assert!(scoped(
        "https://me@example.test",
        "https://me:secret@example.test/repo"
    ));
    assert!(!scoped(
        "https://me@example.test",
        "https://you@example.test/repo"
    ));
}

/// State review P2-1: the section name is matched as Git's `urlmatch` matches it, not byte for byte.
#[test]
fn a_pattern_is_matched_as_git_normalizes_urls() {
    // Case: the scheme and the host are lowercase in Git; this row was once asserted false.
    assert!(scoped("https://Example.test", URL));
    assert!(scoped("https://Example.Test", URL));
    assert!(scoped("HTTPS://example.test", URL));
    assert!(scoped("https://example.test", "https://EXAMPLE.test/repo"));
    // The scheme's default port is the same as none.
    assert!(scoped("https://example.test:443", URL));
    assert!(scoped(
        "https://example.test",
        "https://example.test:443/repo"
    ));
    assert!(scoped("http://example.test:80", "http://example.test/repo"));
    assert!(!scoped("https://example.test:80", URL));
    assert!(!scoped(
        "http://example.test:443",
        "http://example.test/repo"
    ));
    // A wildcard is one whole label.
    assert!(scoped(
        "https://*.example.test",
        "https://git.example.test/r"
    ));
    assert!(scoped("https://git.*.test", "https://git.example.test/r"));
    assert!(!scoped(
        "https://*.example.test",
        "https://a.b.example.test/r"
    ));
    assert!(!scoped("https://*.example.test", URL));
    assert!(!scoped("https://exa*.test", URL));
    // A path is a prefix at a `/`: an Azure DevOps organization covers its projects.
    let azure = "https://dev.azure.com/org/proj/_git/r";
    assert!(scoped("https://dev.azure.com/org", azure));
    assert!(scoped("https://dev.azure.com/org/", azure));
    assert!(scoped("https://dev.azure.com/org/proj", azure));
    assert!(scoped("https://dev.azure.com", azure));
    assert!(!scoped("https://dev.azure.com/or", azure));
    assert!(!scoped("https://dev.azure.com/org/pro", azure));
    assert!(!scoped("https://dev.azure.com/org/other", azure));
    assert!(!scoped("https://dev.azure.com/Org", azure));
    // Different port, user or path still do not match.
    assert!(!scoped("https://example.test:8443", URL));
    assert!(!scoped("https://me@example.test", URL));
    assert!(!scoped("https://example.test/repo/sub", URL));
}

/// A section name that is not a URL Git can normalize is a partial one, matched part by part and byte for byte
/// (`match_partial_url`). These were asserted to be ignored; Git applies them.
#[test]
fn a_pattern_that_is_not_a_url_is_a_partial_one() {
    assert!(scoped("example.test", URL));
    assert!(scoped("example.test/repo", URL));
    assert!(!scoped("example.test/other", URL));
    assert!(!scoped("Example.test", URL));
    assert!(scoped("https://", URL));
    assert!(!scoped("http://", URL));
    // The empty section name names nothing, so it names everything.
    assert!(scoped("", URL));
    assert!(held(&[("credential..helper", Some("store"))], URL));
    assert!(!scoped("x", URL));
}

#[test]
fn what_git_ignores_is_not_a_helper() {
    // A helper key with no value at all, and a request that is no URL.
    assert!(!held(&[("credential.helper", None)], URL));
    assert!(!held(&[("credential.helper", Some("store"))], "not a url"));
    assert!(!held(&[("core.helper", Some("store"))], URL));
}

/// A request Git cannot normalize is matched by no section, but the unscoped helper still applies to it.
#[test]
fn a_request_git_cannot_normalize_gets_only_the_unscoped_helper() {
    let url = "https://exa_mple.test/repo";
    assert!(held(&[("credential.helper", Some("store"))], url));
    assert!(!scoped("https://exa_mple.test", url));
    assert!(!scoped("exa_mple.test", url));
}

/// Git 2.52.0's choice for each row: `git -c credential.<pattern>.helper=<helper that answers> credential fill`
/// with `url=<url>`, and whether it used the helper. The same table is run against Git for Windows on dabeest.
const GIT_SELECTS: &[(&str, &str, bool)] = &[
    ("https://example.test", "https://example.test/repo", true),
    ("https://Example.Test", "https://example.test/repo", true),
    ("https://example.test", "https://EXAMPLE.test/repo", true),
    ("HTTPS://example.test", "https://example.test/repo", true),
    (
        "https://example.test:443",
        "https://example.test/repo",
        true,
    ),
    (
        "https://example.test",
        "https://example.test:443/repo",
        true,
    ),
    (
        "https://example.test:443",
        "https://example.test:443/repo",
        true,
    ),
    (
        "https://example.test:8443",
        "https://example.test/repo",
        false,
    ),
    (
        "https://example.test",
        "https://example.test:8443/repo",
        false,
    ),
    ("http://example.test:80", "http://example.test/repo", true),
    ("http://example.test:443", "http://example.test/repo", false),
    (
        "https://example.test:80",
        "https://example.test/repo",
        false,
    ),
    ("https://*.example.test", "https://git.example.test/r", true),
    (
        "https://*.example.test",
        "https://a.b.example.test/r",
        false,
    ),
    ("https://*.example.test", "https://example.test/r", false),
    ("https://git.*.test", "https://git.example.test/r", true),
    ("https://*.test", "https://git.example.test/r", false),
    (
        "https://*.example.test",
        "https://git.example.test:8443/r",
        false,
    ),
    (
        "https://*.example.test:8443",
        "https://git.example.test:8443/r",
        true,
    ),
    ("https://*", "https://example.test/r", false),
    ("https://*.*", "https://example.test/r", true),
    ("https://exa*.test", "https://example.test/r", false),
    (
        "https://example.test/org",
        "https://example.test/org/proj/_git/r",
        true,
    ),
    ("https://example.test/org", "https://example.test/org", true),
    (
        "https://example.test/org",
        "https://example.test/organ/x",
        false,
    ),
    (
        "https://example.test/org/",
        "https://example.test/org/x",
        true,
    ),
    (
        "https://example.test/org/",
        "https://example.test/org",
        true,
    ),
    (
        "https://example.test/org/proj",
        "https://example.test/org",
        false,
    ),
    ("https://example.test/", "https://example.test/org", true),
    (
        "https://example.test/Org",
        "https://example.test/org/x",
        false,
    ),
    (
        "https://example.test/a%2Fb",
        "https://example.test/a%2fb/x",
        false,
    ),
    ("https://example.test/%61", "https://example.test/a/x", true),
    ("https://me@example.test", "https://example.test/r", false),
    ("https://me@example.test", "https://me@example.test/r", true),
    (
        "https://me@example.test",
        "https://you@example.test/r",
        false,
    ),
    (
        "https://me@example.test",
        "https://me:pw@example.test/r",
        true,
    ),
    ("https://example.test", "https://me@example.test/r", true),
    (
        "https://Me@example.test",
        "https://me@example.test/r",
        false,
    ),
    ("example.test", "https://example.test/r", true),
    ("example.test", "https://Example.test/r", false),
    ("example.test/org", "https://example.test/org/x", false),
    ("example.test/org", "https://example.test/org", true),
    ("me@example.test", "https://me@example.test/r", true),
    ("example.test:8443", "https://example.test:8443/r", true),
    ("*.example.test", "https://git.example.test/r", false),
    ("https://", "https://example.test/r", true),
    ("https://.", "https://example.test/r", false),
    ("https://example.test?", "https://example.test/r", false),
    ("//example.test", "https://example.test/r", false),
    ("://example.test", "https://example.test/r", true),
    ("https://example.test/.", "https://example.test/r", true),
    (
        "https://example.test/a/./b",
        "https://example.test/a/b/c",
        true,
    ),
    (
        "https://example.test/a/../org",
        "https://example.test/org/x",
        true,
    ),
    ("https://example.test#frag", "https://example.test/r", false),
    (
        "https://example.test/org?x",
        "https://example.test/org/x",
        false,
    ),
    (
        "https://user:pw@example.test",
        "https://user@example.test/r",
        true,
    ),
    ("https://example.test.:443", "https://example.test/r", true),
    ("https://example.test.", "https://example.test/r", true),
    ("https://example..test", "https://example.test/r", false),
    ("https://[::1]", "https://[::1]/r", true),
    ("https://[ABCD::1]", "https://[abcd::1]/r", true),
    ("https://[abcd::1]", "https://[ABCD::1]/r", true),
    ("https://[abcd::1]:443", "https://[abcd::1]/r", true),
    ("https://[::1]:8443", "https://[::1]:8443/r", true),
    ("https://127.0.0.1", "https://127.0.0.1/r", true),
    ("https://127.*.0.1", "https://127.0.0.1/r", true),
    ("ssh://git@example.test", "ssh://git@example.test/r", true),
    ("ssh://example.test:22", "ssh://example.test/r", false),
    ("git://example.test:9418", "git://example.test/r", false),
    (
        "https://example.test/a%20b",
        "https://example.test/a%20b/x",
        true,
    ),
    (
        "https://example.test/a%20b",
        "https://example.test/a b/x",
        true,
    ),
    (
        "https://example.test/a%2fb",
        "https://example.test/a%2Fb/x",
        false,
    ),
    (
        "https://example.test/a%2Fb",
        "https://example.test/a%2Fb/x",
        false,
    ),
    (
        "https://example.test/a%7eb",
        "https://example.test/a~b",
        true,
    ),
    (
        "https://example.test/a~b",
        "https://example.test/a%7Eb",
        true,
    ),
    (
        "https://example.test/%e9",
        "https://example.test/%E9/x",
        true,
    ),
    ("https://example.test/%zz", "https://example.test/%zz", true),
    (
        "https://example.test/a//b",
        "https://example.test/a/b",
        false,
    ),
    (
        "https://example.test/a/b",
        "https://example.test/a//b",
        false,
    ),
    (
        "https://example.test/a/b/",
        "https://example.test/a/b",
        true,
    ),
    (
        "https://example.test/a/b",
        "https://example.test/a/b/",
        true,
    ),
    (
        "https://example.test/a/b",
        "https://example.test/a/b/../b/c",
        true,
    ),
    (
        "https://example.test",
        "https://example.test/a/../../x",
        false,
    ),
    ("https://example.test", "https://example.test:443/r", true),
    ("https://example.test", "http://example.test/r", false),
    ("http://example.test", "https://example.test/r", false),
    ("https://example.test:0443", "https://example.test/r", true),
    ("https://example.test:", "https://example.test/r", true),
    (
        "https://example.test:abc",
        "https://example.test:abc/r",
        true,
    ),
    (
        "https://example.test:65536",
        "https://example.test/r",
        false,
    ),
    ("https://example.test", "https://example.test./r", true),
    ("https://example.test", "https://EXAMPLE.TEST./r", true),
    ("https://exa_mple.test", "https://exa_mple.test/r", false),
    ("https://@example.test", "https://example.test/r", false),
    (
        "https://me:@example.test",
        "https://me@example.test/r",
        true,
    ),
    ("https://:pw@example.test", "https://example.test/r", false),
    (
        "https://me@*.example.test",
        "https://me@git.example.test/r",
        true,
    ),
    (
        "https://*.example.test/org",
        "https://git.example.test/org/x",
        true,
    ),
    (
        "https://git.example.test/*",
        "https://git.example.test/org",
        false,
    ),
    (
        "https://git.example.test/org/*/x",
        "https://git.example.test/org/y/x",
        false,
    ),
    ("https://*@example.test", "https://me@example.test/r", false),
    ("https://*.example.test", "https://GIT.example.test/r", true),
    ("https://*.EXAMPLE.test", "https://git.example.test/r", true),
    (
        "https://*.example.test.",
        "https://git.example.test/r",
        true,
    ),
    ("https://.example.test", "https://example.test/r", false),
    (
        "https://*..example.test",
        "https://git.example.test/r",
        false,
    ),
    ("https://**", "https://a.b/r", false),
    ("https://*.*.*", "https://a.b.c/r", true),
    ("https://*.*.*", "https://a.b/r", false),
    ("https://a*.b", "https://ax.b/r", false),
    ("https://*a.b", "https://xa.b/r", false),
    ("http*://example.test", "http://example.test/r", false),
    ("HTTPS://EXAMPLE.TEST", "https://example.test/r", true),
    ("Example.Test", "https://example.test/r", false),
    ("example.test", "https://example.test:8443/r", false),
    ("example.test", "https://me@example.test/r", true),
    ("https://example.test", "ssh://example.test/r", false),
    ("example.test/", "https://example.test/r", true),
    ("example.test/", "https://example.test", true),
    ("example.test/r", "https://example.test/r", true),
    ("/r", "https://example.test/r", true),
    ("@example.test", "https://example.test/r", false),
    ("a b", "https://example.test/r", false),
    ("https:/example.test", "https://example.test/r", false),
    ("https:example.test", "https://example.test/r", false),
    ("https:///", "https://example.test/r", true),
    ("https:///x", "https://example.test/x", true),
    ("https://:8443", "https://example.test:8443/r", false),
    ("http://", "https://example.test/r", false),
    ("https://example.test/%2e", "https://example.test/./r", true),
    (
        "https://example.test/%2e%2e/org",
        "https://example.test/org",
        false,
    ),
    ("", "https://example.test/r", true),
    ("https://example.test/a/", "https://example.test/a", true),
    ("https://example.test/a/", "https://example.test/a/", true),
    ("https://example.test/ab/", "https://example.test/ab", true),
    ("x", "https://example.test/r", false),
];

#[test]
fn the_helper_view_selects_sections_as_git_does() {
    let mut wrong = Vec::new();
    for (pattern, url, expected) in GIT_SELECTS {
        if scoped(pattern, url) != *expected {
            wrong.push(format!(
                "credential.{pattern}.helper for {url}: Git says {expected}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} rows differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
