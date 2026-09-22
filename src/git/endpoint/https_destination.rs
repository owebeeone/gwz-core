//! Side-effect-free HTTPS destination and smart-HTTP request construction.
//!
//! This module deliberately owns URL grammar only.  DNS, credentials, TLS and
//! redirects beyond one validated location belong to the endpoint runtime.

use gwz_transport::protocol::{ErrorCode, GitService};
use url::{Host, Url};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Destination {
    pub(crate) url: Url,
}

impl Destination {
    pub(crate) fn parse(input: &str) -> Result<Self, ErrorCode> {
        if input.is_empty() || input.len() > 18_000 || input.chars().any(char::is_control) {
            return Err(ErrorCode::InvalidRequest);
        }
        let has_path = input.split_once("://").is_some_and(|(_, rest)| {
            let slash = rest.find('/');
            let other_delimiter = rest.find(['?', '#']);
            slash.is_some_and(|slash| other_delimiter.is_none_or(|delimiter| slash < delimiter))
        });
        if !has_path {
            return Err(ErrorCode::InvalidRequest);
        }
        if input.split_once("://").is_some_and(|(_, rest)| {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            rest[..end].contains(['%', '\\', '@'])
        }) {
            return Err(ErrorCode::InvalidRequest);
        }
        let url = Url::parse(input).map_err(|_| ErrorCode::InvalidRequest)?;
        Self::from_url(url)
    }

    pub(crate) fn authority(&self) -> String {
        let host = self.host();
        let host = if self
            .url
            .host()
            .is_some_and(|value| matches!(value, Host::Ipv6(_)))
            && !host.starts_with('[')
        {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        if self.port() == 443 {
            host
        } else {
            format!("{host}:{}", self.port())
        }
    }

    pub(crate) fn host(&self) -> &str {
        self.url
            .host_str()
            .expect("validated HTTPS destination host")
            .trim_start_matches('[')
            .trim_end_matches(']')
    }

    pub(crate) fn port(&self) -> u16 {
        self.url
            .port_or_known_default()
            .expect("validated HTTPS port")
    }

    pub(crate) fn base(&self) -> String {
        self.url.as_str().to_owned()
    }

    pub(crate) fn request(&self, service: GitService) -> Url {
        let mut request = self.url.clone();
        let path = service_path(service);
        {
            let mut segments = request
                .path_segments_mut()
                .expect("validated HTTPS URL is a base");
            segments.pop_if_empty();
            for segment in path.split('/') {
                segments.push(segment);
            }
        }
        request.set_query(match service_query(service) {
            Some(query) => Some(query),
            None => None,
        });
        request
    }

    /// Resolve one discovery Location and recover its repository base.
    /// Redirect hop limits and write-once route ownership are endpoint concerns.
    pub(crate) fn redirect(&self, service: GitService, location: &str) -> Result<Self, ErrorCode> {
        let expected = service_query(service).ok_or(ErrorCode::UnsupportedOperation)?;
        if location.is_empty() || location.len() > 18_000 || location.chars().any(char::is_control)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        if ambiguous_authority(location) {
            return Err(ErrorCode::InvalidRequest);
        }
        let target = self
            .request(service)
            .join(location)
            .map_err(|_| ErrorCode::InvalidRequest)?;
        validate_redirect_url(&target, expected)?;
        if target.query().is_some_and(|query| query != expected) {
            return Err(ErrorCode::InvalidRequest);
        }
        let path = target
            .path()
            .strip_suffix("/info/refs")
            .ok_or(ErrorCode::InvalidRequest)?;
        let base_path = if path.is_empty() { "/" } else { path };
        let mut base = target.clone();
        base.set_query(None);
        base.set_fragment(None);
        base.set_path(base_path);
        let candidate = Self::from_url(base)?;
        let mut expected_request = target.clone();
        expected_request.set_query(Some(expected));
        if candidate.request(service) != expected_request {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(candidate)
    }

    fn from_url(url: Url) -> Result<Self, ErrorCode> {
        validate_url(&url)?;
        Ok(Self { url })
    }
}

fn service_query(service: GitService) -> Option<&'static str> {
    match service {
        GitService::UploadPackAdvertisement => Some("service=git-upload-pack"),
        GitService::ReceivePackAdvertisement => Some("service=git-receive-pack"),
        GitService::UploadPackExchange | GitService::ReceivePackExchange => None,
    }
}

fn service_path(service: GitService) -> &'static str {
    match service {
        GitService::UploadPackAdvertisement | GitService::ReceivePackAdvertisement => "info/refs",
        GitService::UploadPackExchange => "git-upload-pack",
        GitService::ReceivePackExchange => "git-receive-pack",
    }
}

fn ambiguous_authority(input: &str) -> bool {
    let rest = input
        .split_once("://")
        .map(|(_, rest)| rest)
        .or_else(|| input.strip_prefix("//"));
    rest.is_some_and(|rest| {
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        rest[..end].contains(['%', '\\', '@'])
    })
}

fn validate_url(url: &Url) -> Result<(), ErrorCode> {
    validate_url_shape(url)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(())
}

fn validate_redirect_url(url: &Url, expected_query: &str) -> Result<(), ErrorCode> {
    validate_url_shape(url)?;
    if url.fragment().is_some() || url.query().is_some_and(|query| query != expected_query) {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(())
}

fn validate_url_shape(url: &Url) -> Result<(), ErrorCode> {
    if !url.scheme().eq_ignore_ascii_case("https")
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ErrorCode::InvalidRequest);
    }
    let host = url.host_str().ok_or(ErrorCode::InvalidRequest)?;
    if host.is_empty()
        || host.contains('%')
        || host.contains('@')
        || host.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ErrorCode::InvalidRequest);
    }
    let path = url.path();
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(ErrorCode::InvalidRequest);
    }
    if url.port_or_known_default().is_none_or(|port| port == 0) {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(())
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;

            fn upload_advertisement() -> GitService {
                GitService::UploadPackAdvertisement
            }

            #[test]
            fn preserves_base_path_and_builds_each_request() {
                let destination = Destination::parse("https://EXAMPLE.com:8443/owner/repo%2Fmirror/").unwrap();
                assert_eq!(destination.host(), "example.com");
                assert_eq!(destination.port(), 8443);
                assert_eq!(destination.authority(), "example.com:8443");
                assert_eq!(destination.base(), "https://example.com:8443/owner/repo%2Fmirror/");
                assert_eq!(
                    destination.request(upload_advertisement()).as_str(),
                    "https://example.com:8443/owner/repo%2Fmirror/info/refs?service=git-upload-pack"
                );
                assert_eq!(
                    destination.request(GitService::UploadPackExchange).as_str(),
                    "https://example.com:8443/owner/repo%2Fmirror/git-upload-pack"
                );
            }

            #[test]
            fn accepts_ipv6_and_omits_default_port() {
                let destination = Destination::parse("https://[2001:db8::1]/repo").unwrap();
                assert_eq!(destination.authority(), "[2001:db8::1]");
                assert_eq!(destination.port(), 443);
            }

            #[test]
            fn rejects_credential_query_fragment_and_escaped_authority_forms() {
                for input in [
                    "https://user@example.com/repo",
                    "https://user:password@example.com/repo",
                    "https://example.com/repo?token=secret",
                    "https://example.com/repo#fragment",
                    "https://%65xample.com/repo",
                    "http://example.com/repo",
                    "https://example.com",
                ] {
                    assert_eq!(Destination::parse(input), Err(ErrorCode::InvalidRequest), "{input}");
                }
            }

            #[test]
            fn redirect_accepts_exact_generated_query_and_relative_location() {
                let source = Destination::parse("https://example.com/old/repo").unwrap();
                let redirected = source
                    .redirect(upload_advertisement(), "../../new/info/refs?service=git-upload-pack")
                    .unwrap();
                assert_eq!(redirected.base(), "https://example.com/old/new");
                assert_eq!(
                    redirected.request(upload_advertisement()).as_str(),
                    "https://example.com/old/new/info/refs?service=git-upload-pack"
                );
            }

            #[test]
            fn redirect_rejects_query_mismatch_duplicates_encoded_values_and_downgrade() {
                let source = Destination::parse("https://example.com/repo").unwrap();
                for location in [
                    "https://example.com/repo/info/refs?service=git-receive-pack",
                    "https://example.com/repo/info/refs?service=git-upload-pack&service=git-upload-pack",
                    "https://example.com/repo/info/refs?service=git%2Dupload%2Dpack",
                    "https://example.com/repo/info/refs?other=value",
                    "http://example.com/repo/info/refs",
                    "https://example.com/repo/info/refs#frag",
                ] {
                    assert_eq!(
                        source.redirect(upload_advertisement(), location),
                        Err(ErrorCode::InvalidRequest),
                        "{location}"
                    );
                }
            }

            #[test]
            fn redirect_adds_the_generated_query_to_a_query_free_info_refs_location() {
                let source = Destination::parse("https://example.com/repo").unwrap();
                let redirected = source
                    .redirect(upload_advertisement(), "/moved/info/refs")
                    .unwrap();
                assert_eq!(redirected.base(), "https://example.com/moved");
                assert_eq!(
                    redirected.request(upload_advertisement()).as_str(),
                    "https://example.com/moved/info/refs?service=git-upload-pack"
                );
            }

            #[test]
            fn exchange_redirects_are_unsupported() {
                let source = Destination::parse("https://example.com/repo").unwrap();
                assert_eq!(
                    source.redirect(GitService::UploadPackExchange, "/repo/info/refs"),
                    Err(ErrorCode::UnsupportedOperation)
                );
            }
        }
    }
}
