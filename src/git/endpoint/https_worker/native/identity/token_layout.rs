//! Reading the identifiers out of the bytes `GetTokenInformation` returned, without the Windows API: each function takes
//! the bytes the call reported (its returned size, never the size of the buffer it was given) and refuses anything that
//! does not fit inside them. The layouts are Windows' own (`TOKEN_USER`, `TOKEN_STATISTICS`, a `DWORD`), which the
//! Windows reader checks against `windows-sys` at compile time, so the parsing runs and is tested on every platform.

use std::ops::Range;

/// The bytes of a `SID_AND_ATTRIBUTES`: a pointer, then a `DWORD`, padded to the pointer's alignment.
pub(crate) const USER_HEADER: usize = 2 * std::mem::size_of::<usize>();
/// Where `AuthenticationId` sits in a `TOKEN_STATISTICS`, after the token's own `LUID`.
pub(crate) const LOGON_OFFSET: usize = 8;
/// The bytes of a SID before its sub-authorities: revision, sub-authority count, and the 6-byte identifier authority.
const SID_HEADER: usize = 8;
/// The only SID revision there is.
const SID_REVISION: u8 = 1;
/// `SID_MAX_SUB_AUTHORITIES`.
const SID_MAX_SUB_AUTHORITIES: usize = 15;
/// A SID's alignment: its sub-authorities are `DWORD`s, and Windows hands a SID out `DWORD`-aligned.
const SID_ALIGN: usize = std::mem::align_of::<u32>();

/// Where the user's SID lies in `buffer`, the `TokenUser` bytes the call returned, whose first byte is at the address
/// `base`: the `TOKEN_USER` points at the SID, which follows the structure, and the SID must lie whole inside `buffer`
/// from the offset the pointer gives. `None` when the pointer is not inside the bytes, the offset is not a multiple of 4
/// (`DWORD`-aligned in a buffer that is), or the SID is not a whole one.
pub(crate) fn user_sid(buffer: &[u8], base: usize) -> Option<Range<usize>> {
    let word = std::mem::size_of::<usize>();
    let pointer = usize::from_ne_bytes(buffer.get(..word)?.try_into().ok()?);
    let start = pointer.checked_sub(base)?;
    if start < USER_HEADER {
        return None;
    }
    if !start.is_multiple_of(SID_ALIGN) {
        return None;
    }
    let header = buffer.get(start..start.checked_add(SID_HEADER)?)?;
    let count = usize::from(header[1]);
    if header[0] != SID_REVISION || count > SID_MAX_SUB_AUTHORITIES {
        return None;
    }
    let end = start.checked_add(SID_HEADER + 4 * count)?;
    if end > buffer.len() {
        return None;
    }
    Some(start..end)
}

/// The logon session's `LUID` (`AuthenticationId`) of the `TokenStatistics` bytes the call returned, as they lie.
pub(crate) fn logon(buffer: &[u8]) -> Option<[u8; 8]> {
    buffer.get(LOGON_OFFSET..LOGON_OFFSET + 8)?.try_into().ok()
}

/// The session id (a `DWORD`) of the `TokenSessionId` bytes the call returned, as it lies.
pub(crate) fn session(buffer: &[u8]) -> Option<[u8; 4]> {
    buffer.get(..4)?.try_into().ok()
}
