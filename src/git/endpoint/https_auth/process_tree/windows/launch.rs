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
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
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
fn environment_block(
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> io::Result<Vec<u16>> {
    let mut entries = Vec::with_capacity(environment.len());
    for (name, value) in environment {
        let name = wide(name)?;
        if name.is_empty() || name.contains(&(b'=' as u16)) {
            return Err(invalid("an environment name that is empty or holds '='"));
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
    drop(attributes);
    let (stdin_parent, stdout_parent, stderr_parent) = (stdin.parent, stdout.parent, stderr.parent);
    // The child's ends are closed here, created or not: the helper has its own copies.
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
    let wrapped = (
        ChildStdin::from_std(std::process::ChildStdin::from(stdin_parent))?,
        ChildStdout::from_std(std::process::ChildStdout::from(stdout_parent))?,
        ChildStderr::from_std(std::process::ChildStderr::from(stderr_parent))?,
    );
    Ok(HelperChild::new(process, wrapped.0, wrapped.1, wrapped.2))
}
