//! The Windows pipe's own rows: the flags it is created with, the first-instance squat, the DACL, the remote refusal.
use super::super::{
    CarrierLimits,
    local::{
        LocalListener, LocalRefusal, PIPE_MODE, PIPE_OPEN_MODE, PeerPolicy, Principal,
        create_first_instance, created_pipe_dacl_sids, own_identity, pipe_sddl,
    },
};
use super::{local_rows::*, support::*};
use windows_sys::Win32::{
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED},
    System::Pipes::{PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE},
};

#[test]
fn the_pipe_is_created_overlapped_first_instance_and_rejecting_remote_clients() {
    assert_ne!(PIPE_OPEN_MODE & FILE_FLAG_FIRST_PIPE_INSTANCE, 0);
    assert_ne!(PIPE_OPEN_MODE & FILE_FLAG_OVERLAPPED, 0);
    assert_ne!(PIPE_MODE & PIPE_REJECT_REMOTE_CLIENTS, 0);
    // A byte pipe that waits: the two flags that would say otherwise are absent (byte type and blocking wait are 0).
    assert_eq!(PIPE_MODE & PIPE_TYPE_MESSAGE, 0);
    assert_eq!(PIPE_MODE & PIPE_NOWAIT, 0);
}

#[test]
fn the_sddl_grants_one_sid_and_protects_the_acl() {
    let sddl = pipe_sddl("S-1-5-21-1-2-3-1001");
    assert_eq!(sddl, "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)");
    assert_eq!(sddl.matches("(A;").count(), 1);
}

#[test]
fn a_pipe_name_that_is_taken_cannot_be_created_again() {
    let name = format!(r"\\.\pipe\gwz-test-{}", std::process::id());
    let _first = create_first_instance(&name).expect("the first instance");
    let second = create_first_instance(&name)
        .err()
        .expect("a squatted name must fail");
    assert_eq!(second, LocalRefusal::NameTaken);
}

#[test]
fn the_created_pipe_grants_access_to_this_users_sid_alone() {
    let name = format!(r"\\.\pipe\gwz-test-dacl-{}", std::process::id());
    let pipe = create_first_instance(&name).expect("created");
    let Principal::Sid(own) = own_identity().unwrap().principal else {
        panic!("a SID")
    };
    assert_eq!(created_pipe_dacl_sids(&pipe), vec![own]);
}

#[test]
fn this_process_has_a_user_sid_and_an_integrity_level() {
    let identity = own_identity().unwrap();
    let Principal::Sid(sid) = &identity.principal else {
        panic!("a SID")
    };
    assert!(sid.starts_with("S-1-"), "{sid}");
    assert!(identity.integrity.is_some());
    assert!(!identity.app_container);
    println!("own identity: {identity:?}");
}

#[test]
fn a_remote_open_of_the_pipe_through_the_smb_loopback_is_refused() {
    let dir = dir();
    let listener = LocalListener::bind(
        &dir_runtime(&dir),
        PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("bind");
    let local = listener.address().pipe_name().to_string();
    let leaf = local.rsplit('\\').next().unwrap();
    let mut outcomes = Vec::new();
    for host in ["127.0.0.1", "localhost"] {
        let remote = format!(r"\\{host}\pipe\{leaf}");
        let result = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&remote);
        outcomes.push((remote, result.map(|_| ()).map_err(|e| e.raw_os_error())));
    }
    for (remote, outcome) in &outcomes {
        println!("remote open {remote}: {outcome:?}");
        // Opened would be the failure. Access denied (5) is the pipe's refusal of a remote client; bad netpath (53),
        // bad network name (67) and the like mean the SMB loopback does not reach it on this host, which the evidence
        // README records as a limit rather than as a pass of the flag.
        assert!(outcome.is_err(), "{remote} opened");
    }
}
