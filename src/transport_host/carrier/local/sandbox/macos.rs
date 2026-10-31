//! The macOS probe: `sandbox_check_by_audit_token(token, NULL, 0)` on the token the hold recorded, else
//! `sandbox_check(pid, NULL, 0)`. Apple documents neither; both are found with `dlsym` so that a system without one
//! refuses instead of failing to link. A non-zero answer is a sandbox; a missing symbol or a failed call refuses.
use super::super::unix::Hold;
use super::{PeerRefusal, SandboxFindings};
use std::{
    ffi::{CStr, c_char, c_int, c_void},
    os::fd::RawFd,
};

/// `audit_token_t`, passed by value.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct AuditToken(pub(crate) [u32; 8]);

/// Both functions are variadic in `<sandbox.h>`; with the filter type 0 (none) they read no variadic argument.
type CheckByToken = unsafe extern "C" fn(AuditToken, *const c_char, c_int) -> c_int;
type CheckByPid = unsafe extern "C" fn(libc::pid_t, *const c_char, c_int) -> c_int;

/// Finds a symbol: null when there is none. The rows pass their own.
pub(crate) type Resolve<'a> = &'a dyn Fn(&CStr) -> *mut c_void;

/// Which process to ask about. A token names the process exactly; a process ID is used only where there is no token.
pub(crate) enum Subject {
    Token { token: AuditToken, pid: u32 },
    Pid(u32),
}

fn dlsym_default(name: &CStr) -> *mut c_void {
    // SAFETY: `name` is NUL-terminated; `RTLD_DEFAULT` searches the images already loaded.
    unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) }
}

/// The findings for `subject`, the symbols found with `resolve`. A sandbox is reported only while `holds` says the process
/// asked about is still the one held: the answer for a process that has gone, or whose ID names another, is not about the
/// peer, and is refused as unreadable instead.
pub(crate) fn probe_with(
    resolve: Resolve,
    subject: Subject,
    holds: &dyn Fn() -> Result<(), PeerRefusal>,
) -> Result<SandboxFindings, PeerRefusal> {
    let by_pid = |pid: u32| {
        let symbol = resolve(c"sandbox_check");
        if symbol.is_null() {
            return Err(PeerRefusal::Unreadable("the sandbox check"));
        }
        // SAFETY: `sandbox_check` has this signature; the operation is NULL and the type 0.
        let call = unsafe { std::mem::transmute::<*mut c_void, CheckByPid>(symbol) };
        Ok(unsafe { call(pid as libc::pid_t, std::ptr::null(), 0) })
    };
    let answer = match subject {
        Subject::Token { token, pid } => {
            let symbol = resolve(c"sandbox_check_by_audit_token");
            if symbol.is_null() {
                by_pid(pid)?
            } else {
                // SAFETY: as above, for the token form.
                let call = unsafe { std::mem::transmute::<*mut c_void, CheckByToken>(symbol) };
                unsafe { call(token, std::ptr::null(), 0) }
            }
        }
        Subject::Pid(pid) => by_pid(pid)?,
    };
    match answer {
        0 => Ok(SandboxFindings::default()),
        1.. => {
            holds()?;
            Ok(SandboxFindings {
                profile: true,
                ..SandboxFindings::default()
            })
        }
        _ => Err(PeerRefusal::Unreadable("the sandbox check's answer")),
    }
}

/// The peer held by `hold` on the socket `fd`, through its audit token, the symbols found with `resolve`.
pub(crate) fn of_peer_with(
    resolve: Resolve,
    hold: &Hold,
    fd: RawFd,
) -> Result<SandboxFindings, PeerRefusal> {
    let subject = Subject::Token {
        token: AuditToken(hold.token()),
        pid: hold.pid(),
    };
    probe_with(resolve, subject, &|| hold.check(fd))
}

/// The peer held by `hold` on the socket `fd`.
pub(crate) fn of_peer(hold: &Hold, fd: RawFd) -> Result<SandboxFindings, PeerRefusal> {
    of_peer_with(&dlsym_default, hold, fd)
}

/// This process, by its ID. It is there: it is asking.
pub(crate) fn of_self() -> Result<SandboxFindings, PeerRefusal> {
    probe_with(&dlsym_default, Subject::Pid(std::process::id()), &|| Ok(()))
}
