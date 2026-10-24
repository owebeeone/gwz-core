//! The Windows named pipe carrier. See the module documentation of [`super`] for what the pipe is created with and what
//! the peer check reads.
//!
//! The pipe is overlapped, with one outstanding read and one outstanding write at most, because a synchronous pipe
//! handle serializes its callers: a thread blocked in `ReadFile` would keep `WriteFile` from running on the same
//! handle. Each call waits for its own completion, so the `OVERLAPPED` it uses can live on its stack. A caller that
//! must stop a waiting call cancels with `CancelIoEx`, and the call returns once the kernel has completed the request.
use super::super::{CarrierBinding, CarrierId, CarrierLimits, Peer, Side};
use super::{
    Accepted, Integrity, LocalRefusal, PeerIdentity, PeerPolicy, PeerRefusal, Principal, Relation,
    StreamCarrier,
};
use std::{
    io::{self, Read, Write},
    path::Path,
    ptr, slice,
    sync::{Arc, Mutex, PoisonError},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA,
        ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
        GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetSidSubAuthority,
        GetSidSubAuthorityCount, GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TOKEN_USER, TokenIntegrityLevel, TokenIsAppContainer, TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
        },
        Threading::{
            CreateEventW, GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, ResetEvent,
        },
    },
};

/// How a pipe instance is opened by the server: duplex, overlapped, and the first of its name.
pub(crate) const PIPE_OPEN_MODE: u32 =
    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE;
/// How it behaves: a byte pipe that waits, and that refuses a client from another machine.
pub(crate) const PIPE_MODE: u32 =
    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
/// The pipe's buffers, each way.
const BUFFER: u32 = 64 * 1024;
/// How many times `bind` draws a new name when the one drawn is taken.
const NAME_DRAWS: usize = 3;
/// How long a client waits for a busy pipe, in milliseconds, per attempt, and how many attempts it makes.
const BUSY_WAIT_MS: u32 = 2_000;
const BUSY_ATTEMPTS: usize = 3;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn last() -> io::Error {
    io::Error::last_os_error()
}

fn os(call: &'static str) -> LocalRefusal {
    LocalRefusal::os(call, &last())
}

/// A handle closed on drop.
struct Owned(HANDLE);

// SAFETY: a kernel handle may be used and closed from any thread.
unsafe impl Send for Owned {}
// SAFETY: as above; the calls made through `&Owned` are the kernel's own, which it serializes.
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this value owns and closes once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// The text of a wide string the system allocated, freed here.
///
/// # Safety
/// `text` is a NUL-terminated string allocated by a call whose result is freed with `LocalFree`.
unsafe fn take_local_string(text: *mut u16) -> String {
    // SAFETY: by the contract above.
    unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let string = String::from_utf16_lossy(slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        string
    }
}

/// A SID in its string form.
///
/// # Safety
/// `sid` points to a valid SID.
unsafe fn sid_string(sid: *mut core::ffi::c_void) -> Result<String, LocalRefusal> {
    let mut text = ptr::null_mut();
    // SAFETY: `sid` is valid by the contract; `text` receives a system allocation that `take_local_string` frees.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err(os("ConvertSidToStringSidW"));
        }
        Ok(take_local_string(text))
    }
}

/// The bytes of one token information class, aligned for the structures read out of them.
fn token_class(token: HANDLE, class: i32) -> Result<Vec<u64>, LocalRefusal> {
    let mut size = 0u32;
    // SAFETY: the first call only asks for the size.
    unsafe { GetTokenInformation(token, class, ptr::null_mut(), 0, &mut size) };
    if size == 0 {
        return Err(os("GetTokenInformation(size)"));
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: the buffer holds `size` bytes at 8-byte alignment.
    if unsafe { GetTokenInformation(token, class, buffer.as_mut_ptr().cast(), size, &mut size) }
        == 0
    {
        return Err(os("GetTokenInformation"));
    }
    Ok(buffer)
}

/// What a token says: its user, its integrity level and whether it is an AppContainer token.
fn token_identity(token: HANDLE, pid: u32) -> Result<PeerIdentity, LocalRefusal> {
    let user = token_class(token, TokenUser)?;
    // SAFETY: `user` holds a `TOKEN_USER` whose SID points into the same buffer.
    let principal = unsafe {
        let user = &*(user.as_ptr() as *const TOKEN_USER);
        Principal::Sid(sid_string(user.User.Sid)?)
    };
    let label = token_class(token, TokenIntegrityLevel)?;
    // SAFETY: `label` holds a `TOKEN_MANDATORY_LABEL` whose SID points into the same buffer; the last sub-authority of
    // a mandatory label is the level.
    let rid = unsafe {
        let label = &*(label.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(LocalRefusal::Peer(PeerRefusal::Unreadable(
                "an integrity level",
            )));
        }
        *GetSidSubAuthority(sid, u32::from(count) - 1)
    };
    let boxed = token_class(token, TokenIsAppContainer)?;
    Ok(PeerIdentity {
        principal,
        pid: Some(pid),
        integrity: Some(Integrity::from_rid(rid)),
        app_container: boxed.first().is_some_and(|flag| (*flag & 0xffff_ffff) != 0),
    })
}

/// The identity of the process `pid`, read through a query-only handle on it and its token. A process that cannot be
/// opened (it has gone, or runs at a level this one may not query) is an error: the check cannot be made.
fn process_identity(pid: u32) -> Result<PeerIdentity, LocalRefusal> {
    // SAFETY: opens a query-only handle, closed by its guard.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(os("OpenProcess"));
    }
    let process = Owned(process);
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is a live handle; the token is closed by its guard.
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
        return Err(os("OpenProcessToken"));
    }
    let token = Owned(token);
    token_identity(token.0, pid)
}

pub(crate) fn own_identity() -> Result<PeerIdentity, LocalRefusal> {
    // SAFETY: reads this process's id and opens a query-only token, closed by its guard.
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(os("OpenProcessToken"));
        }
        let token = Owned(token);
        token_identity(token.0, GetCurrentProcessId())
    }
}

/// The descriptor: a protected DACL with one entry, full access for `sid`. Nobody else, administrators and SYSTEM
/// included, is granted anything.
pub(crate) fn pipe_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

/// A pipe instance, closed on drop.
pub(crate) struct PipeInstance(Owned);

/// Creates an instance of the pipe `name` with this user's descriptor. `first` asks for the first instance of the
/// name, which fails if the name is already in use by anything.
fn create_instance(name: &str, first: bool) -> Result<PipeInstance, LocalRefusal> {
    let Principal::Sid(sid) = own_identity()?.principal else {
        return Err(LocalRefusal::Peer(PeerRefusal::Unreadable("a user SID")));
    };
    let sddl = wide(&pipe_sddl(&sid));
    let mut descriptor = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; the descriptor is a system allocation freed below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(os("ConvertStringSecurityDescriptorToSecurityDescriptorW"));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let open_mode = if first {
        PIPE_OPEN_MODE
    } else {
        PIPE_OPEN_MODE & !FILE_FLAG_FIRST_PIPE_INSTANCE
    };
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated and `attributes` and its descriptor outlive the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_MODE,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            &attributes,
        )
    };
    let error = if handle == INVALID_HANDLE_VALUE {
        // SAFETY: reads the calling thread's last error, before another call can change it.
        Some(unsafe { GetLastError() })
    } else {
        None
    };
    // SAFETY: frees the descriptor `ConvertString...` allocated, which the pipe has copied.
    unsafe { LocalFree(descriptor) };
    match error {
        None => Ok(PipeInstance(Owned(handle))),
        // A name that exists makes a first-instance creation fail with access denied (or busy).
        Some(ERROR_ACCESS_DENIED | ERROR_PIPE_BUSY) if first => Err(LocalRefusal::NameTaken),
        Some(code) => Err(LocalRefusal::Os {
            call: "CreateNamedPipeW",
            kind: io::Error::from_raw_os_error(code as i32).kind(),
            code: Some(code as i32),
        }),
    }
}

/// The first instance of a pipe name (a squatter that holds the name makes this fail with `NameTaken`).
pub(crate) fn create_first_instance(name: &str) -> Result<PipeInstance, LocalRefusal> {
    create_instance(name, true)
}

/// The user SIDs the DACL of `pipe` grants access to: one entry for each access-allowed ACE.
pub(crate) fn created_pipe_dacl_sids(pipe: &PipeInstance) -> Vec<String> {
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: a live pipe handle; the descriptor is freed below and the ACL points into it.
    unsafe {
        let status = GetSecurityInfo(
            (pipe.0).0,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        );
        assert_eq!(status, 0, "GetSecurityInfo");
        let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
        assert_ne!(
            GetAclInformation(
                dacl,
                (&raw mut info).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ),
            0,
            "GetAclInformation"
        );
        let mut sids = Vec::new();
        for index in 0..info.AceCount {
            let mut ace = ptr::null_mut();
            assert_ne!(GetAce(dacl, index, &mut ace), 0, "GetAce");
            let ace = &mut *(ace as *mut ACCESS_ALLOWED_ACE);
            assert_eq!(ace.Header.AceType, 0, "an access-allowed ACE");
            sids.push(sid_string((&raw mut ace.SidStart).cast()).expect("a SID string"));
        }
        LocalFree(descriptor);
        sids
    }
}

/// The per-user directory does not exist on Windows: the pipe's name is random and its descriptor is the user's.
#[derive(Debug)]
pub(crate) struct RuntimeDir;

impl RuntimeDir {
    pub(crate) fn for_current_user() -> Result<Self, LocalRefusal> {
        Ok(Self)
    }

    pub(crate) fn at(_: &Path) -> Result<Self, LocalRefusal> {
        Ok(Self)
    }
}

/// A pipe's name, `\\.\pipe\gwz-` and 32 hex digits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalAddress(String);

impl LocalAddress {
    /// An address given explicitly. It is verified when it is connected to, never trusted before.
    pub(crate) fn from_name(name: &str) -> Self {
        Self(name.to_owned())
    }

    pub(crate) fn pipe_name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LocalAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One overlapped pipe handle used by one reader and one writer at a time.
struct Pipe {
    handle: Owned,
    read: Mutex<Event>,
    write: Mutex<Event>,
}

struct Event(Owned);

impl Event {
    fn new() -> io::Result<Self> {
        // SAFETY: a manual-reset, unsignalled, unnamed event, closed by its guard.
        let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if event.is_null() {
            return Err(last());
        }
        Ok(Self(Owned(event)))
    }
}

impl Pipe {
    fn new(handle: Owned) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            handle,
            read: Mutex::new(Event::new()?),
            write: Mutex::new(Event::new()?),
        }))
    }

    /// Runs one overlapped request and waits for the kernel to finish it, however it ends.
    fn run(&self, event: &Event, issue: impl FnOnce(*mut OVERLAPPED) -> i32) -> io::Result<usize> {
        // SAFETY: the event is this call's alone (its mutex is held); the request lives on this stack and is waited for
        // below whatever happens, so the kernel never writes to it after the call returns.
        unsafe {
            ResetEvent((event.0).0);
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            overlapped.hEvent = (event.0).0;
            let started = issue(&mut overlapped);
            if started == 0 {
                match GetLastError() {
                    ERROR_IO_PENDING => {}
                    ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => return Ok(0),
                    code => return Err(io::Error::from_raw_os_error(code as i32)),
                }
            }
            let mut moved = 0u32;
            if GetOverlappedResult(self.handle.0, &overlapped, &mut moved, 1) == 0 {
                return match GetLastError() {
                    ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => Ok(0),
                    ERROR_OPERATION_ABORTED => Err(io::ErrorKind::ConnectionAborted.into()),
                    code => Err(io::Error::from_raw_os_error(code as i32)),
                };
            }
            Ok(moved as usize)
        }
    }

    fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let event = self.read.lock().unwrap_or_else(PoisonError::into_inner);
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        self.run(&event, |overlapped| {
            // SAFETY: the buffer outlives the request, which `run` waits for.
            unsafe {
                ReadFile(
                    self.handle.0,
                    buffer.as_mut_ptr().cast(),
                    length,
                    ptr::null_mut(),
                    overlapped,
                )
            }
        })
    }

    fn write(&self, bytes: &[u8]) -> io::Result<usize> {
        let event = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let moved = self.run(&event, |overlapped| {
            // SAFETY: the bytes outlive the request, which `run` waits for.
            unsafe {
                WriteFile(
                    self.handle.0,
                    bytes.as_ptr().cast(),
                    length,
                    ptr::null_mut(),
                    overlapped,
                )
            }
        })?;
        if moved == 0 && !bytes.is_empty() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(moved)
    }

    /// Cancels the requests waiting on the pipe, so that their callers return.
    fn cancel(&self) {
        // SAFETY: cancels all of this process's outstanding requests on a live handle.
        unsafe { CancelIoEx(self.handle.0, ptr::null()) };
    }
}

struct ReadHalf(Arc<Pipe>);
struct WriteHalf(Arc<Pipe>);

impl Read for ReadHalf {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for WriteHalf {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn carrier_over(pipe: Owned, side: Side, peer: PeerIdentity, limits: CarrierLimits) -> Accepted {
    let spawn = |error: io::Error| LocalRefusal::os("spawn", &error);
    let pipe = Pipe::new(pipe).map_err(spawn)?;
    let id = CarrierId::random().map_err(spawn)?;
    let killer = pipe.clone();
    StreamCarrier::new(
        Box::new(ReadHalf(pipe.clone())),
        Box::new(WriteHalf(pipe)),
        Box::new(move || killer.cancel()),
        CarrierBinding::new(id, side, Peer::Verified(peer)),
        limits,
    )
    .map_err(spawn)
}

/// A listening pipe: the instance waiting for the next client, and the rule applied to whoever connects.
pub(crate) struct LocalListener {
    address: LocalAddress,
    waiting: Mutex<Option<PipeInstance>>,
    policy: PeerPolicy,
    limits: CarrierLimits,
}

impl LocalListener {
    /// Listens on a new pipe with a random name. A name that is taken is not used: another name is drawn, up to three
    /// times.
    pub(crate) fn bind(
        _: &RuntimeDir,
        policy: PeerPolicy,
        limits: CarrierLimits,
    ) -> Result<Self, LocalRefusal> {
        for _ in 0..NAME_DRAWS {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)
                .map_err(|_| LocalRefusal::os("getrandom", &io::Error::other("no random bytes")))?;
            let name = format!(
                r"\\.\pipe\gwz-{}",
                random
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            match create_instance(&name, true) {
                Ok(first) => {
                    return Ok(Self {
                        address: LocalAddress(name),
                        waiting: Mutex::new(Some(first)),
                        policy,
                        limits,
                    });
                }
                Err(LocalRefusal::NameTaken) => {}
                Err(other) => return Err(other),
            }
        }
        Err(LocalRefusal::NameTaken)
    }

    pub(crate) fn address(&self) -> &LocalAddress {
        &self.address
    }

    /// Waits for a client and judges its process and token before anything is read or written. A refused client is
    /// disconnected and reported; the listener goes on with a fresh instance.
    pub(crate) fn accept(&self) -> Accepted {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let instance = match waiting.take() {
            Some(instance) => instance,
            None => create_instance(self.address.pipe_name(), false)?,
        };
        let handle = (instance.0).0;
        // The instance is overlapped, so the connect is too.
        let event = Event::new().map_err(|error| LocalRefusal::os("CreateEventW", &error))?;
        // SAFETY: the request lives on this stack and is waited for before it is left.
        let connected = unsafe {
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            overlapped.hEvent = (event.0).0;
            if ConnectNamedPipe(handle, &mut overlapped) == 0 {
                match GetLastError() {
                    ERROR_PIPE_CONNECTED => true,
                    ERROR_IO_PENDING => {
                        let mut moved = 0u32;
                        GetOverlappedResult(handle, &overlapped, &mut moved, 1) != 0
                    }
                    _ => false,
                }
            } else {
                true
            }
        };
        if !connected {
            return Err(os("ConnectNamedPipe"));
        }
        // The next client may connect to a fresh instance while this one is judged.
        *waiting = Some(create_instance(self.address.pipe_name(), false)?);
        drop(waiting);
        let refuse = |refusal| {
            // SAFETY: a live server instance; the client reads a broken pipe.
            unsafe { DisconnectNamedPipe(handle) };
            LocalRefusal::Peer(refusal)
        };
        let mut pid = 0u32;
        // SAFETY: a live, connected server instance.
        if unsafe { GetNamedPipeClientProcessId(handle, &mut pid) } == 0 {
            return Err(refuse(PeerRefusal::Unreadable("the client's process id")));
        }
        let peer = process_identity(pid)
            .map_err(|_| refuse(PeerRefusal::Unreadable("the client's token")))?;
        self.policy
            .verify(&peer, Relation::ListenerChecksClient)
            .map_err(refuse)?;
        let PipeInstance(owned) = instance;
        carrier_over(owned, Side::Core, peer, self.limits)
    }
}

/// Connects to the pipe `address`, verifying the server before a byte is sent: the process that holds the pipe, its
/// user, its integrity level and that it is not an AppContainer.
pub(crate) fn connect(
    address: &LocalAddress,
    policy: &PeerPolicy,
    limits: CarrierLimits,
) -> Accepted {
    let name = wide(address.pipe_name());
    let mut attempts = 0;
    let handle = loop {
        // SAFETY: `name` is NUL-terminated. The pipe is opened overlapped, and with identification impersonation only:
        // a server can learn who the client is but never act as it.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            break Owned(handle);
        }
        // SAFETY: reads the calling thread's last error.
        let code = unsafe { GetLastError() };
        attempts += 1;
        if code != ERROR_PIPE_BUSY || attempts >= BUSY_ATTEMPTS {
            return Err(LocalRefusal::Os {
                call: "CreateFileW",
                kind: io::Error::from_raw_os_error(code as i32).kind(),
                code: Some(code as i32),
            });
        }
        // SAFETY: waits for an instance of a name this process has just seen busy.
        unsafe { WaitNamedPipeW(name.as_ptr(), BUSY_WAIT_MS) };
    };
    let refuse = |refusal| LocalRefusal::Peer(refusal);
    let mut pid = 0u32;
    // SAFETY: a live client handle.
    if unsafe { GetNamedPipeServerProcessId(handle.0, &mut pid) } == 0 {
        return Err(refuse(PeerRefusal::Unreadable("the server's process id")));
    }
    let peer =
        process_identity(pid).map_err(|_| refuse(PeerRefusal::Unreadable("the server's token")))?;
    policy
        .verify(&peer, Relation::ClientChecksListener)
        .map_err(refuse)?;
    carrier_over(handle, Side::Client, peer, limits)
}
