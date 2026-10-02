//! Scheme tokens only: realms and authentication parameters never leave HTTP.
use hyper::{HeaderMap, header::WWW_AUTHENTICATE};
pub(super) fn schemes(headers: &HeaderMap) -> (bool, Vec<String>) {
    let mut basic = false;
    let mut tokens = Vec::new();
    for value in headers.get_all(WWW_AUTHENTICATE) {
        let bytes = value.as_bytes();
        let mut start = 0;
        let mut quoted = false;
        let mut escaped = false;
        for i in 0..=bytes.len() {
            let byte = bytes.get(i).copied();
            if !quoted && (byte == Some(b',') || byte.is_none()) {
                let segment = &bytes[start..i];
                let segment = segment.trim_ascii();
                let n = segment.iter().take_while(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(b)).count();
                if n > 0 && segment.get(n).is_none_or(|b| b.is_ascii_whitespace()) {
                    let token = std::str::from_utf8(&segment[..n]).expect("ASCII token");
                    basic |= token.eq_ignore_ascii_case("Basic");
                    if tokens.len() < 4 && !tokens.iter().any(|s: &String| s.eq_ignore_ascii_case(token)) { tokens.push(token[..token.len().min(32)].into()); }
                }
                start = i + 1;
            }
            if escaped { escaped = false; }
            else if quoted && byte == Some(b'\\') { escaped = true; }
            else if byte == Some(b'"') { quoted = !quoted; }
        }
    }
    (basic, tokens)
}
cfg_if::cfg_if! { if #[cfg(test)] {
    #[test]
    fn challenge_parameters_and_quoted_commas_are_not_schemes() {
        let mut headers = hyper::HeaderMap::new();
        headers.append(hyper::header::WWW_AUTHENTICATE, "Digest realm=\"Basic,private\", nonce=\"secret\", bAsIc realm=\"secret\"".parse().unwrap());
        let (basic, names) = schemes(&headers);
        assert!(basic);
        assert_eq!(names, ["Digest", "bAsIc"]);
        headers.clear();
        headers.append(hyper::header::WWW_AUTHENTICATE, "Bearer realm=\"Basic\"".parse().unwrap());
        assert_eq!(schemes(&headers), (false, vec!["Bearer".into()]));
    }
} }
