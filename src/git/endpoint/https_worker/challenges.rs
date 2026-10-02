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
                    if !tokens.iter().any(|s: &String| s.eq_ignore_ascii_case(token)) {
                        let bounded = token[..token.len().min(32)].to_owned();
                        if tokens.len() < 4 { tokens.push(bounded); }
                        else if token.eq_ignore_ascii_case("Negotiate") {
                            // This token affects private-repository classification.
                            // Preserve it within the existing four-token contract.
                            tokens[3] = bounded;
                        }
                    }
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
    fn remediation_negotiate_survives_bounded_projection() {
        for value in [
            "One, Two, Three, Four, nEgOtIaTe realm=\"private\", Basic",
            "One, One, Two, Three, Four, Negotiate, NEGOTIATE",
            "Negotiate, One, Two, Three, Four",
        ] {
            let mut headers = hyper::HeaderMap::new();
            headers.append(hyper::header::WWW_AUTHENTICATE, value.parse().unwrap());
            let (basic, names) = schemes(&headers);
            assert_eq!(basic, value.contains("Basic"));
            assert!(names.len() <= 4);
            assert!(names.iter().any(|s| s.eq_ignore_ascii_case("Negotiate")));
            assert!(!names.iter().any(|s| s.contains("private")));
            let envelope = gwz_transport::protocol::Envelope {
                version: 2, session_id: "schemes".into(), stream_id: 1,
                kind: gwz_transport::protocol::MessageKind::OpenFailed,
                open_failed: Some(gwz_transport::protocol::Failure {
                    code: gwz_transport::protocol::ErrorCode::Authentication,
                    detail: Some(Box::new(gwz_transport::protocol::FailureDetail { schemes: Some(names), ..Default::default() })),
                    ..Default::default()
                }), ..Default::default()
            };
            let decoded = gwz_transport::codec::decode(&gwz_transport::codec::encode(&envelope).unwrap()).unwrap();
            assert_eq!(decoded, envelope);
        }
    }
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
