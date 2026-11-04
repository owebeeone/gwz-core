//! Windows' checks after the grammar (parked server design §3 "Windows' checks after the grammar"). There is no walk:
//! the grammar admits one local name in `\\.\pipe\` and nothing else, and `CreateFileW` resolves it once. Two checks
//! follow the grammar, both judged in `grammar` so that every host runs their rows:
//! - `GetFullPathNameW` must return the name unchanged;
//! - `QueryDosDeviceW(L"PIPE")` must answer `\Device\NamedPipe` first, so that a `pipe` device redefined in the caller's
//!   logon session is not followed (defence in depth: only unsandboxed code of the same user can redefine it).
use super::{
    Platform, ServerAddress, Source, WindowsFault, judge_full_path, judge_pipe_device, parse_for,
};
use std::{io, iter, ptr};
use windows_sys::Win32::Storage::FileSystem::{GetFullPathNameW, QueryDosDeviceW};

/// The grammar this host applies.
pub(crate) const HOST: Platform = Platform::Windows;

fn os(call: &'static str) -> WindowsFault {
    WindowsFault::Os {
        call,
        code: io::Error::last_os_error().raw_os_error(),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(iter::once(0)).collect()
}

/// What `GetFullPathNameW` makes of `name`.
fn full_path(name: &str) -> Result<String, WindowsFault> {
    let name = wide(name);
    let mut buffer = vec![0u16; 512];
    loop {
        let size = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: `name` is NUL-terminated and `buffer` is valid for `size` units; the file part is not asked for.
        let length =
            unsafe { GetFullPathNameW(name.as_ptr(), size, buffer.as_mut_ptr(), ptr::null_mut()) };
        if length == 0 {
            return Err(os("GetFullPathNameW"));
        }
        let length = length as usize;
        if length < buffer.len() {
            return Ok(String::from_utf16_lossy(&buffer[..length]));
        }
        // Too small: `length` is the size needed, its NUL included.
        buffer.resize(length, 0);
    }
}

/// The targets `QueryDosDeviceW(L"PIPE")` lists, NUL-separated.
fn pipe_device_targets() -> Result<Vec<u16>, WindowsFault> {
    let device = wide("PIPE");
    let mut buffer = vec![0u16; 1024];
    let size = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: `device` is NUL-terminated and `buffer` is valid for `size` units.
    let length = unsafe { QueryDosDeviceW(device.as_ptr(), buffer.as_mut_ptr(), size) };
    if length == 0 {
        return Err(os("QueryDosDeviceW"));
    }
    buffer.truncate(length as usize);
    Ok(buffer)
}

/// A pipe name that passed the grammar and both checks: the one input of the pipe's connect and create
/// (`local::windows`; plan 2.3e2), which only [`check_pipe`] makes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckedPipe(String);

impl CheckedPipe {
    pub(crate) fn name(&self) -> &str {
        &self.0
    }
}

/// Both checks on the pipe form of a parsed address. `None` for any other form, or for a pipe name this host's grammar
/// does not give back as it is: it names no pipe. A call that fails refuses.
pub(crate) fn check_pipe(address: &ServerAddress) -> Option<Result<CheckedPipe, WindowsFault>> {
    let ServerAddress::Pipe(name) = address else {
        return None;
    };
    // The variant can be built by hand, so the grammar is asked again.
    if parse_for(HOST, Source::Bridge, name.as_bytes())
        .ok()
        .as_ref()
        != Some(address)
    {
        return None;
    }
    Some(checks(name).map(|()| CheckedPipe(name.clone())))
}

/// Both checks on a pipe name that passed the grammar. A call that fails refuses.
pub(crate) fn checks(name: &str) -> Result<(), WindowsFault> {
    judge_full_path(name, &full_path(name)?)?;
    judge_pipe_device(&pipe_device_targets()?)
}

/// The pipe checks of the pipe form of `name` ([`check_pipe`]), a refusal as the carrier reports it.
pub(crate) fn check_name(name: &str) -> Result<CheckedPipe, super::super::LocalRefusal> {
    let refused = |text: String| super::super::LocalRefusal::Address {
        unavailable: false,
        text,
    };
    match check_pipe(&ServerAddress::Pipe(name.to_owned())) {
        Some(checked) => checked.map_err(|fault| refused(fault.to_string())),
        None => Err(refused(format!("{name} names no pipe"))),
    }
}
