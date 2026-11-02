//! The caller's identity in an operation's scope (R3b-win; the HTTPS model draft r5 section 3.2 item 3): two captures
//! that differ only in the caller get different scopes, two captures of one caller share one, and a capture whose
//! identity could not be read gets a scope no other capture has.
use crate::git::{
    endpoint::{
        https_auth::{Config, OperationAuth},
        https_worker::native::{
            CallerIdentity, NativeCaller, NativeHost, identity, identity::token_layout,
        },
    },
    spawn_gate::SpawnGate,
};
use gwz_connection::AuthScope;

/// One helper for every row, so that only the caller differs.
fn helper() -> Config {
    Config {
        executable: "gwz-fixture-helper".into(),
        environment: vec![("GH_TOKEN".into(), "fixture-token".into())],
    }
}

/// The scope of an operation over `helper()` whose caller is `native`.
fn scope(native: Option<NativeCaller>) -> AuthScope {
    OperationAuth::context(helper(), native).scope()
}

/// A caller whose identity is `identity`, and otherwise a refused one.
fn caller(identity: CallerIdentity) -> NativeCaller {
    NativeCaller {
        identity,
        ..NativeCaller::refused_for_test(gwz_sspi::ErrorKind::UnsupportedPlatform)
    }
}

/// A read identity of the identifiers `sid`, logon session 1 and session 1, as the Windows reader folds them.
fn read_identity(sid: &[u8]) -> CallerIdentity {
    CallerIdentity::of(Some(identity::fold(&[sid, &[1; 8], &[1; 4]])))
}

#[test]
fn two_captures_differing_only_in_the_caller_get_different_scopes() {
    let a = read_identity(b"S-1-5-21-fixture-a");
    let b = read_identity(b"S-1-5-21-fixture-b");
    assert_ne!(scope(Some(caller(a))), scope(Some(caller(b))));
    // The same caller captured twice shares a scope, and a caller's scope is not that of an operation with none.
    assert_eq!(scope(Some(caller(a))), scope(Some(caller(a))));
    assert_ne!(scope(Some(caller(a))), scope(None));
}

#[test]
fn an_unreadable_identity_gives_each_capture_a_scope_of_its_own() {
    let first = CallerIdentity::of(None);
    let second = CallerIdentity::of(None);
    assert!(first != second, "two unreadable captures are told apart");
    assert_ne!(first.digest(), &[0; 32], "never a constant");
    assert_ne!(scope(Some(caller(first))), scope(Some(caller(second))));
    // Neither merges with the caller whose identity was read.
    let read = CallerIdentity::current();
    assert!(first != read && second != read);
    assert_ne!(scope(Some(caller(first))), scope(Some(caller(read))));
}

#[test]
fn two_captures_of_one_caller_share_a_scope() {
    // The production capture (`NativeHost::caller` on the entry thread), here on a host whose supervisor is refused.
    let host = NativeHost::on_gate(SpawnGate::new(), |_| {
        Err(gwz_sspi::ErrorKind::UnsupportedPlatform)
    });
    assert!(
        identity::platform::read().is_some(),
        "this host's caller identity is readable"
    );
    let (first, second) = (host.caller(), host.caller());
    assert!(first.identity() == second.identity());
    assert!(first.identity() == CallerIdentity::current());
    assert_eq!(scope(Some(first)), scope(Some(second)));
}

#[test]
fn every_identifier_enters_the_identity_digest() {
    let base = identity::fold(&[b"sid", b"logon", b"session"]);
    for changed in [
        identity::fold(&[b"sid-2", b"logon", b"session"]),
        identity::fold(&[b"sid", b"logon-2", b"session"]),
        identity::fold(&[b"sid", b"logon", b"session-2"]),
        identity::fold(&[b"sid", b"logon"]),
    ] {
        assert_ne!(base, changed);
    }
    // The fields are length-prefixed: moving a byte across a boundary, or adding an empty field, changes the digest.
    assert_ne!(
        identity::fold(&[b"ab", b"c"]),
        identity::fold(&[b"a", b"bc"])
    );
    assert_ne!(identity::fold(&[b"a"]), identity::fold(&[b"a", b""]));
}

/// A SID of revision 1, authority 5 and the sub-authorities `subs`.
fn sid(subs: &[u32]) -> Vec<u8> {
    let mut bytes = vec![1, subs.len() as u8, 0, 0, 0, 0, 0, 5];
    for sub in subs {
        bytes.extend(sub.to_le_bytes());
    }
    bytes
}

/// The bytes of a `TokenUser` result at the address `base`: the `TOKEN_USER` (a pointer at the SID that follows it, then
/// the attributes) and the SID.
fn token_user(base: usize, sid: &[u8]) -> Vec<u8> {
    let mut bytes = (base + token_layout::USER_HEADER).to_ne_bytes().to_vec();
    bytes.resize(token_layout::USER_HEADER, 0);
    bytes.extend(sid);
    bytes
}

#[test]
fn the_user_sid_is_read_from_its_offset_inside_the_returned_bytes() {
    let sid = sid(&[21, 1, 2, 3, 1001]);
    let buffer = token_user(0x4000, &sid);
    let range = token_layout::user_sid(&buffer, 0x4000).expect("a whole SID");
    assert_eq!(range.start, token_layout::USER_HEADER);
    assert_eq!(&buffer[range], &sid[..]);
    // Bytes after the SID belong to the API's buffer, not to the SID.
    let mut padded = buffer.clone();
    padded.extend([0xAA; 40]);
    assert_eq!(
        token_layout::user_sid(&padded, 0x4000).map(|range| range.len()),
        Some(sid.len())
    );
}

#[test]
fn a_sid_running_past_the_returned_bytes_is_refused() {
    let sid = sid(&[21, 1, 2, 3, 1001]);
    let buffer = token_user(0x4000, &sid);
    // The SID alone is shorter than the buffer, so a check of its length against the whole buffer from its start
    // accepts a SID cut short here: the SID begins `USER_HEADER` bytes in.
    for cut in 1..=sid.len() {
        assert!(
            token_layout::user_sid(&buffer[..buffer.len() - cut], 0x4000).is_none(),
            "a SID {cut} bytes short is refused"
        );
    }
    // A sub-authority count that promises more than the bytes hold is refused the same way.
    let mut lying = buffer.clone();
    lying[token_layout::USER_HEADER + 1] = 15;
    assert!(token_layout::user_sid(&lying, 0x4000).is_none());
    lying[token_layout::USER_HEADER + 1] = 16;
    assert!(token_layout::user_sid(&lying, 0x4000).is_none());
}

#[test]
fn a_sid_with_more_sub_authorities_than_windows_allows_is_refused_though_its_bytes_are_there() {
    // Sixteen sub-authorities, one past SID_MAX_SUB_AUTHORITIES, and all 8 + 64 bytes of them inside the result: only
    // the count's cap refuses it, not the end bound.
    let subs: Vec<u32> = (1..=16).collect();
    let sixteen = sid(&subs);
    assert_eq!(sixteen.len(), 8 + 64);
    let buffer = token_user(0x4000, &sixteen);
    assert!(token_layout::user_sid(&buffer, 0x4000).is_none());
    // Fifteen, the most Windows allows, is a whole SID.
    let fifteen = token_user(0x4000, &sid(&subs[..15]));
    assert_eq!(
        token_layout::user_sid(&fifteen, 0x4000).map(|range| range.len()),
        Some(8 + 60)
    );
}

#[test]
fn a_sid_pointer_that_is_not_dword_aligned_is_refused() {
    // A whole SID, inside the result and past the header, but at an offset that is not a multiple of 4: Windows hands a
    // SID out DWORD-aligned, and the reader's calls need one.
    let sid = sid(&[21, 1, 2, 3, 1001]);
    for skew in 1..4 {
        let mut bytes = (0x4000 + token_layout::USER_HEADER + skew)
            .to_ne_bytes()
            .to_vec();
        bytes.resize(token_layout::USER_HEADER + skew, 0);
        bytes.extend(&sid);
        assert!(
            token_layout::user_sid(&bytes, 0x4000).is_none(),
            "a SID {skew} bytes off a DWORD boundary is refused"
        );
    }
    // The same SID at the next aligned offset is read.
    let mut aligned = (0x4000 + token_layout::USER_HEADER + 4)
        .to_ne_bytes()
        .to_vec();
    aligned.resize(token_layout::USER_HEADER + 4, 0);
    aligned.extend(&sid);
    assert_eq!(
        token_layout::user_sid(&aligned, 0x4000),
        Some(token_layout::USER_HEADER + 4..token_layout::USER_HEADER + 4 + sid.len())
    );
}

#[test]
fn a_sid_pointer_outside_the_returned_bytes_or_inside_the_header_is_refused() {
    let sid = sid(&[18]);
    let buffer = token_user(0x4000, &sid);
    // The pointer is read against the buffer's own address.
    assert!(
        token_layout::user_sid(&buffer, 0x4008).is_none(),
        "before the buffer"
    );
    assert!(
        token_layout::user_sid(&buffer, 0x3000).is_none(),
        "past the bytes"
    );
    assert!(
        token_layout::user_sid(&buffer, usize::MAX).is_none(),
        "below the buffer's base"
    );
    let mut inside = buffer.clone();
    inside[..std::mem::size_of::<usize>()].copy_from_slice(&0x4000usize.to_ne_bytes());
    assert!(
        token_layout::user_sid(&inside, 0x4000).is_none(),
        "the pointer names the header itself"
    );
    // A whole SID that lies inside the header's own bytes is not the user's SID either: the attributes slot holds one.
    let mut within = buffer.clone();
    let word = std::mem::size_of::<usize>();
    within[..word].copy_from_slice(&(0x4000 + word).to_ne_bytes());
    within[word..word + 8].copy_from_slice(&[1, 0, 0, 0, 0, 0, 0, 5]);
    assert!(
        token_layout::user_sid(&within, 0x4000).is_none(),
        "the pointer names the header's tail"
    );
    // Not a SID: another revision, or fewer bytes than the header.
    let mut revision = buffer.clone();
    revision[token_layout::USER_HEADER] = 2;
    assert!(token_layout::user_sid(&revision, 0x4000).is_none());
    assert!(token_layout::user_sid(&buffer[..3], 0x4000).is_none());
    assert!(token_layout::user_sid(&[], 0x4000).is_none());
}

#[test]
fn the_logon_session_and_session_id_are_read_only_from_whole_results() {
    // TOKEN_STATISTICS: the token's LUID, then the logon session's.
    let mut statistics = vec![0xEE; 8];
    statistics.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    statistics.resize(56, 0);
    assert_eq!(
        token_layout::logon(&statistics),
        Some([1, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(
        token_layout::logon(&statistics[..16]),
        Some([1, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(token_layout::logon(&statistics[..15]), None);
    assert_eq!(token_layout::logon(&[]), None);
    assert_eq!(token_layout::session(&[7, 0, 0, 0]), Some([7, 0, 0, 0]));
    assert_eq!(token_layout::session(&[7, 0, 0]), None);
    assert_eq!(token_layout::session(&[]), None);
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::Authorization::ConvertSidToStringSidW,
            System::{RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId},
        };

        /// The text form of the SID `sid`, as Windows' own `ConvertSidToStringSidW` writes it.
        fn sid_string(sid: &[u8]) -> String {
            // The SID is copied to 4-byte aligned storage, as Windows hands one out.
            let mut words = vec![0u32; sid.len().div_ceil(4)];
            // SAFETY: `words` holds at least `sid.len()` bytes and does not overlap `sid`.
            unsafe { std::ptr::copy_nonoverlapping(sid.as_ptr(), words.as_mut_ptr().cast::<u8>(), sid.len()) };
            let mut text: *mut u16 = std::ptr::null_mut();
            // SAFETY: `words` holds a whole SID (its bytes came from a validated one); `text` receives a string that
            // `LocalFree` releases.
            assert_ne!(unsafe { ConvertSidToStringSidW(words.as_mut_ptr().cast(), &mut text) }, 0);
            // SAFETY: on success `text` is a NUL-terminated UTF-16 string, valid until the `LocalFree` below.
            let string = unsafe {
                let mut length = 0;
                while *text.add(length) != 0 {
                    length += 1;
                }
                String::from_utf16(std::slice::from_raw_parts(text, length)).expect("a SID string is ASCII")
            };
            // SAFETY: `text` was allocated by `ConvertSidToStringSidW`, which documents `LocalFree` for it.
            unsafe { LocalFree(text.cast()) };
            string
        }

        /// The SID `whoami /user` prints for the account this test runs as: the last field of its one CSV row.
        fn whoami_sid() -> String {
            let root = std::env::var("SystemRoot").expect("SystemRoot names the Windows directory");
            let output = std::process::Command::new(format!("{root}\\System32\\whoami.exe"))
                .args(["/user", "/fo", "csv", "/nh"])
                .output()
                .expect("whoami runs");
            assert!(output.status.success(), "whoami /user succeeds");
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            let row = text.lines().find(|line| !line.trim().is_empty()).expect("whoami prints a row");
            row.rsplit(',').next().expect("a last field").trim().trim_matches('"').to_owned()
        }

        /// The reader yields all three identifiers and folds them. Only the SID and the session id are then checked
        /// against sources independent of the reader (the next row); the logon LUID is not: it relies on the compile-time
        /// `offset_of!` assert against `windows-sys` in the reader and on the pure parser row
        /// `the_logon_session_and_session_id_are_read_only_from_whole_results`.
        #[test]
        fn the_process_tokens_sid_logon_session_and_session_are_read() {
            let token = identity::platform::token().expect("the process token is readable");
            assert_eq!(token.sid.first(), Some(&1), "a SID starts with its revision, 1");
            assert!(token.sid.len() >= 8, "a SID has its header and an authority");
            assert_ne!(token.logon, [0; 8], "the logon session's LUID");
            assert_eq!(
                identity::platform::read(),
                Some(identity::fold(&[&token.sid, &token.logon, &token.session]))
            );
            assert!(CallerIdentity::current() == CallerIdentity::of(identity::platform::read()));
        }

        /// The reader against sources that are not the reader, for the SID and the session id only: the SID Windows
        /// writes as text against the SID `whoami /user` prints for this account, and the session id against
        /// `ProcessIdToSessionId` for this process. The logon LUID has no independent source here (see the row above).
        #[test]
        fn the_process_tokens_sid_and_session_match_whoami_and_the_process_session() {
            let token = identity::platform::token().expect("the process token is readable");
            let sid = sid_string(&token.sid);
            assert!(sid.starts_with("S-1-"), "a SID string starts S-1-: {sid}");
            assert_eq!(sid, whoami_sid(), "the token's user SID is the account whoami names");
            let mut session = 0u32;
            // SAFETY: `session` receives the session id of this process.
            let found = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
            assert_ne!(found, 0, "ProcessIdToSessionId answers for this process");
            assert_eq!(
                u32::from_le_bytes(token.session),
                session,
                "the token's session id is this process's session"
            );
        }
    }
}
