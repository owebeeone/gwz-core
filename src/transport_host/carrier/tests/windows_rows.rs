//! The Windows pipe's own rows: the flags it is created with, the first-instance squat, the DACL, the remote refusal.
use super::super::{
    CarrierLimits,
    local::{
        Integrity, LocalAddress, LocalListener, LocalRefusal, PIPE_MODE, PIPE_OPEN_MODE,
        PeerPolicy, PeerRefusal, Principal, ProcessHold, connect, create_first_instance,
        create_first_instance_with, created_pipe_dacl_sids, own_identity, pipe_sddl,
    },
};
use super::{local_rows::*, support::*};
use std::{
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{
        Authorization::ConvertStringSidToSidW, DuplicateTokenEx, GetLengthSid, SID_AND_ATTRIBUTES,
        SecurityImpersonation, SetTokenInformation, TOKEN_ALL_ACCESS, TOKEN_MANDATORY_LABEL,
        TokenIntegrityLevel, TokenPrimary,
    },
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED},
    System::{
        Pipes::{PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE},
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, GetCurrentProcess,
            OpenProcessToken, PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess,
        },
    },
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

/// What opening `\\<host>\pipe\<leaf>` through the SMB loopback did, for each host spelling.
fn remote_opens(leaf: &str) -> Vec<(String, Result<(), Option<i32>>)> {
    ["127.0.0.1", "localhost"]
        .iter()
        .map(|host| {
            let remote = format!(r"\\{host}\pipe\{leaf}");
            let result = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&remote);
            let outcome = result.map(|_| ()).map_err(|error| error.raw_os_error());
            println!("remote open {remote}: {outcome:?}");
            (remote, outcome)
        })
        .collect()
}

fn leaf_of(name: &str) -> &str {
    name.rsplit('\\').next().unwrap()
}

/// `ERROR_ACCESS_DENIED`.
const DENIED: Option<i32> = Some(5);

#[test]
fn a_remote_open_of_the_pipe_is_refused_with_access_denied() {
    let dir = dir();
    let listener = LocalListener::bind(
        dir_runtime(&dir),
        PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("bind");
    let local = listener.address().pipe_name().to_string();
    for (remote, outcome) in remote_opens(leaf_of(&local)) {
        // Exactly the pipe's refusal: not "no route" (53) or "bad network name" (67), which would mean the loopback
        // never reached the pipe and the row proved nothing.
        assert_eq!(outcome, Err(DENIED), "{remote}");
    }
}

/// Everyone and ANONYMOUS LOGON, full access: a descriptor that grants every remote caller what the user's own SID
/// would, so that a refusal under it is not the descriptor's.
const PERMISSIVE: &str = "D:P(A;;GA;;;WD)(A;;GA;;;AN)";

#[test]
fn the_remote_client_flag_alone_refuses_a_remote_open_and_clearing_it_changes_the_outcome() {
    let flagged = format!(r"\\.\pipe\gwz-test-flag-{}", std::process::id());
    let unflagged = format!(r"\\.\pipe\gwz-test-noflag-{}", std::process::id());
    let _flagged = create_first_instance_with(&flagged, PERMISSIVE, PIPE_MODE).expect("flagged");
    let _unflagged = create_first_instance_with(
        &unflagged,
        PERMISSIVE,
        PIPE_MODE & !PIPE_REJECT_REMOTE_CLIENTS,
    )
    .expect("unflagged");
    // With the flag set, a remote open is refused although the descriptor grants Everyone and ANONYMOUS LOGON.
    for (remote, outcome) in remote_opens(leaf_of(&flagged)) {
        assert_eq!(outcome, Err(DENIED), "{remote}");
    }
    // With the flag cleared and the same descriptor, the open is not refused the same way: so it is the flag that
    // refuses.
    for (remote, outcome) in remote_opens(leaf_of(&unflagged)) {
        assert_ne!(
            outcome,
            Err(DENIED),
            "{remote} behaved as if the flag were set"
        );
    }
}

// ---- a Low-integrity listener, end to end ------------------------------------------------------------------------------

/// `SE_GROUP_INTEGRITY`: the attribute a mandatory-label SID carries in a token.
const SE_GROUP_INTEGRITY: u32 = 0x20;

const LOW_PIPE: &str = "GWZ_CARRIER_ROW_LOW_PIPE";

fn child_test() -> String {
    let path = module_path!();
    let relative = path.split_once("::").map_or(path, |(_, rest)| rest);
    format!("{relative}::low_listener_child")
}

/// The other process of the row below, started at Low integrity: it holds the first instance of a pipe and waits to be
/// ended. Run on its own it does nothing and passes.
#[test]
fn low_listener_child() {
    let Some(name) = std::env::var_os(LOW_PIPE) else {
        return;
    };
    let identity = own_identity().unwrap();
    assert_eq!(
        identity.integrity,
        Some(Integrity::Low),
        "started at Low integrity"
    );
    let _pipe = create_first_instance(&name.to_string_lossy()).expect("the child's pipe");
    thread::sleep(Duration::from_secs(120));
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A primary token for this user at Low integrity, duplicated from this process's own.
fn low_integrity_token() -> HANDLE {
    // SAFETY: opens and duplicates this process's own token; the SID allocation is freed by the process's end, which is
    // the end of a test.
    unsafe {
        let mut own: HANDLE = ptr::null_mut();
        assert_ne!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut own),
            0,
            "OpenProcessToken"
        );
        let mut low: HANDLE = ptr::null_mut();
        assert_ne!(
            DuplicateTokenEx(
                own,
                TOKEN_ALL_ACCESS,
                ptr::null(),
                SecurityImpersonation,
                TokenPrimary,
                &mut low
            ),
            0,
            "DuplicateTokenEx"
        );
        CloseHandle(own);
        let mut sid = ptr::null_mut();
        assert_ne!(
            ConvertStringSidToSidW(wide("S-1-16-4096").as_ptr(), &mut sid),
            0,
            "ConvertStringSidToSidW"
        );
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: SE_GROUP_INTEGRITY,
            },
        };
        let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid);
        assert_ne!(
            SetTokenInformation(low, TokenIntegrityLevel, (&raw const label).cast(), size),
            0,
            "SetTokenInformation(TokenIntegrityLevel)"
        );
        low
    }
}

#[test]
fn a_listener_running_at_low_integrity_is_refused_by_the_client_end_to_end() {
    let name = format!(r"\\.\pipe\gwz-test-low-{}", std::process::id());
    let token = low_integrity_token();
    // The child's environment: this process's, plus the pipe's name.
    let mut environment: Vec<u16> = Vec::new();
    for (key, value) in std::env::vars_os().chain([(LOW_PIPE.into(), name.clone().into())]) {
        environment.extend(key.to_string_lossy().encode_utf16());
        environment.push(u16::from(b'='));
        environment.extend(value.to_string_lossy().encode_utf16());
        environment.push(0);
    }
    environment.push(0);
    let exe = std::env::current_exe().unwrap();
    let mut command = wide(&format!(
        "\"{}\" --exact {} --nocapture --test-threads=1",
        exe.display(),
        child_test()
    ));
    // SAFETY: plain data in, a process handle out; the command line is a writable buffer; the environment is a
    // double-NUL-terminated block of UTF-16 entries.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        let mut startup: STARTUPINFOW = std::mem::zeroed();
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        CreateProcessAsUserW(
            token,
            ptr::null(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            environment.as_ptr().cast(),
            ptr::null(),
            &startup,
            &mut info,
        )
    };
    assert_ne!(
        created,
        0,
        "CreateProcessAsUserW: {}",
        std::io::Error::last_os_error()
    );
    // The client waits for the child's pipe to exist, then connects: its check of the listener's process refuses.
    let address = LocalAddress::from_name(&name);
    let policy = PeerPolicy::current_user().unwrap();
    let deadline = Instant::now() + WAIT;
    let outcome = loop {
        match connect(&address, &policy, CarrierLimits::DEFAULT) {
            Err(LocalRefusal::Os { code: Some(2), .. }) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            other => break other,
        }
    };
    // SAFETY: the handles are this test's; the child is ended before they close.
    unsafe {
        TerminateProcess(info.hProcess, 1);
        CloseHandle(info.hProcess);
        CloseHandle(info.hThread);
        CloseHandle(token);
    }
    assert_eq!(
        outcome.err(),
        Some(LocalRefusal::Peer(PeerRefusal::Integrity)),
        "a Low-integrity listener is refused"
    );
}

#[test]
fn a_hold_on_a_process_is_given_up_when_the_process_goes() {
    let mut child = std::process::Command::new("ping")
        .args(["-n", "60", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("a process to hold");
    let pid = child.id();
    // The hold is taken while the process is there and confirms as long as it is.
    let hold = ProcessHold::take(pid).expect("a hold on a live process");
    assert!(hold.confirm().is_ok());
    // The process goes, and the hold no longer confirms; a new hold on it is not taken.
    child.kill().expect("end the process");
    child.wait().expect("reap it");
    assert!(hold.confirm().is_err(), "a gone process is not confirmed");
    assert!(
        ProcessHold::take(pid).is_err(),
        "and no hold is taken on it"
    );
}
