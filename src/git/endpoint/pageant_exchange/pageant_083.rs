//! The exchange against Pageant 0.83 itself (PuTTY's own `pageant.exe`), the rows the TR1.8 baseline executed with a
//! script (P01's mapping name, descriptor and signature; the deferred-decryption prompt's timeout and the mapping's
//! life), re-run through the product code. They need the pinned binary and two disposable keys, which no checkout
//! holds, so they are ignored unless asked for:
//!
//! `GWZ_TEST_PAGEANT_DIR=<dir with pageant.exe, rsa.ppk and encrypted.ppk> cargo test --lib pageant_083 -- --ignored`
//!
//! `rsa.ppk` is an unencrypted RSA key; `encrypted.ppk` a passphrase-protected one that Pageant is started with
//! `-encrypted` to load, so it asks for the passphrase on each use, in a dialog nobody answers. Each test starts its
//! own Pageant and stops it by its exact process id; a Pageant already running refuses the test.
use super::*;
use std::{
    path::PathBuf,
    process::{Child, Command},
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND},
    System::Memory::{FILE_MAP_ALL_ACCESS, OpenFileMappingW},
    UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId},
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn find_pageant() -> HWND {
    let (class, title) = (wide("Pageant"), wide("Pageant"));
    // SAFETY: looks a window up by class and title.
    unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) }
}

/// A Pageant this test started, which it stops by its process id when dropped.
struct Pageant {
    child: Child,
    window: HWND,
}

impl Pageant {
    fn start(arguments: &[&str]) -> Self {
        let directory = PathBuf::from(
            std::env::var_os("GWZ_TEST_PAGEANT_DIR")
                .expect("GWZ_TEST_PAGEANT_DIR names the directory of pageant.exe and the keys"),
        );
        assert!(
            find_pageant().is_null(),
            "a Pageant is running already; the test will not use or stop it"
        );
        let mut command = Command::new(directory.join("pageant.exe"));
        for argument in arguments {
            // An option goes as it is; anything else names a file of the directory.
            if argument.starts_with('-') {
                command.arg(argument);
            } else {
                command.arg(directory.join(argument));
            }
        }
        let child = command.spawn().expect("pageant.exe starts");
        let mut pageant = Self {
            child,
            window: ptr::null_mut(),
        };
        let until = Instant::now() + Duration::from_secs(8);
        while pageant.window.is_null() {
            assert!(Instant::now() < until, "Pageant made no window");
            std::thread::sleep(Duration::from_millis(50));
            let window = find_pageant();
            let mut owner = 0u32;
            // SAFETY: reads the window's process, which may be gone.
            unsafe {
                if !window.is_null() {
                    GetWindowThreadProcessId(window, &mut owner);
                }
            }
            if !window.is_null() && owner == pageant.child.id() {
                pageant.window = window;
            }
        }
        // Pageant loads its keys after it makes its window.
        std::thread::sleep(Duration::from_millis(300));
        pageant
    }
}

impl Drop for Pageant {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn string(bytes: &[u8]) -> Vec<u8> {
    let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(bytes);
    out
}

/// The key blob of the first identity in an identities answer (type 12), and the number of identities.
fn first_identity(answer: &[u8]) -> (u32, Vec<u8>) {
    assert_eq!(answer[0], 12, "an identities answer");
    let count = u32::from_be_bytes(answer[1..5].try_into().unwrap());
    let length = u32::from_be_bytes(answer[5..9].try_into().unwrap()) as usize;
    (count, answer[9..9 + length].to_vec())
}

/// The algorithm name inside a sign response (type 14): its signature blob is a string of `string(name) string(sig)`.
fn signature_algorithm(answer: &[u8]) -> String {
    assert_eq!(
        answer[0],
        14,
        "a sign response: {:?}",
        &answer[..answer.len().min(8)]
    );
    let blob_length = u32::from_be_bytes(answer[1..5].try_into().unwrap()) as usize;
    let blob = &answer[5..5 + blob_length];
    let name_length = u32::from_be_bytes(blob[..4].try_into().unwrap()) as usize;
    String::from_utf8(blob[4..4 + name_length].to_vec()).unwrap()
}

fn sign_request(blob: &[u8], flags: u32) -> Vec<u8> {
    let mut request = vec![13];
    request.extend_from_slice(&string(blob));
    request.extend_from_slice(&string(b"gwz step 3.5 disposable signature probe"));
    request.extend_from_slice(&flags.to_be_bytes());
    request
}

#[test]
#[ignore = "needs PuTTY 0.83's pageant.exe and disposable keys (GWZ_TEST_PAGEANT_DIR)"]
fn pageant_083_lists_and_signs_through_the_exchange_on_a_local_mapping() {
    let pageant = Pageant::start(&["rsa.ppk"]);
    let (exchange, ids) = (
        Exchange::new().unwrap(),
        crate::operation_context::new_id_source(),
    );
    let bound = Duration::from_secs(10);
    let listed = exchange.send(pageant.window, &ids, &[11], bound).unwrap();
    let (count, blob) = first_identity(&listed);
    assert_eq!(count, 1);
    assert_eq!(&blob[4..11], b"ssh-rsa");
    // RSA signs at the flag asked: SHA-256 (2) and SHA-512 (4).
    for (flags, name) in [(2, "rsa-sha2-256"), (4, "rsa-sha2-512")] {
        let signed = exchange
            .send(pageant.window, &ids, &sign_request(&blob, flags), bound)
            .unwrap();
        assert_eq!(signature_algorithm(&signed), name);
    }
    // A request Pageant cannot serve is answered with its failure message, which is a reply like any other.
    let failed = exchange
        .send(pageant.window, &ids, &sign_request(&blob[..8], 0), bound)
        .unwrap();
    assert_eq!(failed[0], 5);
}

#[test]
#[ignore = "needs PuTTY 0.83's pageant.exe and disposable keys (GWZ_TEST_PAGEANT_DIR)"]
fn pageant_083_prompt_times_out_and_the_mapping_outlives_the_sender_until_pageant_is_stopped() {
    let mut pageant = Pageant::start(&["-encrypted", "encrypted.ppk"]);
    let (exchange, ids) = (
        Exchange::new().unwrap(),
        crate::operation_context::new_id_source(),
    );
    // Listing does not decrypt; signing asks for the passphrase in a dialog nobody answers.
    let listed = exchange
        .send(pageant.window, &ids, &[11], Duration::from_secs(10))
        .unwrap();
    let (count, blob) = first_identity(&listed);
    assert_eq!(count, 1);
    let name = mapping_name(std::process::id(), &ids);
    let begun = Instant::now();
    let error = exchange
        .send_named(
            pageant.window,
            &name,
            &sign_request(&blob, 2),
            Duration::from_millis(1500),
        )
        .unwrap_err();
    assert_eq!(error, PageantError::Timeout);
    assert!(
        begun.elapsed() < Duration::from_secs(4),
        "{:?}",
        begun.elapsed()
    );
    let wide_name = wide(&name);
    let exists = || {
        // SAFETY: opens a mapping by name and closes it again.
        unsafe {
            let handle = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, wide_name.as_ptr());
            let found = !handle.is_null();
            if found {
                CloseHandle(handle);
            }
            found
        }
    };
    // The sender has closed its handle; Pageant still holds the request's mapping while its prompt is open.
    assert!(exists(), "Pageant holds the mapping while it prompts");
    pageant.child.kill().unwrap();
    pageant.child.wait().unwrap();
    assert!(!exists(), "the mapping is gone once Pageant is");
}
