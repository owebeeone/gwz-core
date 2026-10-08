//! The platform's roots where the TLS backend is OpenSSL: the roots of
//! OpenSSL's default verify paths, as the process the CLI is gets them.
//!
//! gwz 1.0.17 verifies HTTPS through libgit2 on OpenSSL, which trusts what
//! `SSL_CTX_set_default_verify_paths` finds and nothing else: the file
//! `SSL_CERT_FILE` names, else the system's bundle, and the directory
//! `SSL_CERT_DIR` names, else the system's. Setting either narrows trust to
//! that file or directory. The native-tls build of the transport loads the
//! same paths, and then every standard certificate directory of its own
//! beside them, so a user who narrowed trust through `SSL_CERT_FILE` or
//! `SSL_CERT_DIR` got the system's roots back. The endpoint's TLS
//! configuration therefore drops the backend's roots (`https_tls`) and takes
//! these instead.
//!
//! The paths are those of `openssl-probe` 0.1, which libgit2's host (git2)
//! runs once at its start: a variable that names an existing path stands, and
//! a variable that does not is replaced by the first standard location that
//! exists. Both files and directories are read as OpenSSL reads them, which
//! is leniently: a file is read block by block until a damaged one, a file
//! that holds no certificate holds no roots, and nothing
//! is ever refused, so that a bad file is a failed verification and not an
//! error before any connection, as it is in 1.0.17.
//!
//! The directory is read whole, where OpenSSL finds a certificate in it
//! by the hash of its subject, in a file named `<hash>.<n>`. Every file with
//! such a name is read; a file whose name is not the hash of the certificate
//! it holds, which OpenSSL never finds, is a root here. A `TRUSTED
//! CERTIFICATE` block is a root, its trust settings not being kept.
//!
//! On Debian the files of the directory are links to the certificates that the
//! bundle holds, and a root that came from the file is not read again from the
//! directory: a file whose text is that of a block already read is skipped
//! before it is parsed. The roots are the same.
//!
//! Not consulted: OpenSSL's compiled-in locations, where neither the
//! variables nor the probe's locations name an existing path.
use super::ca_bundle::{Block, scan_spans};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// The labels of a block OpenSSL reads as a certificate.
const LABELS: &[&str] = &["CERTIFICATE", "TRUSTED CERTIFICATE", "X509 CERTIFICATE"];
/// The most of one file that is read.
const MAX_FILE: u64 = 16 * 1024 * 1024;

/// The file and the directory of a set of default verify paths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Paths {
    pub(crate) file: Option<PathBuf>,
    pub(crate) dir: Option<PathBuf>,
}

impl Paths {
    /// The paths for the values of `SSL_CERT_FILE` and `SSL_CERT_DIR` (`None`
    /// where unset or empty): a value that names an existing path, and
    /// otherwise what `standard` finds, once, only if one is missing.
    pub(crate) fn resolve(
        file: Option<&OsStr>,
        dir: Option<&OsStr>,
        standard: impl FnOnce() -> Paths,
    ) -> Self {
        let existing =
            |value: Option<&OsStr>| value.map(PathBuf::from).filter(|path| path.exists());
        let (file, dir) = (existing(file), existing(dir));
        if file.is_some() && dir.is_some() {
            return Self { file, dir };
        }
        let standard = standard();
        Self {
            file: file.or(standard.file),
            dir: dir.or(standard.dir),
        }
    }

    /// The certificates the paths hold, the file's first, each once.
    pub(crate) fn roots(&self) -> Vec<native_tls::Certificate> {
        self.read().0
    }

    /// `roots`, with what reading them took.
    pub(crate) fn read(&self) -> (Vec<native_tls::Certificate>, Reads) {
        let mut seen = Seen::default();
        let mut roots = Vec::new();
        if let Some(file) = &self.file {
            read_file(file, &mut seen, &mut roots);
        }
        if let Some(dir) = &self.dir {
            let mut hashed: Vec<PathBuf> = fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| is_hashed_name(path))
                .collect();
            hashed.sort();
            for path in hashed {
                read_file(&path, &mut seen, &mut roots);
            }
        }
        (roots, seen.reads)
    }
}

/// How many files were read and parsed, and how many were skipped because
/// their text was that of a block already read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    pub(crate) read: usize,
    pub(crate) skipped: usize,
}

/// What has been read: the certificates by their DER, and the text of the
/// blocks they came from, so that a file holding only such a block is not
/// parsed again.
#[derive(Default)]
struct Seen {
    certificates: HashSet<Vec<u8>>,
    blocks: HashSet<Vec<u8>>,
    reads: Reads,
}

/// Whether `path` is named as OpenSSL names a certificate in a directory it
/// looks in: eight lower-case hexadecimal digits, a dot and a number.
fn is_hashed_name(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.split_once('.'))
        .is_some_and(|(hash, number)| {
            hash.len() == 8
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// Adds the certificates of the file at `path` that are not in `seen`, up to
/// the first block that is not one. A file that cannot be read adds none.
fn read_file(path: &Path, seen: &mut Seen, roots: &mut Vec<native_tls::Certificate>) {
    let mut bytes = Vec::new();
    let read = fs::File::open(path).and_then(|file| file.take(MAX_FILE).read_to_end(&mut bytes));
    if read.is_err() {
        return;
    }
    // Exactly one block already read: it is parsed to the same certificate.
    if seen.blocks.contains(bytes.trim_ascii()) {
        seen.reads.skipped += 1;
        return;
    }
    seen.reads.read += 1;
    let (scanned, spans) = scan_spans(&bytes, LABELS);
    for (block, span) in scanned.blocks.into_iter().zip(spans) {
        let Block::Der(der) = block else {
            return;
        };
        if seen.certificates.contains(&der) {
            seen.blocks.insert(bytes[span].to_vec());
            continue;
        }
        match native_tls::Certificate::from_der(&der) {
            Ok(certificate) => {
                seen.certificates.insert(der);
                seen.blocks.insert(bytes[span].to_vec());
                roots.push(certificate);
            }
            Err(_) => return,
        }
    }
}
