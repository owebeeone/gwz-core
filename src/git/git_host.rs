



/// Extracts the remote host from a git URL, for per-host connection limiting.
/// Handles scp-like `git@host:path`, scheme URLs (`https://`, `ssh://`, …), and
/// returns `None` for local paths or any URL with no parseable host (which the
/// caller bounds only by the global concurrency ceiling).
pub fn git_host(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if url.contains("://") {
        return url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
            .filter(|host| !host.is_empty());
    }
    // scp-like: [user@]host:path — a colon before any slash.
    let colon = url.find(':')?;
    let authority = &url[..colon];
    if authority.contains('/') {
        return None; // a local path that happens to contain a colon
    }
    let host = authority.rsplit('@').next().unwrap_or(authority).trim();
    // A lone alphabetic char before ':' is a Windows drive letter, not a host.
    if host.is_empty() || (host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic())) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

