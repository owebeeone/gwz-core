//! The macOS probe's binding to the hold (step 2.3a remediation round 1: code C-2 and state P3-3). The probe asks about
//! the token the hold recorded, and a sandbox answer for a process that is no longer the one held is not read as a
//! sandbox profile.
use super::super::local::{
    Hold, PeerRefusal, Principal, peer_of,
    sandbox::{
        Cause, SandboxFindings,
        macos::{AuditToken, Subject, of_peer_with, probe_with},
    },
};
use super::local_rows::*;
use std::{
    ffi::{CStr, c_char, c_int, c_void},
    os::{
        fd::AsRawFd,
        unix::net::{UnixListener, UnixStream},
    },
};

/// Answers 1 when the token is this process's (word 5 is the process ID), else 0.
extern "C" fn this_process_by_token(token: AuditToken, _: *const c_char, _: c_int) -> c_int {
    c_int::from(token.0[5] == std::process::id())
}

/// Answers "sandboxed" whatever it is asked.
extern "C" fn sandboxed_by_token(_: AuditToken, _: *const c_char, _: c_int) -> c_int {
    3
}

/// Answers "not sandboxed": the answer a probe by process ID would give for the rows' process.
extern "C" fn nothing_by_pid(_: libc::pid_t, _: *const c_char, _: c_int) -> c_int {
    0
}

#[test]
fn the_probe_of_a_peer_asks_about_the_token_the_hold_recorded() {
    let dir = dir();
    let path = dir_runtime(&dir).path().join("probe-row");
    let socket = UnixListener::bind(&path).unwrap();
    let _client = UnixStream::connect(&path).unwrap();
    let (stream, _) = socket.accept().unwrap();
    let fd = stream.as_raw_fd();
    let peer = peer_of(fd).unwrap();
    let (Some(pid), Principal::Uid(uid)) = (peer.pid, peer.principal) else {
        panic!("a pid and a uid")
    };
    let hold = Hold::take(fd, pid, uid).expect("a hold on this process");
    let resolve = |name: &CStr| match name.to_str().unwrap() {
        "sandbox_check_by_audit_token" => this_process_by_token as *mut c_void,
        _ => nothing_by_pid as *mut c_void,
    };
    // The token form is asked about the hold's token and says sandboxed; a probe by process ID would say clean.
    let found = of_peer_with(&resolve, &hold, fd);
    assert_eq!(found.unwrap().cause(), Some(Cause::Profile));
}

#[test]
fn a_sandbox_answer_for_a_peer_that_is_no_longer_held_is_unreadable() {
    let resolve = |name: &CStr| match name.to_str().unwrap() {
        "sandbox_check_by_audit_token" => sandboxed_by_token as *mut c_void,
        _ => nothing_by_pid as *mut c_void,
    };
    let subject = || Subject::Token {
        token: AuditToken([0; 8]),
        pid: 0,
    };
    let gone = || Err(PeerRefusal::Unreadable("a live peer process"));
    let stale = probe_with(&resolve, subject(), &gone);
    assert_eq!(stale, Err(PeerRefusal::Unreadable("a live peer process")));
    // While it is held, the same answer is a sandbox profile.
    let held = probe_with(&resolve, subject(), &|| Ok(()));
    assert_eq!(held.unwrap().cause(), Some(Cause::Profile));
    // A clean answer is not asked to confirm anything.
    let none = |_: &CStr| nothing_by_pid as *mut c_void;
    let clean = probe_with(&none, Subject::Pid(1), &gone);
    assert_eq!(clean.unwrap(), SandboxFindings::default());
}
