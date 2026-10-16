//! A raw pipe server (Windows) for the rows that step 1.7's agent cannot script (plan step 3.3): it reads one five-byte request (a listing),
//! looks at the impersonation level the client connected with, writes bytes of the test's choosing, and then waits for
//! the client to leave. Test code only; it encodes no policy.
use std::{ptr, sync::mpsc, thread};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    Security::{GetTokenInformation, RevertToSelf, TOKEN_QUERY, TokenImpersonationLevel},
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile},
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, ImpersonateNamedPipeClient,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::{GetCurrentThread, OpenThreadToken},
    },
};

/// What the server saw of its one client.
pub(crate) struct Seen {
    /// The request bytes it read.
    pub(crate) request: Vec<u8>,
    /// Whether it saw the client's end of the pipe close.
    pub(crate) client_left: bool,
    /// The impersonation level of the client's token as the server saw it (1 is `SecurityIdentification`).
    pub(crate) level: Option<u32>,
}

pub(crate) struct RawServer {
    replied: mpsc::Receiver<()>,
    thread: thread::JoinHandle<Seen>,
}

struct Handle(HANDLE);

// SAFETY: a pipe handle names a kernel object and can be used from any thread; the thread that gets it is its only user.
unsafe impl Send for Handle {}

impl RawServer {
    /// Creates the pipe `name` (so that it exists when this returns) and serves one client: it replies with `reply`
    /// after the request, and the join returns once the client has left.
    pub(crate) fn start(name: &str, reply: Vec<u8>) -> Self {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65_536,
                65_536,
                0,
                ptr::null(),
            )
        };
        assert_ne!(handle, INVALID_HANDLE_VALUE, "creating the raw pipe failed");
        let (replied_tx, replied) = mpsc::channel();
        let pipe = Handle(handle);
        let thread = thread::spawn(move || {
            let pipe = pipe;
            serve(pipe.0, &reply, &replied_tx)
        });
        Self { replied, thread }
    }

    /// Waits until the reply has been written.
    pub(crate) fn wait_replied(&self) {
        self.replied
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the raw server never replied");
    }

    pub(crate) fn join(self) -> Seen {
        self.thread.join().unwrap()
    }
}

fn serve(pipe: HANDLE, reply: &[u8], replied: &mpsc::Sender<()>) -> Seen {
    let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
    assert!(
        connected != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED,
        "no client connected"
    );
    let mut request = vec![0_u8; 5];
    let mut filled = 0;
    while filled < request.len() {
        let mut read = 0_u32;
        let ok = unsafe {
            ReadFile(
                pipe,
                request[filled..].as_mut_ptr(),
                (request.len() - filled) as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            request.truncate(filled);
            break;
        }
        filled += read as usize;
    }
    // The client must have sent data before the server can impersonate it.
    let level = impersonation_level(pipe);
    let mut sent = 0;
    while sent < reply.len() {
        let mut written = 0_u32;
        let ok = unsafe {
            WriteFile(
                pipe,
                reply[sent..].as_ptr(),
                (reply.len() - sent) as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            break;
        }
        sent += written as usize;
    }
    let _ = replied.send(());
    // Waits for the client to leave: a read that fails means its end closed.
    let client_left = loop {
        let mut byte = 0_u8;
        let mut read = 0_u32;
        let ok = unsafe { ReadFile(pipe, &mut byte, 1, &mut read, ptr::null_mut()) };
        if ok == 0 {
            break true;
        }
    };
    unsafe {
        DisconnectNamedPipe(pipe);
        CloseHandle(pipe);
    }
    Seen {
        request,
        client_left,
        level,
    }
}

fn impersonation_level(pipe: HANDLE) -> Option<u32> {
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return None;
    }
    let mut level = None;
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } != 0 {
        let mut value = 0_i32;
        let mut returned = 0_u32;
        let queried = unsafe {
            GetTokenInformation(
                token,
                TokenImpersonationLevel,
                (&mut value as *mut i32).cast(),
                4,
                &mut returned,
            )
        };
        if queried != 0 {
            level = Some(value as u32);
        }
        unsafe { CloseHandle(token) };
    }
    unsafe { RevertToSelf() };
    level
}
