//! One anonymous pipe whose parent end is overlapped, as std's child pipes require, and whose child end is the
//! only inheritable handle: the parent end is never inheritable, so no other process spawn can take it.
use std::{
    io,
    os::windows::io::{FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
        OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
    },
    System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    },
};

/// A 64 KiB pipe, the capacity of a typical Linux default.
const CAPACITY: u32 = 64 * 1024;

/// Which end the parent uses.
#[derive(Clone, Copy)]
pub(super) enum Parent {
    Reads,
    Writes,
}

pub(super) struct Pipe {
    /// The parent's end: overlapped and not inheritable.
    pub(super) parent: OwnedHandle,
    /// The child's end: inheritable, to be closed once the child exists.
    pub(super) child: OwnedHandle,
}

pub(super) fn create(parent: Parent) -> io::Result<Pipe> {
    // A name nobody can guess is the whole access control of the window between creating the server end and
    // opening the client end; the first-instance flag refuses a squatted name.
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let name: Vec<u16> = format!(
        r"\\.\pipe\gwz-helper-{}",
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let (server_access, client_access) = match parent {
        Parent::Reads => (PIPE_ACCESS_INBOUND, GENERIC_WRITE),
        Parent::Writes => (PIPE_ACCESS_OUTBOUND, GENERIC_READ),
    };
    // SAFETY: `name` is NUL-terminated and outlives the call; no security attributes make the handle
    // non-inheritable; the result is checked and owned at once.
    let server = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            server_access | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            CAPACITY,
            CAPACITY,
            0,
            std::ptr::null(),
        )
    };
    if server == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `server` is a new handle that nothing else owns.
    let parent_end = unsafe { OwnedHandle::from_raw_handle(server) };
    let inheritable = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: `name` is NUL-terminated and outlives the call; the attributes are initialized.
    let client = unsafe {
        CreateFileW(
            name.as_ptr(),
            client_access,
            0,
            &inheritable,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if client == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(Pipe {
        parent: parent_end,
        // SAFETY: `client` is a new handle that nothing else owns.
        child: unsafe { OwnedHandle::from_raw_handle(client) },
    })
}
