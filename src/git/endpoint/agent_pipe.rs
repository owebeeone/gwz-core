//! The OpenSSH agent's named pipe as an agent channel (GwzTransportWindowsParityPlan.md, step 3.3; TR1.8 §4; S4.3).
//! Windows only: the agent is a service that listens on `\\.\pipe\openssh-ssh-agent`, or on the local pipe
//! `SSH_AUTH_SOCK` names, and the Unix twin is `agent_socket`.
//!
//! What the design fixes, and where it is here:
//! - The connect asks for `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, libssh2's own (`agent_win.c`), which limits
//!   what the server can do with the caller's token. It checks nothing about who the server is (TD12).
//! - `ERROR_PIPE_BUSY` is waited out inside `Control`, a slice at a time, not by libssh2's fixed second.
//! - The I/O is overlapped, with one owner per handle: at most one read and one write are outstanding, and a retry
//!   after `WouldBlock` is the same call. The buffer, the `OVERLAPPED` and the event of a request stay where they are
//!   until the kernel has completed it. `CancelIoEx` is asked for, and its answer is never taken as evidence: an
//!   `ERROR_NOT_FOUND` says nothing about whether the request has finished, so completion is always observed on the
//!   request's own event. A request whose completion cannot be observed within [`RETIRE_MS`] keeps its storage and
//!   its handle for good (a leak, which is memory-safe), never freed under a kernel that may still write to it.
//! - A pipe that has gone, or a reply that makes no sense, ends the channel: [`super::agent_client::Agent`] poisons
//!   itself on the first error.
//!
//! The pipe's name was admitted by the grammar of [`PipeName`] before anything gets here, so nothing is checked twice.
use super::{
    agent_client::Channel,
    agent_job::Control,
    agent_source::{AgentFault, PipeName},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    mem::ManuallyDrop,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    ptr,
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_BAD_PIPE, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND,
        ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PATH_NOT_FOUND,
        ERROR_PIPE_BUSY, ERROR_PIPE_NOT_CONNECTED, GetLastError, HANDLE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Storage::FileSystem::{FILE_FLAG_OVERLAPPED, ReadFile, SECURITY_IDENTIFICATION, WriteFile},
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::WaitNamedPipeW,
        Threading::{CreateEventW, WaitForSingleObject},
    },
};

/// How much one read asks for. A reply longer than this takes several reads; the rest of a read stays in the channel.
const READ_SIZE: usize = 16 * 1024;
/// The most one write hands the kernel.
const WRITE_SIZE: usize = 64 * 1024;
/// How long a cancelled request is waited for before its storage is kept for good.
const RETIRE_MS: u32 = 5_000;

/// What retiring the outstanding requests of a channel observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Retired {
    /// Nothing was outstanding.
    Idle,
    /// Every outstanding request was cancelled or had finished, and the kernel's completion was observed.
    Observed,
    /// A completion was not observed in time. The storage of that request is kept.
    Unconfirmed,
}

/// A manual-reset event, closed on drop.
struct Event(HANDLE);

impl Event {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// One overlapped request, with everything the kernel may touch until it completes. The pieces are boxed so that
/// they stay where they are when the channel moves.
struct Request {
    overlapped: Box<OVERLAPPED>,
    event: Event,
    buffer: Box<[u8]>,
}

impl Request {
    fn new(buffer: Box<[u8]>) -> io::Result<Self> {
        let event = Event::new()?;
        let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
        overlapped.hEvent = event.0;
        Ok(Self {
            overlapped,
            event,
            buffer,
        })
    }
}

enum Issued {
    /// The request finished at once, having moved this many bytes.
    Done(usize),
    /// The request is outstanding.
    Pending,
    /// The other end has gone.
    Ended,
}

/// An open agent pipe.
pub(crate) struct AgentPipe {
    file: ManuallyDrop<File>,
    read: Option<Request>,
    write: Option<Request>,
    /// Bytes read and not yet handed on, from `taken`.
    ready: Vec<u8>,
    taken: usize,
}

/// Connects to `name` under `control`: the pipe is opened for identification only, a busy pipe is waited for in
/// slices of the control's quantum, and a pipe that is not there is [`AgentFault::NotRunning`].
pub(crate) fn connect(name: &PipeName, control: &Arc<Control>) -> io::Result<AgentPipe> {
    control.check()?;
    control.begin_wait()?;
    let wide: Vec<u16> = std::ffi::OsStr::new(name.as_str())
        .encode_wide()
        .chain([0])
        .collect();
    loop {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name.as_str());
        let error = match opened {
            Ok(file) => {
                control.complete_wait()?;
                return Ok(AgentPipe {
                    file: ManuallyDrop::new(file),
                    read: None,
                    write: None,
                    ready: Vec::new(),
                    taken: 0,
                });
            }
            Err(error) => error,
        };
        match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_PIPE_BUSY) => {
                control.check()?;
                let slice = control.quantum()?;
                let millis = slice.as_millis().clamp(1, 20) as u32;
                // Returns when an instance is free, or when the slice is up; either way the open is tried again.
                unsafe { WaitNamedPipeW(wide.as_ptr(), millis) };
                control.check()?;
            }
            Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => {
                return Err(AgentFault::NotRunning.into_error());
            }
            Some(ERROR_ACCESS_DENIED) => return Err(AgentFault::Denied.into_error()),
            _ => return Err(error.kind().into()),
        }
    }
}

impl AgentPipe {
    fn handle(&self) -> HANDLE {
        self.file.as_raw_handle() as HANDLE
    }

    /// Starts `request` as a read or a write of `len` bytes.
    fn issue(&self, request: &mut Request, writing: bool, len: usize) -> io::Result<Issued> {
        let handle = self.handle();
        let ok = unsafe {
            if writing {
                WriteFile(
                    handle,
                    request.buffer.as_ptr(),
                    len as u32,
                    ptr::null_mut(),
                    &mut *request.overlapped,
                )
            } else {
                ReadFile(
                    handle,
                    request.buffer.as_mut_ptr(),
                    len as u32,
                    ptr::null_mut(),
                    &mut *request.overlapped,
                )
            }
        };
        if ok != 0 {
            return Ok(match self.outcome(request)? {
                Some(0) => Issued::Ended,
                Some(moved) => Issued::Done(moved),
                None => Issued::Pending,
            });
        }
        match unsafe { GetLastError() } {
            ERROR_IO_PENDING => Ok(Issued::Pending),
            code if gone(code) => Ok(Issued::Ended),
            code => Err(os_error(code, writing)),
        }
    }

    /// How many bytes `request` moved, if it has completed; `None` while it is outstanding.
    fn outcome(&self, request: &Request) -> io::Result<Option<usize>> {
        let mut moved = 0_u32;
        let ok = unsafe { GetOverlappedResult(self.handle(), &*request.overlapped, &mut moved, 0) };
        if ok != 0 {
            return Ok(Some(moved as usize));
        }
        match unsafe { GetLastError() } {
            ERROR_IO_INCOMPLETE => Ok(None),
            code if gone(code) => Ok(Some(0)),
            code => Err(os_error(code, false)),
        }
    }

    /// Takes `moved` bytes of a finished read into the channel and serves `out` from them.
    fn take_read(&mut self, request: Request, moved: usize, out: &mut [u8]) -> usize {
        self.ready.clear();
        self.ready.extend_from_slice(&request.buffer[..moved]);
        self.taken = 0;
        self.serve(out)
    }

    fn serve(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.ready.len() - self.taken);
        out[..count].copy_from_slice(&self.ready[self.taken..self.taken + count]);
        self.taken += count;
        count
    }

    /// Cancels the outstanding requests and observes their completion, so that their storage can be released. A
    /// request that completes on its own while the cancel is made counts as observed too.
    pub(crate) fn cancel_pending(&mut self) -> Retired {
        let mut worst = Retired::Idle;
        for slot in 0..2 {
            let request = if slot == 0 {
                self.read.take()
            } else {
                self.write.take()
            };
            let Some(request) = request else {
                continue;
            };
            // Whether the cancel found anything is not evidence of anything: the answer is not read.
            unsafe { CancelIoEx(self.handle(), &*request.overlapped) };
            let waited = unsafe { WaitForSingleObject(request.event.0, RETIRE_MS) };
            let observed =
                waited == WAIT_OBJECT_0 && matches!(self.outcome(&request), Ok(Some(_)) | Err(_));
            if observed {
                worst = worst.max(Retired::Observed);
            } else {
                // The kernel may still write to the buffer or the OVERLAPPED: they are never freed.
                std::mem::forget(request);
                worst = Retired::Unconfirmed;
            }
        }
        worst
    }
}

impl Drop for AgentPipe {
    fn drop(&mut self) {
        if self.cancel_pending() == Retired::Unconfirmed {
            // The handle stays open with the storage it may still be writing to.
            return;
        }
        // SAFETY: dropped once, here, with no request outstanding.
        unsafe { ManuallyDrop::drop(&mut self.file) };
    }
}

impl Read for AgentPipe {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.taken < self.ready.len() {
            return Ok(self.serve(out));
        }
        let request = match self.read.take() {
            Some(request) => request,
            None => {
                let mut request = Request::new(vec![0; READ_SIZE].into_boxed_slice())?;
                match self.issue(&mut request, false, READ_SIZE)? {
                    Issued::Done(moved) => return Ok(self.take_read(request, moved, out)),
                    Issued::Ended => return Ok(0),
                    Issued::Pending => {
                        self.read = Some(request);
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                }
            }
        };
        match self.outcome(&request)? {
            None => {
                self.read = Some(request);
                Err(io::ErrorKind::WouldBlock.into())
            }
            Some(0) => Ok(0),
            Some(moved) => Ok(self.take_read(request, moved, out)),
        }
    }
}

impl Write for AgentPipe {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        // A write that was outstanding is the same bytes again: the caller retries with what it was refused.
        let request = match self.write.take() {
            Some(request) => request,
            None => {
                let take = data.len().min(WRITE_SIZE);
                let mut request = Request::new(data[..take].to_vec().into_boxed_slice())?;
                match self.issue(&mut request, true, take)? {
                    Issued::Done(moved) => return Ok(moved),
                    Issued::Ended => return Err(io::ErrorKind::BrokenPipe.into()),
                    Issued::Pending => {
                        self.write = Some(request);
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                }
            }
        };
        match self.outcome(&request)? {
            None => {
                self.write = Some(request);
                Err(io::ErrorKind::WouldBlock.into())
            }
            Some(0) => Err(io::ErrorKind::BrokenPipe.into()),
            Some(moved) => Ok(moved.min(data.len())),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Channel for AgentPipe {
    fn wait(&mut self, writing: bool, control: &Control) -> io::Result<()> {
        let outstanding = if writing { &self.write } else { &self.read };
        let event = outstanding.as_ref().map(|request| request.event.0);
        control.wait_step(|slice| {
            // With nothing outstanding there is nothing to wait for: the caller tries again at once.
            let Some(event) = event else {
                return Ok(true);
            };
            let millis = slice.as_millis().clamp(1, 20) as u32;
            match unsafe { WaitForSingleObject(event, millis) } {
                WAIT_OBJECT_0 => Ok(true),
                WAIT_TIMEOUT => Ok(false),
                _ => Err(io::Error::last_os_error()),
            }
        })
    }
}

/// Whether `code` says the other end has gone: it closed the pipe, or disconnected this client.
fn gone(code: u32) -> bool {
    matches!(
        code,
        ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_BAD_PIPE | ERROR_NO_DATA
    )
}

/// The error for a Windows error code, as its kind only: the system's text can name the pipe.
fn os_error(code: u32, writing: bool) -> io::Error {
    let kind = io::Error::from_raw_os_error(code as i32).kind();
    if writing && kind == io::ErrorKind::Other {
        return io::ErrorKind::BrokenPipe.into();
    }
    kind.into()
}
