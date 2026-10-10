//! One `CreateProcessW`, made the way gwz-sspi's worker launch makes its own (`supervisor/windows/launch.rs`):
//! the exact executable, a job list and a handle list in a `STARTUPINFOEXW`, so that the helper is a member of its
//! job from its first instruction and inherits its three pipe ends and nothing else. The pattern is copied, not
//! the crate: this stays the HTTPS helpers' private owner, not a second SSPI owner.
use super::{HelperChild, attributes::Attributes, pipe};
use crate::git::endpoint::https_auth::process_tree::HelperCommand;
use std::{
    ffi::OsStr,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle},
    },
};
use tokio::net::windows::named_pipe::NamedPipeServer;
use windows_sys::Win32::System::{
    JobObjects::IsProcessInJob,
    Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, EXTENDED_STARTUPINFO_PRESENT,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    },
};

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// `text` as UTF-16 without a terminator, refusing the NUL that would end a C string early.
fn wide(text: &OsStr) -> io::Result<Vec<u16>> {
    let units: Vec<u16> = text.encode_wide().collect();
    if units.contains(&0) {
        return Err(invalid("a NUL in a process argument, name or path"));
    }
    Ok(units)
}

/// `"program" arg ...`, quoted the way `CommandLineToArgvW` reads it back.
pub(super) fn command_line(program: &[u16], args: &[Vec<u16>]) -> io::Result<Vec<u16>> {
    if program.contains(&QUOTE) {
        return Err(invalid("a quote in the executable path"));
    }
    let mut line = vec![QUOTE];
    line.extend_from_slice(program);
    line.push(QUOTE);
    for arg in args {
        line.push(b' ' as u16);
        let quoted = arg.is_empty()
            || arg
                .iter()
                .any(|unit| *unit == b' ' as u16 || *unit == b'\t' as u16);
        if quoted {
            line.push(QUOTE);
        }
        let mut backslashes = 0;
        for unit in arg {
            if *unit == BACKSLASH {
                backslashes += 1;
            } else {
                if *unit == QUOTE {
                    line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(*unit);
        }
        if quoted {
            line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
            line.push(QUOTE);
        }
    }
    line.push(0);
    Ok(line)
}

/// The environment block: `NAME=value` strings, sorted by name without regard to case, ending in a double NUL.
pub(super) fn environment_block(
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> io::Result<Vec<u16>> {
    let mut entries = Vec::with_capacity(environment.len());
    for (name, value) in environment {
        let name = wide(name)?;
        // A name may start with '=' (the hidden per-drive variables, "=C:"), as the snapshot accepts; an '=' after
        // the first unit would end the name early.
        if name.is_empty() || name[1..].contains(&(b'=' as u16)) {
            return Err(invalid(
                "an environment name that is empty or holds '=' after its first unit",
            ));
        }
        entries.push((name, wide(value)?));
    }
    let fold = |unit: u16| match u8::try_from(unit) {
        Ok(byte) => u16::from(byte.to_ascii_uppercase()),
        Err(_) => unit,
    };
    entries.sort_by(|left, right| {
        left.0
            .iter()
            .map(|u| fold(*u))
            .cmp(right.0.iter().map(|u| fold(*u)))
    });
    let mut block = Vec::new();
    for (name, value) in entries {
        block.extend(name);
        block.push(b'=' as u16);
        block.extend(value);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

/// The creation window: the child's pipe ends are inheritable from [`InheritWindow::open`] to its drop, and are
/// not inheritable at any other time. It spans exactly the `CreateProcessW` call, including process creation
/// itself (milliseconds), and is closed on every path, a failed call included.
pub(super) struct InheritWindow<'a> {
    ends: [&'a OwnedHandle; 3],
}

impl<'a> InheritWindow<'a> {
    pub(super) fn open(ends: [&'a OwnedHandle; 3]) -> io::Result<Self> {
        // The guard exists before the first end is opened, so that a failure part way closes the ones opened.
        let window = Self { ends };
        for end in window.ends {
            pipe::set_inheritable(end, true)?;
        }
        Ok(window)
    }
}

impl Drop for InheritWindow<'_> {
    fn drop(&mut self) {
        for end in self.ends {
            let _ = pipe::set_inheritable(end, false);
        }
    }
}

/// Creates the helper in `job`. On any failure no process is left running.
pub(super) fn create(command: &HelperCommand, job: &OwnedHandle) -> io::Result<HelperChild> {
    if !command.program.is_absolute() {
        return Err(invalid("a helper executable that is not an absolute path"));
    }
    let program = wide(command.program.as_os_str())?;
    let args = command
        .args
        .iter()
        .map(|arg| wide(arg))
        .collect::<io::Result<Vec<_>>>()?;
    let mut line = command_line(&program, &args)?;
    let mut block = environment_block(&command.environment)?;
    let mut directory = wide(command.directory.as_os_str())?;
    directory.push(0);
    let mut application = program;
    application.push(0);

    let stdin = pipe::create(pipe::Parent::Writes)?;
    let stdout = pipe::create(pipe::Parent::Reads)?;
    let stderr = pipe::create(pipe::Parent::Reads)?;
    // The parent ends are registered with the runtime's completion port before the process exists, so that a
    // refusal (no runtime) starts nothing. They are tokio's named pipes: dropping one cancels its pending read, so
    // nothing of a lookup stays parked in a thread once its futures are gone, whatever a survivor still holds.
    let (stdin_parent, stdout_parent, stderr_parent) = (
        server(stdin.parent)?,
        server(stdout.parent)?,
        server(stderr.parent)?,
    );
    let mut attributes = Attributes::new(
        job,
        [
            stdin.child.as_raw_handle(),
            stdout.child.as_raw_handle(),
            stderr.child.as_raw_handle(),
        ],
    )?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin.child.as_raw_handle();
    startup.StartupInfo.hStdOutput = stdout.child.as_raw_handle();
    startup.StartupInfo.hStdError = stderr.child.as_raw_handle();
    startup.lpAttributeList = attributes.pointer();
    let mut information = PROCESS_INFORMATION::default();
    let window = InheritWindow::open([&stdin.child, &stdout.child, &stderr.child])?;
    // SAFETY: every pointer names storage that lives through the call (and, for the attribute list, until it is
    // deleted below): the exact application, a mutable NUL-terminated command line, a double-NUL environment
    // block, the working directory and the startup information.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            block.as_mut_ptr().cast(),
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut information,
        )
    };
    let failure = (created == 0).then(io::Error::last_os_error);
    // The window closes before anything else is done, created or not.
    drop(window);
    drop(attributes);
    // The child's ends are closed here: the helper has its own copies.
    drop((stdin.child, stdout.child, stderr.child));
    if let Some(error) = failure {
        return Err(error);
    }
    // SAFETY: a successful call returns two new handles that nothing else owns.
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess),
            OwnedHandle::from_raw_handle(information.hThread),
        )
    };
    drop(thread);
    let mut member = 0;
    // SAFETY: both handles are open.
    let inside =
        unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &mut member) };
    if inside == 0 || member == 0 {
        // SAFETY: the process handle is open. The helper is already running, so it is ended at once; the caller's
        // job ends anything it started.
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
        return Err(io::Error::other("the helper is not a member of its job"));
    }
    Ok(HelperChild::new(
        process,
        stdin_parent,
        stdout_parent,
        stderr_parent,
    ))
}

/// Registers a parent pipe end with the runtime's completion port.
fn server(parent: OwnedHandle) -> io::Result<NamedPipeServer> {
    // SAFETY: the handle is an open overlapped named-pipe server end, and the server takes ownership of it.
    unsafe { NamedPipeServer::from_raw_handle(parent.into_raw_handle()) }
}
