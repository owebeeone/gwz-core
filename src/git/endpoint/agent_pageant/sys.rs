//! Pageant's Windows side (GwzTransportWindowsParityPlan.md, step 3.6): the probe that finds Pageant's window the way
//! Pageant's own protocol does, the pin of the process that owns it, and the exchange of step 3.5 under that pin.
use super::{
    super::pageant_exchange::{Exchange, PageantError},
    pin::{Holder, PageantPin, PageantWindow},
};
use std::time::Duration;
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, HWND, WAIT_TIMEOUT},
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    },
    UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, IsWindow},
};

/// A handle to Pageant's process, held so that the process object (and so its id) outlives the window, and closed when
/// the last share of the pin goes.
pub(crate) struct ProcessHandle(HANDLE);

// SAFETY: a process handle names a kernel object; waiting on it and closing it are safe from any thread, and it is
// closed once, on drop.
unsafe impl Send for ProcessHandle {}
unsafe impl Sync for ProcessHandle {}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: a handle this value owns and closes once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl Holder for ProcessHandle {
    fn running(&self) -> bool {
        // SAFETY: a zero-millisecond wait on a handle this value owns.
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
    }
}

impl ProcessHandle {
    /// The process's creation time, 100 ns since 1601, or 0 where it cannot be read.
    fn created(&self) -> u64 {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: four `FILETIME`s of this frame, written by the call.
        let read =
            unsafe { GetProcessTimes(self.0, &mut created, &mut exited, &mut kernel, &mut user) };
        if read == 0 {
            return 0;
        }
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The process that owns `window`, or `None` if it is not a window.
fn owner(window: HWND) -> Option<u32> {
    if window.is_null() {
        return None;
    }
    let mut process = 0u32;
    // SAFETY: both calls only read the window's owner, which may be gone.
    let thread = unsafe {
        if IsWindow(window) == 0 {
            return None;
        }
        GetWindowThreadProcessId(window, &mut process)
    };
    (thread != 0 && process != 0).then_some(process)
}

/// The Pageant window that `FindWindowW("Pageant", "Pageant")` finds in this desktop, with its owner pinned: Pageant's
/// own protocol finds its window this way, and libssh2's Pageant-first backend does too. "Visible" is what that finds,
/// not `IsWindowVisible`: the window can be hidden.
///
/// A process that cannot be opened (another user's, or a protected one) is still pinned, by its id and the window
/// alone: the transport refuses no Pageant for who owns it (TD11 B).
pub(crate) fn find() -> Option<PageantPin> {
    let (class, title) = (wide("Pageant"), wide("Pageant"));
    // SAFETY: looks a window up by class and title.
    let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    pin_of(window)
}

/// The pin of `window`'s owner.
pub(crate) fn pin_of(window: HWND) -> Option<PageantPin> {
    let process = owner(window)?;
    // SAFETY: opens a process by id with the rights to read its times and wait for its end.
    let opened = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            process,
        )
    };
    let held = (!opened.is_null()).then(|| ProcessHandle(opened));
    // The window may have changed hands between the lookup and the open, and the process id with it: what the handle
    // holds must still be the window's owner.
    if owner(window) != Some(process) {
        return None;
    }
    let created = held.as_ref().map_or(0, ProcessHandle::created);
    let holder = held.map(|handle| Box::new(handle) as Box<dyn Holder>);
    Some(PageantPin::new(window as usize, process, created, holder))
}

/// Pageant's window as the exchange reaches it, under its pin.
pub(crate) struct WindowsPageant {
    pin: PageantPin,
    exchange: Exchange,
}

/// The channel's window for `pin`: the caller's identity is read here, once, and every mapping is made for it.
pub(crate) fn open(pin: &PageantPin) -> Result<WindowsPageant, PageantError> {
    Ok(WindowsPageant {
        pin: pin.clone(),
        exchange: Exchange::new()?,
    })
}

impl PageantWindow for WindowsPageant {
    fn owner_pinned(&self) -> bool {
        let window = self.pin.window() as HWND;
        owner(window) == Some(self.pin.process())
            && self.pin.holder().is_none_or(|holder| holder.running())
    }

    fn send(&self, request: &[u8], bound: Duration) -> Result<Vec<u8>, PageantError> {
        self.exchange
            .send(self.pin.window() as HWND, self.pin.ids(), request, bound)
    }
}
