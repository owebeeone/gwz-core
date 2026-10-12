//! Whether the configuration view holds a credential helper for a URL, read as `git credential` reads it.
//!
//! `git credential fill` ends the same way when no helper is configured and when one is and gave nothing: git
//! prompts, prompts are off, and it exits with failure. The view is what tells them apart. It is the configuration
//! Git itself is given (the same entries, conditional includes unevaluated), so a helper list that is empty here is
//! empty for Git.
use super::framing::Entry;

/// The parts of a URL that a `credential.<url>.*` section matches on (Git's `credential_from_url_gently`).
struct Parts<'a> {
    protocol: &'a [u8],
    host: &'a [u8],
    path: Option<&'a [u8]>,
    username: Option<&'a [u8]>,
}

/// `None` where Git ignores the URL (it has no scheme or no host).
fn split(url: &[u8]) -> Option<Parts<'_>> {
    let scheme = url.windows(3).position(|window| window == b"://")?;
    let rest = &url[scheme + 3..];
    let slash = rest
        .iter()
        .position(|byte| matches!(byte, b'/' | b'?' | b'#'))
        .unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(slash);
    let (username, host) = match authority.iter().position(|byte| *byte == b'@') {
        Some(at) => {
            let user = &authority[..at];
            // `user:password@host` names the user before the colon; a password is never matched on.
            let name = user.split(|byte| *byte == b':').next().unwrap_or(user);
            (Some(name), &authority[at + 1..])
        }
        None => (None, authority),
    };
    if scheme == 0 || host.is_empty() {
        return None;
    }
    let path = tail.strip_prefix(b"/").filter(|path| !path.is_empty());
    Some(Parts {
        protocol: &url[..scheme],
        host,
        path,
        username,
    })
}

/// Git's `credential_match`: every part the pattern names is the same in the request.
fn matches(pattern: &Parts<'_>, request: &Parts<'_>) -> bool {
    let same =
        |want: Option<&[u8]>, have: Option<&[u8]>| want.is_none_or(|want| have == Some(want));
    pattern.protocol == request.protocol
        && pattern.host == request.host
        && same(pattern.path, request.path)
        && same(pattern.username, request.username)
}

/// Whether Git, given `entries` and asked for `url`, holds at least one credential helper. Entries apply in their
/// order: `credential.helper` and the `credential.<pattern>.helper` whose pattern matches the URL add a helper,
/// and an empty value clears the list. A key with no value adds nothing.
pub(super) fn in_effect(entries: &[Entry], url: &[u8]) -> bool {
    let Some(request) = split(url) else {
        return false;
    };
    let mut held = false;
    for entry in entries {
        let Some(rest) = entry.name.0.strip_prefix(b"credential.") else {
            continue;
        };
        let applies = if rest == b"helper" {
            true
        } else {
            rest.strip_suffix(b".helper")
                .filter(|pattern| !pattern.is_empty())
                .and_then(split)
                .is_some_and(|pattern| matches(&pattern, &request))
        };
        if !applies {
            continue;
        }
        if let Some(value) = &entry.value {
            held = !value.0.is_empty();
        }
    }
    held
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use crate::git::endpoint::https_auth::SecretBuffer;

            fn entry(name: &str, value: Option<&str>) -> Entry {
                Entry {
                    name: SecretBuffer(name.as_bytes().to_vec()),
                    value: value.map(|value| SecretBuffer(value.as_bytes().to_vec())),
                }
            }
            fn held(entries: &[(&str, Option<&str>)], url: &str) -> bool {
                let entries: Vec<_> = entries.iter().map(|(name, value)| entry(name, *value)).collect();
                in_effect(&entries, url.as_bytes())
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
                    &[("credential.helper", Some("store")), ("credential.helper", Some(""))],
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
                    ("credential.https://github.com.helper", Some("!gh auth git-credential")),
                ];
                assert!(!held(&github, URL));
                assert!(held(&github, "https://github.com/team/repo"));
                assert!(!held(&[("credential.https://example.test.helper", Some("store"))], "http://example.test/repo"));
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
                let scoped = |pattern: &str, url: &str| {
                    held(&[(&format!("credential.{pattern}.helper"), Some("store"))], url)
                };
                assert!(scoped("https://example.test", URL));
                assert!(scoped("https://example.test/repo", URL));
                assert!(!scoped("https://example.test/other", URL));
                assert!(!scoped("https://example.test:8443", URL));
                assert!(scoped("https://example.test:8443", "https://example.test:8443/repo"));
                assert!(!scoped("https://me@example.test", URL));
                assert!(scoped("https://me@example.test", "https://me@example.test/repo"));
                assert!(scoped("https://me@example.test", "https://me:secret@example.test/repo"));
                assert!(!scoped("https://Example.test", URL));
            }

            #[test]
            fn what_git_ignores_is_not_a_helper() {
                // A pattern with no scheme or no host, and a helper key with no value at all.
                assert!(!held(&[("credential.example.test.helper", Some("store"))], URL));
                assert!(!held(&[("credential.https://.helper", Some("store"))], URL));
                assert!(!held(&[("credential..helper", Some("store"))], URL));
                assert!(!held(&[("credential.helper", None)], URL));
                assert!(!held(&[("credential.helper", Some("store"))], "not a url"));
            }
        }
    }
}
