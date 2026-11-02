//! The caller's identity on Windows: the process primary token's user SID, logon-session LUID and session id. The native
//! route verifies a caller against this token (gwz-sspi refuses an impersonating thread), and a credential helper process
//! runs as it. The token is opened for query only and closed on every path; nothing but the three identifiers is copied.
//! Every read is made from the bytes `GetTokenInformation` returned, at the offsets `super::token_layout` checks.

use super::token_layout;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use windows_sys::Win32::{
    Foundation::{HANDLE, LUID},
    Security::{
        GetLengthSid, GetTokenInformation, IsValidSid, SID_AND_ATTRIBUTES, TOKEN_INFORMATION_CLASS,
        TOKEN_QUERY, TOKEN_STATISTICS, TokenSessionId, TokenStatistics, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

// The layouts `token_layout` parses are the ones windows-sys declares.
const _: () = assert!(std::mem::size_of::<SID_AND_ATTRIBUTES>() == token_layout::USER_HEADER);
const _: () =
    assert!(std::mem::offset_of!(TOKEN_STATISTICS, AuthenticationId) == token_layout::LOGON_OFFSET);
const _: () = assert!(std::mem::size_of::<LUID>() == 8);

/// The non-secret identifiers of one token, as they are folded.
pub(crate) struct Token {
    pub(crate) sid: Vec<u8>,
    pub(crate) logon: [u8; 8],
    pub(crate) session: [u8; 4],
}

/// The bytes one `GetTokenInformation` call returned, in storage aligned for the structures inside them.
struct Returned {
    words: Vec<u64>,
    length: usize,
}

impl Returned {
    /// The bytes the call reported, and no more of the buffer than that.
    fn bytes(&self) -> &[u8] {
        // SAFETY: `words` holds `words.len() * 8` initialised bytes, and `length` never exceeds that (see `class`).
        unsafe { std::slice::from_raw_parts(self.words.as_ptr() as *const u8, self.length) }
    }
}

/// The digest of this process's token identifiers, or `None` when any of them cannot be read.
pub(crate) fn read() -> Option<[u8; 32]> {
    token().map(|token| super::fold(&[&token.sid, &token.logon, &token.session]))
}

/// The identifiers of this process's primary token, or `None` when the token or any of them cannot be read.
pub(crate) fn token() -> Option<Token> {
    let mut raw: HANDLE = ptr::null_mut();
    // SAFETY: the pseudo handle of this process needs no closing; `raw` receives a token handle only on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return None;
    }
    // SAFETY: `raw` is the token handle just opened, owned by nothing else; dropping it closes it.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let token: HANDLE = handle.as_raw_handle();
    let user = class(token, TokenUser)?;
    let bytes = user.bytes();
    let range = token_layout::user_sid(bytes, bytes.as_ptr() as usize)?;
    // SAFETY: `range` lies inside the bytes the call returned, from the offset the `TOKEN_USER` gives; the SID's header and
    // all its sub-authorities are inside it (`user_sid` checked its count), so these two calls read nothing beyond it.
    // The storage is 8-byte aligned and `user_sid` refuses an offset that is not a multiple of 4, so the SID is
    // `DWORD`-aligned, as Windows hands one out.
    let sid = unsafe {
        let sid = bytes.as_ptr().add(range.start) as *mut std::ffi::c_void;
        if IsValidSid(sid) == 0 || GetLengthSid(sid) as usize != range.len() {
            return None;
        }
        bytes[range].to_vec()
    };
    let statistics = class(token, TokenStatistics)?;
    let logon = token_layout::logon(statistics.bytes())?;
    let session = class(token, TokenSessionId)?;
    let session = token_layout::session(session.bytes())?;
    Some(Token {
        sid,
        logon,
        session,
    })
}

/// The bytes of one token information class, aligned for the structures read out of them, or `None` when it cannot be
/// read.
fn class(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Returned> {
    let mut size = 0u32;
    // SAFETY: the first call only asks for the size.
    unsafe { GetTokenInformation(token, class, ptr::null_mut(), 0, &mut size) };
    if size == 0 {
        return None;
    }
    let mut words = vec![0u64; (size as usize).div_ceil(8)];
    let capacity = words.len() * 8;
    // SAFETY: the buffer holds at least `size` bytes at 8-byte alignment.
    if unsafe { GetTokenInformation(token, class, words.as_mut_ptr().cast(), size, &mut size) } == 0
    {
        return None;
    }
    // The second call reports the bytes it wrote; a reader never goes past them, nor past the buffer.
    let length = (size as usize).min(capacity);
    Some(Returned { words, length })
}
