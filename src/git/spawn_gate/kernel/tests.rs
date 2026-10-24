//! The Windows process gate's object: exclusion across handles of one name, the fail-closed refusals (an owner that is not
//! ours, a name that cannot be made) and the per-process name (operator decision D-E).
use super::*;
use windows_sys::Win32::{Foundation::LocalFree, Security::Authorization::ConvertStringSidToSidW};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn two_handles_of_one_name_exclude_each_other_and_a_release_may_come_from_another_thread() {
    let first = Kernel::open_named("Local\\gwz-kernel-test-exclusion", Expect::Current).unwrap();
    let second = Kernel::open_named("Local\\gwz-kernel-test-exclusion", Expect::Current).unwrap();
    assert!(first.acquire());
    assert!(!second.try_acquire());
    std::thread::scope(|scope| {
        scope.spawn(|| first.release());
    });
    assert!(second.try_acquire());
    second.release();
}

#[test]
fn an_object_whose_owner_is_not_the_expected_one_fails_closed() {
    let mut sid = ptr::null_mut();
    // SAFETY: a NUL-terminated SID string; the SID it allocates is freed below.
    unsafe {
        assert!(ConvertStringSidToSidW(wide("S-1-5-18").as_ptr(), &mut sid) != 0);
    }
    let refused = Kernel::open_named("Local\\gwz-kernel-test-foreign", Expect::Sid(sid));
    assert!(matches!(refused, Err(GateFault::ForeignOwner)));
    // An object a process of ours made earlier is accepted when the owner is the expected one.
    let ours = Kernel::open_named("Local\\gwz-kernel-test-foreign", Expect::Current);
    assert!(ours.is_ok());
    // SAFETY: the SID `ConvertStringSidToSidW` allocated, freed once.
    unsafe {
        LocalFree(sid);
    }
}

#[test]
fn a_name_that_cannot_be_made_fails_closed() {
    let refused = Kernel::open_named("Local\\gwz\\no\\such\\directory", Expect::Current);
    assert!(matches!(refused, Err(GateFault::NotCreated)));
}

#[test]
fn the_process_name_is_this_processs_own() {
    let name = process_name();
    assert!(name.starts_with("Local\\gwz-spawn-gate-"));
    assert!(name.ends_with(&std::process::id().to_string()));
}

/// The process gate over the real object: handles made independently, as two hosts make them, share it.
#[test]
fn independent_process_gates_share_the_one_object() {
    use super::super::SpawnGate;
    let first = SpawnGate::process();
    let second = SpawnGate::process();
    assert!(first.fault().is_none() && second.fault().is_none());
    let held = first.hold();
    assert!(second.try_hold().is_none());
    drop(held);
    assert!(second.try_hold().is_some());
}
