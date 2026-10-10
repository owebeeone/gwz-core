//! The exchange's Windows calls: the caller's identity, the mapping with its descriptor, and the one
//! `SendMessageTimeoutW`.
use super::{
    COPYDATA_ID, MAPPING_SIZE, PageantError, descriptor, frame, mapping_name, reply, timeout_ms,
    valid_name,
};
use gwz_ids::IdSource;
use std::{ptr, slice, time::Duration};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE,
        LocalFree,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::{
        DataExchange::COPYDATASTRUCT,
        Memory::{
            CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
        },
        Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, OpenProcessToken},
    },
    UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, IsWindow, SMTO_BLOCK, SMTO_ERRORONEXIT, SendMessageTimeoutW,
        WM_COPYDATA,
    },
};

/// `ERROR_TIMEOUT`, the error `SendMessageTimeoutW` leaves when its bound passes.
const ERROR_TIMEOUT: u32 = 1460;

/// A handle that closes when dropped.
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle this value owns and closes once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// A view of a mapping that unmaps when dropped.
struct View(*mut core::ffi::c_void);
impl Drop for View {
    fn drop(&mut self) {
        // SAFETY: a view this value owns and unmaps once.
        unsafe {
            UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS { Value: self.0 },
            );
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn last_error() -> PageantError {
    // SAFETY: reads the calling thread's last error.
    PageantError::Os(unsafe { GetLastError() })
}

/// The caller's user SID in its string form, `S-1-5-21-...`.
fn user_sid() -> Result<String, PageantError> {
    // SAFETY: the token handle is closed by its guard; the buffer is `TOKEN_USER`-aligned and sized by the first
    // call; the string `ConvertSidToStringSidW` allocates is copied and freed here.
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(last_error());
        }
        let _token = Handle(token);
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return Err(last_error());
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        if GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(last_error());
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(last_error());
        }
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let sid = String::from_utf16_lossy(slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        Ok(sid)
    }
}

/// One caller's way to talk to Pageant: it holds the caller's identity, which every mapping is made for.
pub(crate) struct Exchange {
    sid: String,
}

impl Exchange {
    pub(crate) fn new() -> Result<Self, PageantError> {
        Ok(Self { sid: user_sid()? })
    }

    pub(super) fn sid(&self) -> &str {
        &self.sid
    }

    /// One request to the Pageant `window` and its reply, bounded by `bound`, on a mapping named from `ids` and so used
    /// by this request alone.
    #[allow(dead_code)]
    pub(crate) fn send(
        &self,
        window: HWND,
        ids: &IdSource,
        request: &[u8],
        bound: Duration,
    ) -> Result<Vec<u8>, PageantError> {
        // SAFETY: reads this process's id.
        let process = unsafe { GetCurrentProcessId() };
        self.send_named(window, &mapping_name(process, ids), request, bound)
    }

    /// [`Self::send`] on the mapping `name`, which must be one request's alone: a name already in use is refused.
    pub(super) fn send_named(
        &self,
        window: HWND,
        name: &str,
        request: &[u8],
        bound: Duration,
    ) -> Result<Vec<u8>, PageantError> {
        if !valid_name(name) {
            return Err(PageantError::BadName);
        }
        let framed = frame(request)?;
        // SAFETY: only reads the window's thread, which may be gone.
        let owner = unsafe {
            if window.is_null() || IsWindow(window) == 0 {
                return Err(PageantError::NoWindow);
            }
            GetWindowThreadProcessId(window, ptr::null_mut())
        };
        // SAFETY: reads the calling thread's id.
        if owner == unsafe { GetCurrentThreadId() } {
            return Err(PageantError::SameQueue);
        }
        let mapping = self.mapping(name)?;
        // SAFETY: a view of the whole mapping, `MAPPING_SIZE` bytes, unmapped by its guard.
        let view = unsafe { MapViewOfFile(mapping.0, FILE_MAP_ALL_ACCESS, 0, 0, MAPPING_SIZE) };
        if view.Value.is_null() {
            return Err(last_error());
        }
        let view = View(view.Value);
        // SAFETY: the view is `MAPPING_SIZE` writable bytes that nothing else maps until the message is sent.
        let bytes = unsafe { slice::from_raw_parts_mut(view.0.cast::<u8>(), MAPPING_SIZE) };
        bytes[..framed.len()].copy_from_slice(&framed);
        let mut name_nul = name.as_bytes().to_vec();
        name_nul.push(0);
        let data = COPYDATASTRUCT {
            dwData: COPYDATA_ID,
            cbData: name_nul.len() as u32,
            lpData: name_nul.as_mut_ptr().cast(),
        };
        let mut served = 0usize;
        // SAFETY: one send, to this window and no other, of a `COPYDATASTRUCT` that outlives the call. A blocked
        // window that times out has been copied the message already; nothing here is read or reused after that.
        let sent = unsafe {
            SendMessageTimeoutW(
                window,
                WM_COPYDATA,
                0,
                ptr::from_ref(&data) as isize,
                SMTO_BLOCK | SMTO_ERRORONEXIT,
                timeout_ms(bound),
                &mut served,
            )
        };
        if sent == 0 {
            // SAFETY: reads the calling thread's last error.
            return Err(match unsafe { GetLastError() } {
                ERROR_TIMEOUT => PageantError::Timeout,
                _ => PageantError::NoWindow,
            });
        }
        // Pageant has returned: it is done with the mapping, which may now be read and wiped.
        let result = if served == 0 {
            Err(PageantError::Rejected)
        } else {
            reply(bytes).map(<[u8]>::to_vec)
        };
        bytes.fill(0);
        result
    }

    /// The mapping `name`, made now, owned by the caller and open to the caller and SYSTEM alone; refused if the name
    /// exists already.
    fn mapping(&self, name: &str) -> Result<Handle, PageantError> {
        let sddl = wide(&descriptor(&self.sid));
        let wide_name = wide(name);
        // SAFETY: the descriptor is converted from a string this function owns and freed after the mapping is made,
        // which copies it; the mapping name is NUL-terminated.
        unsafe {
            let mut security = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut security,
                ptr::null_mut(),
            ) == 0
            {
                return Err(last_error());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: security,
                bInheritHandle: 0,
            };
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                &attributes,
                PAGE_READWRITE,
                0,
                MAPPING_SIZE as u32,
                wide_name.as_ptr(),
            );
            let error = GetLastError();
            LocalFree(security);
            if handle.is_null() {
                return Err(PageantError::Os(error));
            }
            let handle = Handle(handle);
            if error == ERROR_ALREADY_EXISTS {
                return Err(PageantError::NameInUse);
            }
            Ok(handle)
        }
    }
}
