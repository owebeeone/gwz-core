//! The exchange's pure rules, which run on every platform.
use super::*;

#[test]
fn a_request_is_framed_with_its_length_and_must_fit_the_mapping() {
    assert_eq!(frame(&[11]).unwrap(), [0, 0, 0, 1, 11]);
    let largest = vec![13; MAX_PAYLOAD];
    let framed = frame(&largest).unwrap();
    assert_eq!(framed.len(), MAPPING_SIZE);
    assert_eq!(&framed[..4], (MAX_PAYLOAD as u32).to_be_bytes());
    // Nothing, or one byte too many, is refused before anything is sent.
    assert_eq!(frame(&[]), Err(PageantError::BadRequest));
    assert_eq!(
        frame(&vec![13; MAX_PAYLOAD + 1]),
        Err(PageantError::BadRequest)
    );
    assert_eq!(MAX_PAYLOAD, 8188);
}

/// A mapping as Pageant leaves it: the frame of `payload`, then zeros.
fn mapping(payload: &[u8]) -> Vec<u8> {
    let mut mapping = vec![0; MAPPING_SIZE];
    mapping[..4].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    mapping[4..4 + payload.len()].copy_from_slice(payload);
    mapping
}

#[test]
fn a_reply_is_its_declared_length_of_an_agent_message() {
    for kind in [5u8, 6, 12, 14, 28] {
        assert_eq!(reply(&mapping(&[kind, 1, 2, 3])).unwrap(), [kind, 1, 2, 3]);
    }
    let largest = {
        let mut payload = vec![0; MAX_PAYLOAD];
        payload[0] = 12;
        payload
    };
    assert_eq!(reply(&mapping(&largest)).unwrap().len(), MAX_PAYLOAD);
}

#[test]
fn a_reply_of_a_bad_length_or_type_is_refused() {
    // Zero is no message; above the mapping's room is more than Pageant can have written.
    let mut zero = mapping(&[12]);
    zero[..4].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(reply(&zero), Err(PageantError::BadReplyLength(0)));
    for length in [MAX_PAYLOAD as u32 + 1, 8192, 1 << 24, u32::MAX] {
        let mut over = mapping(&[12]);
        over[..4].copy_from_slice(&length.to_be_bytes());
        assert_eq!(reply(&over), Err(PageantError::BadReplyLength(length)));
    }
    // A mapping cut short cannot hold the length it declares.
    assert_eq!(reply(&[0, 0]), Err(PageantError::BadReplyLength(0)));
    assert_eq!(
        reply(&[0, 0, 0, 9, 12]),
        Err(PageantError::BadReplyLength(9))
    );
    // Only the types an agent answers with.
    for kind in [0u8, 1, 4, 7, 11, 13, 27, 29, 0x99, 0xff] {
        assert_eq!(
            reply(&mapping(&[kind, 0, 0, 0, 0])),
            Err(PageantError::BadReplyType(kind))
        );
    }
}

#[test]
fn mapping_names_are_local_unique_and_valid() {
    let ids = IdSource::new(0xabc);
    let first = mapping_name(4242, &ids);
    let second = mapping_name(4242, &ids);
    assert_ne!(first, second);
    assert!(first.starts_with("Local\\PageantRequest-4242-0000000000000abc-"));
    assert!(valid_name(&first) && valid_name(&second));
    // Another runtime's names differ even where the counters agree.
    assert_ne!(
        mapping_name(4242, &IdSource::new(1)),
        mapping_name(4242, &IdSource::new(2))
    );
    for name in [
        "",
        "Local\\PageantRequest-",
        "Global\\PageantRequest-1-2-3",
        "PageantRequest-1-2-3",
        "Local\\PageantRequest-1 2",
        "Local\\PageantRequest-1\0",
        "Local\\PageantRequest-\u{e9}",
        &format!("Local\\PageantRequest-{}", "1".repeat(MAX_NAME)),
    ] {
        assert!(!valid_name(name), "{name:?}");
    }
}

#[test]
fn the_descriptor_owns_the_mapping_for_the_caller_and_opens_it_to_system_only() {
    let sid = "S-1-5-21-1-2-3-1001";
    assert_eq!(
        descriptor(sid),
        "O:S-1-5-21-1-2-3-1001D:P(A;;GA;;;SY)(A;;GA;;;S-1-5-21-1-2-3-1001)"
    );
    // A protected DACL (`P`) with two entries and no Everyone or Authenticated Users.
    let text = descriptor(sid);
    assert_eq!(text.matches("(A;").count(), 2);
    assert!(text.contains("D:P"));
    assert!(!text.contains(";WD)") && !text.contains(";AU)"));
}

#[test]
fn a_bound_is_a_whole_number_of_milliseconds_of_at_least_one() {
    assert_eq!(timeout_ms(Duration::from_secs(0)), 1);
    assert_eq!(timeout_ms(Duration::from_micros(10)), 1);
    assert_eq!(timeout_ms(Duration::from_millis(1500)), 1500);
    assert_eq!(timeout_ms(Duration::from_secs(u64::MAX / 1000)), u32::MAX);
}

#[test]
fn every_error_has_a_reason_and_a_kind() {
    for (error, kind) in [
        (PageantError::Timeout, io::ErrorKind::TimedOut),
        (PageantError::BadRequest, io::ErrorKind::InvalidInput),
        (PageantError::BadName, io::ErrorKind::InvalidInput),
        (PageantError::SameQueue, io::ErrorKind::InvalidInput),
        (PageantError::NoWindow, io::ErrorKind::ConnectionAborted),
        (PageantError::Rejected, io::ErrorKind::ConnectionAborted),
        (PageantError::NameInUse, io::ErrorKind::AlreadyExists),
        (PageantError::BadReplyLength(0), io::ErrorKind::InvalidData),
        (PageantError::BadReplyType(9), io::ErrorKind::InvalidData),
        (PageantError::Os(5), io::ErrorKind::Other),
    ] {
        let text = error.to_string();
        let converted = io::Error::from(error);
        assert_eq!(converted.kind(), kind, "{text}");
        assert!(text.starts_with("Pageant "), "{text}");
        // The error carries no mapping content: only counts and codes.
        assert_eq!(converted.to_string(), text);
    }
}
