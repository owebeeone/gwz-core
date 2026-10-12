//! A synthetic Pageant for the exchange's tests: a message-only window on a thread of its own that answers
//! `WM_COPYDATA` as Pageant does (opens the named mapping, reads the request, writes a reply into it, returns), with
//! the behaviours the tests script. It records what it was sent and the security descriptor of the mapping it was
//! given, never anything else.
use super::{COPYDATA_ID, MAPPING_SIZE};
use std::{
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, LRESULT, LocalFree, WPARAM},
    Graphics::Gdi::HBRUSH,
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_KERNEL_OBJECT,
        },
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    },
    System::{
        DataExchange::COPYDATASTRUCT,
        Memory::{FILE_MAP_ALL_ACCESS, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile},
        Threading::GetCurrentThreadId,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
        GWLP_WNDPROC, GetMessageW, GetWindowLongPtrW, HWND_MESSAGE, MSG, PostThreadMessageW,
        RegisterClassW, SetWindowLongPtrW, WM_COPYDATA, WM_QUIT, WNDCLASSW,
    },
};

/// How the receiver answers a request.
pub(crate) enum Reply {
    /// Writes this agent message as the reply and says it served the request.
    Message(Vec<u8>),
    /// Writes these bytes at the head of the mapping as they are, a damaged frame among them.
    Raw(Vec<u8>),
    /// Writes the agent message that `answer` makes of the request, and says it served the request: a Pageant with a
    /// key, for the rows that sign.
    Handler(Box<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>),
    /// Says it will not serve the request, and writes nothing.
    Refuse,
    /// Keeps its window thread busy for `delay`, as a confirmation prompt does, then writes `message` and serves the
    /// request: the sender has long timed out.
    Late { delay: Duration, message: Vec<u8> },
}

/// What the receiver was sent by one request.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) name: String,
    pub(crate) request: Vec<u8>,
    /// The mapping's owner and DACL as an SDDL string.
    pub(crate) descriptor: String,
}

struct Shared {
    reply: Reply,
    seen: Mutex<Vec<Seen>>,
    late_written: AtomicBool,
}

pub(crate) struct Receiver {
    pub(crate) window: HWND,
    shared: Arc<Shared>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

/// The window handle crosses to the test's thread as a number.
struct Window(usize);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// What kind of window the receiver makes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A message-only window of the system's `STATIC` class, which nothing finds by class and title.
    Message,
    /// A hidden top-level window of the class `Pageant` and the title `Pageant`, as Pageant makes its own, so that the
    /// probe (`FindWindowW`) finds it.
    Pageant,
}

impl Receiver {
    pub(crate) fn start(reply: Reply) -> Self {
        Self::start_kind(reply, Kind::Message)
    }

    /// A receiver that `FindWindowW("Pageant", "Pageant")` finds. Windows of that name are found one at a time, so a
    /// test that starts one must be the only one in its process to do so.
    pub(crate) fn start_pageant(reply: Reply) -> Self {
        Self::start_kind(reply, Kind::Pageant)
    }

    fn start_kind(reply: Reply, kind: Kind) -> Self {
        let shared = Arc::new(Shared {
            reply,
            seen: Mutex::new(Vec::new()),
            late_written: AtomicBool::new(false),
        });
        let (ready, started) = std::sync::mpsc::channel();
        let owned = shared.clone();
        let thread = thread::spawn(move || {
            // SAFETY: a window on this thread, subclassed with `serve`; its user data is a pointer to `owned`, which
            // this thread holds until the window is destroyed.
            unsafe {
                let window = match kind {
                    Kind::Message => {
                        let class = wide("STATIC");
                        let title = wide("synthetic Pageant");
                        CreateWindowExW(
                            0,
                            class.as_ptr(),
                            title.as_ptr(),
                            0,
                            0,
                            0,
                            0,
                            0,
                            HWND_MESSAGE,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null(),
                        )
                    }
                    Kind::Pageant => {
                        let name = wide("Pageant");
                        let class = WNDCLASSW {
                            style: 0,
                            lpfnWndProc: Some(serve),
                            cbClsExtra: 0,
                            cbWndExtra: 0,
                            hInstance: ptr::null_mut(),
                            hIcon: ptr::null_mut(),
                            hCursor: ptr::null_mut(),
                            hbrBackground: ptr::null_mut::<core::ffi::c_void>() as HBRUSH,
                            lpszMenuName: ptr::null(),
                            lpszClassName: name.as_ptr(),
                        };
                        // A class of the name exists after the first receiver of the process: that registration
                        // serves this window as well.
                        RegisterClassW(&class);
                        CreateWindowExW(
                            0,
                            name.as_ptr(),
                            name.as_ptr(),
                            0,
                            0,
                            0,
                            0,
                            0,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null(),
                        )
                    }
                };
                assert!(!window.is_null(), "the synthetic Pageant window");
                SetWindowLongPtrW(window, GWLP_USERDATA, Arc::as_ptr(&owned) as isize);
                SetWindowLongPtrW(window, GWLP_WNDPROC, serve as *const () as usize as isize);
                ready
                    .send((Window(window as usize), GetCurrentThreadId()))
                    .unwrap();
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
                    DispatchMessageW(&message);
                }
                DestroyWindow(window);
            }
            drop(owned);
        });
        let (window, thread_id) = started.recv().unwrap();
        Self {
            window: window.0 as HWND,
            shared,
            thread_id,
            thread: Some(thread),
        }
    }

    /// The requests served so far, in order.
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.shared
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether a `Reply::Late` reply has been written.
    pub(crate) fn late_written(&self) -> bool {
        self.shared.late_written.load(Ordering::SeqCst)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        // SAFETY: posts WM_QUIT to the receiver's own message loop.
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The window procedure: Pageant's `WM_COPYDATA`, else the system's.
unsafe extern "system" fn serve(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the user data is the `Shared` that `Receiver::start`'s thread keeps alive while the window exists.
    unsafe {
        let shared = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Shared;
        if message == WM_COPYDATA && !shared.is_null() {
            return (*shared).answer(lparam as *const COPYDATASTRUCT);
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}

impl Shared {
    /// Serves one `WM_COPYDATA`: 1 as Pageant says it served the request, 0 as it says it did not.
    ///
    /// # Safety
    ///
    /// `data` is the message's `COPYDATASTRUCT`, valid for the call.
    unsafe fn answer(&self, data: *const COPYDATASTRUCT) -> LRESULT {
        // SAFETY: as the caller says; the mapping handle and view are this function's own.
        unsafe {
            let data = &*data;
            if data.dwData != COPYDATA_ID || data.cbData == 0 {
                return 0;
            }
            let name_bytes =
                std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize);
            let Some((0, name)) = name_bytes.split_last().map(|(nul, name)| (*nul, name)) else {
                return 0;
            };
            let name = String::from_utf8_lossy(name).into_owned();
            let wide_name = wide(&name);
            let mapping = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, wide_name.as_ptr());
            if mapping.is_null() {
                return 0;
            }
            let view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, MAPPING_SIZE);
            if view.Value.is_null() {
                CloseHandle(mapping);
                return 0;
            }
            let bytes = std::slice::from_raw_parts_mut(view.Value as *mut u8, MAPPING_SIZE);
            let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            let request = bytes[4..(4 + length).min(MAPPING_SIZE)].to_vec();
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Seen {
                    name,
                    request: request.clone(),
                    descriptor: descriptor_of(mapping),
                });
            let served = match &self.reply {
                Reply::Message(message) => {
                    write_frame(bytes, message);
                    1
                }
                Reply::Raw(raw) => {
                    bytes[..raw.len()].copy_from_slice(raw);
                    1
                }
                Reply::Handler(answer) => {
                    write_frame(bytes, &answer(&request));
                    1
                }
                Reply::Refuse => 0,
                Reply::Late { delay, message } => {
                    thread::sleep(*delay);
                    write_frame(bytes, message);
                    self.late_written.store(true, Ordering::SeqCst);
                    1
                }
            };
            UnmapViewOfFile(view);
            CloseHandle(mapping);
            served
        }
    }
}

fn write_frame(mapping: &mut [u8], message: &[u8]) {
    mapping[..4].copy_from_slice(&(message.len() as u32).to_be_bytes());
    mapping[4..4 + message.len()].copy_from_slice(message);
}

/// The owner and DACL of the mapping `handle` as an SDDL string.
///
/// # Safety
///
/// `handle` is an open file-mapping handle.
unsafe fn descriptor_of(handle: windows_sys::Win32::Foundation::HANDLE) -> String {
    // SAFETY: the descriptor `GetSecurityInfo` allocates is read and freed here.
    unsafe {
        let mut security = ptr::null_mut();
        let status = GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut security,
        );
        if status != 0 {
            return format!("GetSecurityInfo failed: {status}");
        }
        let mut text = ptr::null_mut();
        let converted = ConvertSecurityDescriptorToStringSecurityDescriptorW(
            security,
            SDDL_REVISION_1,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut text,
            ptr::null_mut(),
        );
        let result = if converted == 0 {
            "ConvertSecurityDescriptorToStringSecurityDescriptorW failed".to_owned()
        } else {
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
            LocalFree(text.cast());
            result
        };
        LocalFree(security);
        result
    }
}
