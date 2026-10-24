//! The Windows process gate: a named kernel semaphore with a name of this process's own (operator decision D-E; contract
//! 7.3 of dev-docs/GwzConnectionApiContract.md).
//!
//! Every gwz host in the process opens the same object by name, so a creation window one holds excludes every other host's
//! spawn, whichever host made its gate. The object is a semaphore, not a mutex, because a hold is moved to the thread that
//! launches and a mutex must be released by the thread that took it.
//!
//! **Fail closed.** The gate is unavailable when the object cannot be created or opened, and when it exists with an owner
//! that is not this process's own (another account made an object of the name before this process did). An unavailable gate
//! never claims exclusion; native authentication, which needs the window, refuses before it reads a credential.
//!
//! **Residual.** The claim is scoped to gwz callers. A thread that does not take the gate (code gwz does not write, a
//! library's own `CreateProcess` with handle inheritance) is not excluded, and a process of the same account could create the
//! name first with an owner that matches; the name carries the process id so that the second needs to know it in advance.
use super::GateFault;
use std::{ffi::c_void, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::{
        Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT},
        EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID, TOKEN_OWNER, TOKEN_QUERY,
        TokenOwner,
    },
    System::Threading::{
        CreateSemaphoreW, GetCurrentProcess, GetCurrentProcessId, INFINITE, OpenProcessToken,
        ReleaseSemaphore, WaitForSingleObject,
    },
};

/// Whose object the gate accepts.
pub(super) enum Expect {
    /// This process's default owner: the SID an object it creates is owned by.
    Current,
    /// A SID a test names (a foreign one, to show the refusal).
    Sid(PSID),
}

/// The object. The handle closes when the last holder of the gate goes.
pub(super) struct Kernel {
    handle: HANDLE,
}
// SAFETY: a semaphore handle is usable from any thread; the value owns it and closes it once.
unsafe impl Send for Kernel {}
// SAFETY: the calls made through the handle are thread-safe kernel calls.
unsafe impl Sync for Kernel {}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// This process's gate name.
pub(super) fn process_name() -> String {
    // SAFETY: reads this process's id.
    format!("Local\\gwz-spawn-gate-{}", unsafe { GetCurrentProcessId() })
}

impl Kernel {
    /// The process's gate: created, or opened if an earlier host of this process made it.
    pub(super) fn open_process() -> Result<Self, GateFault> {
        Self::open_named(&process_name(), Expect::Current)
    }

    /// The semaphore `name`, created with one free place if it is new, accepted only if its owner is `expect`.
    pub(super) fn open_named(name: &str, expect: Expect) -> Result<Self, GateFault> {
        let name = wide(name);
        // SAFETY: a NUL-terminated name that outlives the call; no security attributes, so the default descriptor.
        let handle = unsafe { CreateSemaphoreW(ptr::null(), 1, 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(GateFault::NotCreated);
        }
        let kernel = Self { handle };
        if owner_matches(handle, &expect) {
            Ok(kernel)
        } else {
            // `kernel` closes the handle as it drops.
            Err(GateFault::ForeignOwner)
        }
    }

    /// Waits for the free place and takes it. `false` only if the wait itself failed, and nothing is then held.
    pub(super) fn acquire(&self) -> bool {
        // SAFETY: a live semaphore handle this value owns.
        unsafe { WaitForSingleObject(self.handle, INFINITE) == WAIT_OBJECT_0 }
    }

    pub(super) fn try_acquire(&self) -> bool {
        // SAFETY: as `acquire`.
        let waited = unsafe { WaitForSingleObject(self.handle, 0) };
        debug_assert!(waited == WAIT_OBJECT_0 || waited == WAIT_TIMEOUT);
        waited == WAIT_OBJECT_0
    }

    /// Gives the place back. Only the holder calls it, once per acquire.
    pub(super) fn release(&self) {
        // SAFETY: as `acquire`; a null previous count is allowed.
        unsafe {
            ReleaseSemaphore(self.handle, 1, ptr::null_mut());
        }
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        // SAFETY: a handle this value owns and closes once.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

/// This process's default owner SID, in a buffer that holds a `TOKEN_OWNER` and the SID it points into.
fn current_owner() -> Option<Vec<u64>> {
    // SAFETY: the token handle is closed before returning; the buffer is 8-aligned and sized by the first call.
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut size = 0u32;
        GetTokenInformation(token, TokenOwner, ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
        let read = size != 0
            && GetTokenInformation(
                token,
                TokenOwner,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            ) != 0;
        CloseHandle(token);
        read.then_some(buffer)
    }
}

fn owner_matches(handle: HANDLE, expect: &Expect) -> bool {
    // SAFETY: `owner` points into the security descriptor `descriptor`, which is freed before returning; the expected SID
    // lives in `current` (or is the caller's) for the comparison.
    unsafe {
        let mut owner: PSID = ptr::null_mut();
        let mut descriptor: *mut c_void = ptr::null_mut();
        let status = GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        );
        if status != 0 || owner.is_null() {
            return false;
        }
        let same = match expect {
            Expect::Current => match current_owner() {
                Some(buffer) => {
                    let token_owner = &*(buffer.as_ptr() as *const TOKEN_OWNER);
                    EqualSid(owner, token_owner.Owner) != 0
                }
                None => false,
            },
            Expect::Sid(sid) => EqualSid(owner, *sid) != 0,
        };
        LocalFree(descriptor);
        same
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
