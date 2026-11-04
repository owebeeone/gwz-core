//! The record a server keeps in its lock file (parked server design §4 "The record"; split doc :229): its process ID,
//! its address, the workspace root and its log path, written through the lock's descriptor once the stale check has
//! passed. It is a claim and nothing more: any process of the user may write the file, so a reader parses it as data.
//!
//! The form is text, one field a line, ending with `end` so that a record a starter is still writing reads as
//! [`Record::Incomplete`]:
//!
//! ```text
//! gwz-server-record 1
//! pid <decimal>
//! address <escaped>
//! root <escaped>
//! log <escaped>
//! end
//! ```
//!
//! A path is escaped byte by byte: `%`, a line feed, a carriage return and every byte that is not printable ASCII are
//! `%XX`, so a path with a newline in it cannot end its field early.
use std::path::{Path, PathBuf};

/// The most a reader reads of a lock file. The Windows lock covers the byte at this offset, past every record.
pub(crate) const RECORD_MAX: u64 = 64 * 1024;

const HEADER: &str = "gwz-server-record 1";

/// What a lock file says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Record {
    /// Nothing: no server has recorded itself, or the last one cleared its record at a clean exit.
    None,
    /// Something that is not a whole record: a starter is writing it, or it is not gwz's.
    Incomplete,
    Complete(ServerRecord),
}

/// A whole record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServerRecord {
    pub(crate) pid: u32,
    /// The socket path (Linux, macOS) or the pipe name (Windows).
    pub(crate) address: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) log: PathBuf,
}

/// Why a path cannot be recorded.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct NotRecordable(pub(crate) PathBuf);

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        fn path_bytes(path: &Path) -> Result<Vec<u8>, NotRecordable> {
            use std::os::unix::ffi::OsStrExt;
            Ok(path.as_os_str().as_bytes().to_vec())
        }

        fn path_from(bytes: Vec<u8>) -> Option<PathBuf> {
            use std::os::unix::ffi::OsStringExt;
            Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        }
    } else if #[cfg(windows)] {
        /// A Windows path is recorded as UTF-8; one with an unpaired surrogate cannot be.
        fn path_bytes(path: &Path) -> Result<Vec<u8>, NotRecordable> {
            path.to_str().map(|text| text.as_bytes().to_vec()).ok_or_else(|| NotRecordable(path.to_path_buf()))
        }

        fn path_from(bytes: Vec<u8>) -> Option<PathBuf> {
            String::from_utf8(bytes).ok().map(PathBuf::from)
        }
    }
}

fn escape(path: &Path) -> Result<String, NotRecordable> {
    let mut out = String::new();
    for byte in path_bytes(path)? {
        if (byte.is_ascii_graphic() && byte != b'%') || byte == b' ' {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(out)
}

fn unescape(text: &str) -> Option<PathBuf> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    path_from(bytes)
}

impl ServerRecord {
    /// The record's bytes.
    ///
    /// # Errors
    ///
    /// A path that cannot be recorded (Windows: not Unicode).
    pub(crate) fn encode(&self) -> Result<Vec<u8>, NotRecordable> {
        let text = format!(
            "{HEADER}\npid {}\naddress {}\nroot {}\nlog {}\nend\n",
            self.pid,
            escape(&self.address)?,
            escape(&self.root)?,
            escape(&self.log)?
        );
        Ok(text.into_bytes())
    }
}

impl Record {
    /// What `bytes`, the first [`RECORD_MAX`] bytes of a lock file, say.
    pub(crate) fn parse(bytes: &[u8]) -> Record {
        if bytes.is_empty() {
            return Record::None;
        }
        Self::complete(bytes).map_or(Record::Incomplete, Record::Complete)
    }

    fn complete(bytes: &[u8]) -> Option<ServerRecord> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.strip_suffix('\n')?.split('\n');
        if lines.next()? != HEADER {
            return None;
        }
        let mut field = |name: &str| lines.next()?.strip_prefix(name)?.strip_prefix(' ');
        let pid = field("pid")?.parse().ok()?;
        let address = unescape(field("address")?)?;
        let root = unescape(field("root")?)?;
        let log = unescape(field("log")?)?;
        (lines.next()? == "end" && lines.next().is_none()).then_some(ServerRecord {
            pid,
            address,
            root,
            log,
        })
    }

    /// Whether the record is whole and names exactly `address`.
    pub(crate) fn names(&self, address: &Path) -> bool {
        matches!(self, Record::Complete(record) if record.address == address)
    }
}
