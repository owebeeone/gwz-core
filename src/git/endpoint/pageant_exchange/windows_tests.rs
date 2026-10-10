//! The exchange against a synthetic Pageant (`receiver`): a window of this process on a thread of its own that
//! answers as Pageant does. The rows are 3.5's "tests first" list; the same rows against Pageant 0.83 itself are
//! `pageant_083.rs`'s.
use super::{
    receiver::{Receiver, Reply},
    *,
};
use std::{
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND},
    System::Memory::{
        CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
    },
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE},
};

const BOUND: Duration = Duration::from_secs(10);
const NO_KEYS: [u8; 5] = [12, 0, 0, 0, 0];

/// A source of its own, as each runtime has: a prefix drawn from the system, so that tests in one process, which share
/// a process id, never draw one name.
fn ids() -> IdSource {
    crate::operation_context::new_id_source()
}

fn exchange() -> Exchange {
    Exchange::new().expect("the caller's identity")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn a_request_gets_pageants_reply_and_each_uses_a_mapping_of_its_own() {
    let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let (exchange, ids) = (exchange(), ids());
    for _ in 0..3 {
        assert_eq!(
            exchange.send(pageant.window, &ids, &[11], BOUND).unwrap(),
            NO_KEYS
        );
    }
    let seen = pageant.seen();
    assert_eq!(seen.len(), 3);
    for one in &seen {
        assert_eq!(one.request, [11]);
        assert!(valid_name(&one.name), "{}", one.name);
    }
    let mut names: Vec<_> = seen.iter().map(|one| one.name.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 3);
}

#[test]
fn the_largest_request_and_reply_go_through() {
    let mut reply = vec![14; MAX_PAYLOAD];
    reply[1] = 7;
    let pageant = Receiver::start(Reply::Message(reply.clone()));
    let request = vec![13; MAX_PAYLOAD];
    let got = exchange()
        .send(pageant.window, &ids(), &request, BOUND)
        .unwrap();
    assert_eq!(got, reply);
    assert_eq!(pageant.seen()[0].request, request);
}

#[test]
fn the_mapping_is_the_callers_and_system_s_alone() {
    let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let exchange = exchange();
    exchange.send(pageant.window, &ids(), &[11], BOUND).unwrap();
    let descriptor = &pageant.seen()[0].descriptor;
    // What the mapping holds, as the system reports it: the caller owns it and the DACL is protected, with an entry
    // for SYSTEM and one for the caller and no other (no Everyone, no Authenticated Users, no Administrators).
    assert!(
        descriptor.starts_with(&format!("O:{}", exchange.sid())),
        "{descriptor}"
    );
    assert!(descriptor.contains("D:P"), "{descriptor}");
    assert_eq!(descriptor.matches("(A;").count(), 2, "{descriptor}");
    assert!(
        descriptor.contains(&format!(";{})", exchange.sid())),
        "{descriptor}"
    );
    for other in [";WD)", ";AU)", ";BA)", ";IU)"] {
        assert!(!descriptor.contains(other), "{other}: {descriptor}");
    }
}

#[test]
fn an_unanswered_request_times_out_and_the_late_write_is_never_read() {
    let late = Receiver::start(Reply::Late {
        delay: Duration::from_millis(1500),
        message: vec![12, 0, 0, 0, 9],
    });
    let exchange = exchange();
    let ids = ids();
    let begun = Instant::now();
    let error = exchange
        .send(late.window, &ids, &[11], Duration::from_millis(150))
        .unwrap_err();
    assert_eq!(error, PageantError::Timeout);
    assert!(
        begun.elapsed() < Duration::from_millis(1200),
        "{:?}",
        begun.elapsed()
    );
    // Pageant still holds the request and answers into its mapping once its prompt is done; nothing reads that.
    let until = Instant::now() + Duration::from_secs(10);
    while !late.late_written() {
        assert!(Instant::now() < until, "the late reply was never written");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(late.seen().len(), 1, "the request was not resent");
    // The next request is another mapping and gets its own reply, not the late one.
    let other = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    assert_eq!(
        exchange.send(other.window, &ids, &[11], BOUND).unwrap(),
        NO_KEYS
    );
    assert_ne!(other.seen()[0].name, late.seen()[0].name);
}

#[test]
fn a_name_in_use_is_refused_without_a_write() {
    let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let ids = ids();
    let name = mapping_name(std::process::id(), &ids);
    // Another party holds a mapping of that name, as an attacker who guessed it would.
    // SAFETY: a pagefile-backed mapping of this test, viewed and closed here.
    unsafe {
        let wide_name = wide(&name);
        let held = CreateFileMappingW(
            windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            ptr::null(),
            PAGE_READWRITE,
            0,
            MAPPING_SIZE as u32,
            wide_name.as_ptr(),
        );
        assert!(!held.is_null());
        let view = MapViewOfFile(held, FILE_MAP_ALL_ACCESS, 0, 0, MAPPING_SIZE);
        let error = exchange()
            .send_named(pageant.window, &name, &[11], BOUND)
            .unwrap_err();
        assert_eq!(error, PageantError::NameInUse);
        let held_bytes = std::slice::from_raw_parts(view.Value as *const u8, MAPPING_SIZE);
        assert!(
            held_bytes.iter().all(|byte| *byte == 0),
            "the held mapping was written"
        );
        UnmapViewOfFile(view);
        CloseHandle(held);
    }
    assert!(pageant.seen().is_empty(), "no message was sent");
}

#[test]
fn a_request_that_cannot_be_framed_is_refused_before_it_is_sent() {
    let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let (exchange, ids) = (exchange(), ids());
    for request in [
        vec![],
        vec![11; MAX_PAYLOAD + 1],
        vec![11; MAPPING_SIZE * 4],
    ] {
        assert_eq!(
            exchange
                .send(pageant.window, &ids, &request, BOUND)
                .unwrap_err(),
            PageantError::BadRequest
        );
    }
    for name in [
        "",
        "Global\\PageantRequest-1",
        "Local\\Other-1",
        "Local\\PageantRequest-1 ",
    ] {
        assert_eq!(
            exchange
                .send_named(pageant.window, name, &[11], BOUND)
                .unwrap_err(),
            PageantError::BadName,
            "{name:?}"
        );
    }
    assert!(pageant.seen().is_empty(), "nothing reached the window");
}

#[test]
fn a_reply_of_a_bad_length_or_type_ends_the_exchange() {
    let (exchange, ids) = (exchange(), ids());
    let raw = |length: u32, body: &[u8]| {
        let mut bytes = length.to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    };
    for (reply, expected) in [
        (raw(0, &[12]), PageantError::BadReplyLength(0)),
        (
            raw(MAX_PAYLOAD as u32 + 1, &[12]),
            PageantError::BadReplyLength(MAX_PAYLOAD as u32 + 1),
        ),
        (raw(u32::MAX, &[12]), PageantError::BadReplyLength(u32::MAX)),
        (raw(1, &[0x99]), PageantError::BadReplyType(0x99)),
        (raw(5, &[1, 0, 0, 0, 0]), PageantError::BadReplyType(1)),
    ] {
        let pageant = Receiver::start(Reply::Raw(reply));
        assert_eq!(
            exchange
                .send(pageant.window, &ids, &[11], BOUND)
                .unwrap_err(),
            expected
        );
    }
}

#[test]
fn pageant_refusing_the_request_is_reported() {
    let pageant = Receiver::start(Reply::Refuse);
    assert_eq!(
        exchange()
            .send(pageant.window, &ids(), &[11], BOUND)
            .unwrap_err(),
        PageantError::Rejected
    );
}

#[test]
fn a_window_of_the_calling_thread_is_refused_rather_than_waited_for() {
    // SAFETY: a message-only window of the system's `STATIC` class, made and destroyed on this thread.
    unsafe {
        let class = wide("STATIC");
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        assert!(!window.is_null());
        let begun = Instant::now();
        let error = exchange().send(window, &ids(), &[11], BOUND).unwrap_err();
        DestroyWindow(window);
        assert_eq!(error, PageantError::SameQueue);
        assert!(begun.elapsed() < Duration::from_secs(1));
    }
}

#[test]
fn a_window_that_is_gone_is_reported() {
    let window: HWND = {
        let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
        pageant.window
    };
    assert_eq!(
        exchange().send(window, &ids(), &[11], BOUND).unwrap_err(),
        PageantError::NoWindow
    );
    assert_eq!(
        exchange()
            .send(ptr::null_mut(), &ids(), &[11], BOUND)
            .unwrap_err(),
        PageantError::NoWindow
    );
}
