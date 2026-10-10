//! The Windows twin of [`super::agent_fixture`] (GwzTransportWindowsParityPlan.md, step 1.7): a fake SSH agent
//! served on a named pipe by a thread of the test process, with scripted behaviour, and a test-only
//! [`Channel`](crate::git::endpoint::agent_client::Channel) client over a pipe handle.
//!
//! **Policy-free.** It encodes none of TR1.8's rules: no server-identity check on the pipe, no source selection, no
//! UNC rule, no precedence between agents. It implements no agent form of S4.3 and adds no product code. Step 3.3
//! builds the product channel against it, and step 3.4 signs through its test channel.
//!
//! **Signing.** The Unix fixture is a proxy in front of a private `ssh-agent`. Windows has no such agent to start
//! (Windows OpenSSH's runs as a service, which the host rules forbid starting, and Git for Windows' speaks an MSYS
//! socket), so the agent signs itself, with the `ring` signer that `rustls` (already a test dependency) wraps: the
//! fixture's RSA keys are the PEM keys that `ssh-keygen` writes and that libssh2 on Windows can read, and it signs
//! `rsa-sha2-256` and `rsa-sha2-512`. A request for `ssh-rsa` (SHA-1) gets the agent's refusal, as an agent
//! without SHA-1 answers; the Windows libssh2 offers only `rsa-sha2-512` anyway (the baseline's SSH rows, run `2026-10-10-tr18-windows-ssh-rows`).
//!
//! **Scripted behaviour.** [`Fault`]s are one-shot, consumed by the next sign request: the reply delivered in
//! pieces (1 byte, then 2, then the rest), a stall (no reply, until the client leaves), a vanishing pipe (the
//! agent closes the connection without a reply) and a damaged signature (the four shapes of the Unix fixture).
//! The agent serves one client after another; it records every request and whether the client left.
use crate::git::endpoint::{agent_client::Channel, agent_job::Control, ssh_fixture};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_PIPE_BUSY, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile},
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, PeekNamedPipe,
    },
};

mod identity;
mod tests;

pub(crate) use identity::Identity;
use identity::{signature_blob, split_signature, string, take_string};

/// A request the agent has served, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Request {
    List,
    Sign { key: usize, flags: u32 },
}

/// The four damaged signatures of the Unix fixture, as they look for an RSA key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Damage {
    /// The right algorithm name and a one-byte signature.
    ShortSignature,
    /// The algorithm name's first byte replaced by `X`.
    WrongAlgorithm,
    /// `ssh-rsa` and a one-byte signature.
    OneByteRsa,
    /// `ssh-rsa` and a 257-byte signature.
    OversizeRsa,
}

/// What the next sign request meets instead of a plain answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Fault {
    /// The reply to the next request comes in pieces of 1 byte, 2 bytes and the rest, a pause between them.
    Chunked,
    /// No reply, until the client leaves.
    Stall,
    /// The agent closes the connection without a reply.
    Vanish,
    Damage(Damage),
}

/// How the agent answers a sign request for one listed key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Behavior {
    Signs,
    /// Every request is refused, as for an absent security key.
    Refuses,
    /// The first signature is a malformed SHA-1 one of this length; later requests sign.
    MalformedFirst(usize),
}

/// The named-pipe name for an agent in `dir`: unique because the directory is.
pub(crate) fn pipe_name(dir: &Path) -> String {
    format!(
        r"\\.\pipe\gwz-agent-fixture-{}",
        dir.file_name().unwrap().to_string_lossy()
    )
}

/// How far a start has got, for a test that fails it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    PipeCreated,
    Serving,
}

struct Shared {
    entries: Vec<(Identity, Behavior)>,
    requests: Mutex<Vec<Request>>,
    fault: Mutex<Option<Fault>>,
    left: AtomicUsize,
    stop: AtomicBool,
    malformed_sent: AtomicBool,
}

/// An agent serving on a named pipe. Dropping it stops the thread and frees the pipe's name.
pub(crate) struct PipeAgent {
    name: String,
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
}

/// An open pipe handle, closed on drop.
struct Pipe(HANDLE);

// SAFETY: a pipe handle names a kernel object and can be used from any thread.
unsafe impl Send for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl PipeAgent {
    pub(crate) fn start(name: &str, entries: Vec<(Identity, Behavior)>) -> io::Result<Self> {
        Self::start_observing(name, entries, |_| {})
    }

    pub(crate) fn start_observing(
        name: &str,
        entries: Vec<(Identity, Behavior)>,
        mut observe: impl FnMut(Stage),
    ) -> io::Result<Self> {
        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain([0])
            .collect();
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
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let pipe = Pipe(handle);
        let shared = Arc::new(Shared {
            entries,
            requests: Mutex::new(Vec::new()),
            fault: Mutex::new(None),
            left: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            malformed_sent: AtomicBool::new(false),
        });
        // The agent owns the pipe from here: a failure below drops it, and the name is free again.
        let mut agent = Self {
            name: name.to_owned(),
            shared: shared.clone(),
            worker: None,
        };
        observe(Stage::PipeCreated);
        agent.worker = Some(thread::spawn(move || serve(pipe, &shared)));
        observe(Stage::Serving);
        Ok(agent)
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.shared.requests.lock().unwrap().clone()
    }

    /// Arranges for the next sign request (or, for [`Fault::Chunked`], the next request) to meet `fault`.
    pub(crate) fn script(&self, fault: Fault) {
        *self.shared.fault.lock().unwrap() = Some(fault);
    }

    /// Waits for a client to leave, since the last time one was waited for.
    pub(crate) fn wait_client_closed(&self, within: Duration) -> bool {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if self.shared.left.load(Ordering::SeqCst) > 0 {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }

    pub(crate) fn connect(&self) -> PipeChannel {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            match PipeChannel::open(&self.name) {
                Ok(channel) => return channel,
                // The one instance is serving another client, or is between two.
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                        && Instant::now() < until =>
                {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("connecting to the fixture agent failed: {error}"),
            }
        }
    }
}

impl Drop for PipeAgent {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // A worker waiting for a client wakes for this one, and sees the flag.
        let _ = PipeChannel::open(&self.name);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn serve(pipe: Pipe, shared: &Shared) {
    while !shared.stop.load(Ordering::SeqCst) {
        // Waits for a client; ERROR_PIPE_CONNECTED means one connected before the call, which is a connection.
        let connected = unsafe { ConnectNamedPipe(pipe.0, ptr::null_mut()) };
        if connected == 0
            && unsafe { GetLastError() } != windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED
        {
            break;
        }
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        converse(&pipe, shared);
        shared.left.fetch_add(1, Ordering::SeqCst);
        unsafe { DisconnectNamedPipe(pipe.0) };
    }
}

/// One client's requests, until it leaves, the agent vanishes on it, or the fixture stops.
fn converse(pipe: &Pipe, shared: &Shared) {
    while let Some(body) = read_frame(pipe, shared) {
        let Some(fault) = answer(&body, shared) else {
            return;
        };
        match fault {
            Reply::None => {}
            Reply::Whole(reply) => write_all(pipe, &frame(&reply)),
            Reply::Chunked(reply) => {
                let bytes = frame(&reply);
                let mut rest = &bytes[..];
                for size in [1, 2] {
                    let (head, tail) = rest.split_at(size.min(rest.len()));
                    write_all(pipe, head);
                    rest = tail;
                    thread::sleep(Duration::from_millis(40));
                }
                write_all(pipe, rest);
            }
            Reply::Stall => {
                // Never answers: waits for the client to go.
                while !shared.stop.load(Ordering::SeqCst) && available(pipe).is_ok() {
                    thread::sleep(Duration::from_millis(2));
                }
                return;
            }
            Reply::Vanish => return,
        }
    }
}

enum Reply {
    None,
    Whole(Vec<u8>),
    Chunked(Vec<u8>),
    Stall,
    Vanish,
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    string(&mut out, body);
    out
}

/// The agent's answer to one request, or `None` when the request is not one the agent understands.
fn answer(body: &[u8], shared: &Shared) -> Option<Reply> {
    // A listing meets only the fault that is about replies in general; the others wait for a sign request.
    let fault = {
        let mut scripted = shared.fault.lock().unwrap();
        match body.first() {
            Some(11) => scripted.take_if(|fault| *fault == Fault::Chunked),
            _ => scripted.take(),
        }
    };
    let wrap = |reply: Vec<u8>| match fault {
        Some(Fault::Chunked) => Reply::Chunked(reply),
        _ => Reply::Whole(reply),
    };
    match *body.first()? {
        11 => {
            shared.requests.lock().unwrap().push(Request::List);
            let mut reply = vec![12];
            reply.extend_from_slice(&(shared.entries.len() as u32).to_be_bytes());
            for (identity, _) in &shared.entries {
                string(&mut reply, identity.blob());
                string(&mut reply, b"");
            }
            Some(wrap(reply))
        }
        13 => {
            let (key, rest) = take_string(&body[1..])?;
            let (data, rest) = take_string(rest)?;
            let flags = u32::from_be_bytes(rest.try_into().ok()?);
            let index = shared
                .entries
                .iter()
                .position(|(identity, _)| identity.blob() == key)?;
            shared
                .requests
                .lock()
                .unwrap()
                .push(Request::Sign { key: index, flags });
            match fault {
                Some(Fault::Stall) => return Some(Reply::Stall),
                Some(Fault::Vanish) => return Some(Reply::Vanish),
                _ => {}
            }
            let (identity, behavior) = &shared.entries[index];
            let signed = match behavior {
                Behavior::Refuses => None,
                Behavior::MalformedFirst(length)
                    if !shared.malformed_sent.swap(true, Ordering::SeqCst) =>
                {
                    Some(signature_blob(b"ssh-rsa", &vec![0xff; *length]))
                }
                _ => identity.sign(flags, data),
            };
            let Some(mut signature) = signed else {
                return Some(wrap(vec![5]));
            };
            if let Some(Fault::Damage(damage)) = fault {
                signature = damaged(damage, &signature);
            }
            let mut reply = vec![14];
            string(&mut reply, &signature);
            Some(wrap(reply))
        }
        _ => Some(wrap(vec![5])),
    }
}

fn damaged(damage: Damage, signature: &[u8]) -> Vec<u8> {
    let (name, _) = split_signature(signature).unwrap();
    match damage {
        Damage::ShortSignature => signature_blob(name, &[1]),
        Damage::WrongAlgorithm => {
            let mut name = name.to_vec();
            name[0] = b'X';
            let (_, raw) = split_signature(signature).unwrap();
            signature_blob(&name, raw)
        }
        Damage::OneByteRsa => signature_blob(b"ssh-rsa", &[0xff]),
        Damage::OversizeRsa => signature_blob(b"ssh-rsa", &[0xff; 257]),
    }
}

/// How many bytes can be read from `pipe` now; an error once the other end has gone.
fn available(pipe: &Pipe) -> io::Result<u32> {
    let mut count = 0_u32;
    let peeked = unsafe {
        PeekNamedPipe(
            pipe.0,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut count,
            ptr::null_mut(),
        )
    };
    if peeked == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(count)
}

/// One request frame's body, or `None` when the client left, the stream is damaged or the fixture stopped.
fn read_frame(pipe: &Pipe, shared: &Shared) -> Option<Vec<u8>> {
    let header = read_exact(pipe, 4, shared)?;
    let length = u32::from_be_bytes(header.try_into().ok()?) as usize;
    if !(1..=1 << 20).contains(&length) {
        return None;
    }
    read_exact(pipe, length, shared)
}

fn read_exact(pipe: &Pipe, count: usize, shared: &Shared) -> Option<Vec<u8>> {
    let mut bytes = vec![0_u8; count];
    let mut filled = 0;
    while filled < count {
        if shared.stop.load(Ordering::SeqCst) {
            return None;
        }
        let ready = available(pipe).ok()? as usize;
        if ready == 0 {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        let want = ready.min(count - filled);
        let mut read = 0_u32;
        let ok = unsafe {
            ReadFile(
                pipe.0,
                bytes[filled..].as_mut_ptr().cast(),
                want as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            return None;
        }
        filled += read as usize;
    }
    Some(bytes)
}

fn write_all(pipe: &Pipe, bytes: &[u8]) {
    let mut sent = 0;
    while sent < bytes.len() {
        let mut written = 0_u32;
        let ok = unsafe {
            WriteFile(
                pipe.0,
                bytes[sent..].as_ptr().cast(),
                (bytes.len() - sent) as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return;
        }
        sent += written as usize;
    }
}

/// A test-only client of an agent pipe, for the agent client (`agent_client::Agent`): reads that never block (the
/// agent client waits through [`Channel::wait`]), and a wait of at most one supervised slice.
pub(crate) struct PipeChannel {
    file: File,
}

impl PipeChannel {
    pub(crate) fn open(name: &str) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(name)?;
        Ok(Self { file })
    }

    fn ready(&self) -> io::Result<u32> {
        let mut count = 0_u32;
        let handle = self.file.as_raw_handle() as HANDLE;
        let peeked = unsafe {
            PeekNamedPipe(
                handle,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut count,
                ptr::null_mut(),
            )
        };
        if peeked != 0 {
            return Ok(count);
        }
        Err(io::Error::last_os_error())
    }
}

impl Read for PipeChannel {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.ready() {
            // The agent closed its end, or disconnected the client: the end of the stream.
            Err(error) if is_end_of_stream(&error) => Ok(0),
            Err(error) => Err(error),
            Ok(0) => Err(io::ErrorKind::WouldBlock.into()),
            Ok(count) => {
                let want = (count as usize).min(buffer.len());
                self.file.read(&mut buffer[..want])
            }
        }
    }
}

/// Whether an error from the pipe means the agent has gone: it closed the pipe (`ERROR_BROKEN_PIPE`), or
/// disconnected this client (`ERROR_PIPE_NOT_CONNECTED`, `ERROR_BAD_PIPE`).
fn is_end_of_stream(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(109 | 230 | 233))
}

impl Write for PipeChannel {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Channel for PipeChannel {
    fn wait(&mut self, writing: bool, control: &Control) -> io::Result<()> {
        control.wait_step(|slice| {
            if writing {
                return Ok(true);
            }
            let until = Instant::now() + slice.min(Duration::from_millis(20));
            loop {
                match self.ready() {
                    Ok(0) => {}
                    // Bytes, or the end of the stream: either is something to read.
                    _ => return Ok(true),
                }
                if Instant::now() >= until {
                    return Ok(false);
                }
                thread::sleep(Duration::from_millis(1));
            }
        })
    }
}

/// The agent as the endpoint's agent tests use it: two RSA keys, the first one the server would reject and the
/// second the one it trusts (the order of the Unix fixture), served on a pipe in the fixture's own directory.
pub(crate) struct Fixture {
    pub(crate) agent: PipeAgent,
    _dir: tempfile::TempDir,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let entries = ["rejected_key", "auth_key"]
            .into_iter()
            .map(|name| {
                let path = dir.path().join(name);
                ssh_fixture::run_fixture_key(&path);
                (Identity::from_key_file(&path), Behavior::Signs)
            })
            .collect();
        let agent = PipeAgent::start(&pipe_name(dir.path()), entries).unwrap();
        Self { agent, _dir: dir }
    }

    pub(crate) fn blobs(&self) -> Vec<Vec<u8>> {
        self.agent
            .shared
            .entries
            .iter()
            .map(|(identity, _)| identity.blob().to_vec())
            .collect()
    }

    pub(crate) fn identity(&self, index: usize) -> &Identity {
        &self.agent.shared.entries[index].0
    }

    pub(crate) fn connect(&self) -> PipeChannel {
        self.agent.connect()
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.agent.requests()
    }

    pub(crate) fn script(&self, fault: Fault) {
        self.agent.script(fault);
    }

    pub(crate) fn wait_client_closed(&self, within: Duration) -> bool {
        self.agent.wait_client_closed(within)
    }
}
