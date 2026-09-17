//! The clone copy record: what `gwz local clone` copied, per repository
//! (`dev-docs/GwzLaneCleanFixes.md` R1, plan `GwzLaneCleanFixesPlan.md`
//! S1.1).
//!
//! # Why it exists
//!
//! A verbatim lane inherits every ignored entry, every native stash entry
//! and every reflog-only commit of its source. With no record of that,
//! `gwz local dispose` has no baseline and must treat all of it as the
//! lane's own work: 112 hazard entries for a lane of the gwz-dev workspace,
//! identical in every lane, none of them the lane's doing (gwz-dev
//! `dev-docs/GwzLaneIssues.md`, L1). This file is that baseline.
//!
//! # Where it lives, and what it is not (plan decision D1)
//!
//! [`COPY_RECORD_RELATIVE_PATH`]: the **lane's** own `.gwz/`, beside its
//! family pointer (`.gwz/family-root`) and its allocation marker
//! (`.gwz/local-clone-allocation`). It is **not** a member-row field and
//! **not** part of the family index, so the index schema and the row are
//! untouched by this work.
//!
//! It is *evidence*, not authority. R2 clears an entry only when it is
//! **both** unchanged since this record **and** still present in the
//! surviving family, so a record inside the very tree being deleted can
//! never certify a loss on its own. A lane with no record at all is not a
//! refusal either: R3 has dispose make the comparison itself.
//!
//! # Format
//!
//! One frozen schema string, `deny_unknown_fields` in both directions, and
//! a `schema:` that is not this version refuses as *malformed* rather than
//! being accepted or upgraded -- the same strictness, and for the same
//! reason, as the pointer and marker formats in `gwz-family-store`. An
//! unreadable or undecodable record is never silently "no record": the
//! caller reports it, and disposal treats it as unknown evidence.
//!
//! Paths are Git's raw bytes, which need not be UTF-8, so every recorded
//! path is stored percent-escaped ([`escape_path`]) and round-trips byte
//! for byte.

use std::fmt;
use std::fs;
use std::path::Path;

use gwz_repo_contract::{BytePath, ObjectFormat, ObjectId, RepoKey, RootSource, WorkKind};
use serde::{Deserialize, Serialize};

/// The record's frozen `schema:` value.
pub const COPY_RECORD_SCHEMA: &str = "gwz.local-clone-copy/v1";

/// Where the record stands, relative to the lane's own root.
pub const COPY_RECORD_RELATIVE_PATH: &str = ".gwz/local-clone-copy.yml";

/// What one `gwz local clone` copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyRecord {
    /// The family the lane belongs to, as the pointer records it.
    pub family_id: String,
    /// The allocation this record belongs to; a record whose allocation is
    /// not the row's describes an earlier tenant of the path and is not
    /// this lane's baseline.
    pub allocation_id: String,
    /// The root-relative path of the member that was copied (`.` for the
    /// root), matching `MemberRow::source_path`.
    pub source_path: String,
    /// The clone mode that made the lane, matching `MemberRow::mode`.
    pub mode: String,
    /// One entry per repository inside the lane, in inventory order.
    pub repositories: Vec<RepositoryCopy>,
}

/// One repository's copied history roots and copied on-disk entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryCopy {
    pub key: RepoKey,
    /// Every protected root the copy carried: refs, `HEAD`, reflog
    /// entries, native stash entries and annotated tags.
    pub roots: Vec<CopiedRoot>,
    /// Every untracked and ignored entry the copy carried, with the cheap
    /// fingerprint R1 asks for.
    pub entries: Vec<CopiedEntry>,
}

/// One protected root as the copy carried it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedRoot {
    pub source: RootSource,
    pub oid: ObjectId,
}

/// One copied worktree entry and its fingerprint. The fingerprint is
/// deliberately cheap -- one `stat` -- because a lane of this workspace
/// holds tens of thousands of ignored files under a handful of recorded
/// directories (R1, R13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedEntry {
    /// Repository-relative, as Git reports it: raw bytes, `/` separators,
    /// a trailing `/` for a directory reported whole.
    pub path: BytePath,
    pub kind: WorkKind,
    pub fingerprint: Fingerprint,
}

/// What one `stat` established. `inode` is `0` where the platform has no
/// inode number; comparison treats a zero on both sides as "no evidence
/// from this field", never as a match of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub inode: u64,
}

impl Fingerprint {
    /// The fingerprint of `path`, without following a symbolic link: a lane
    /// carries build-tool convenience symlinks, and following one would
    /// fingerprint something outside the lane.
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = fs::symlink_metadata(path).ok()?;
        let (mtime_secs, mtime_nanos) = modified_parts(&metadata);
        Some(Self {
            size: metadata.len(),
            mtime_secs,
            mtime_nanos,
            inode: inode_of(&metadata),
        })
    }
}

fn modified_parts(metadata: &fs::Metadata) -> (i64, u32) {
    let Ok(modified) = metadata.modified() else {
        return (0, 0);
    };
    match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => (since.as_secs() as i64, since.subsec_nanos()),
        Err(before) => {
            let duration = before.duration();
            (-(duration.as_secs() as i64), duration.subsec_nanos())
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt as _;

        fn inode_of(metadata: &fs::Metadata) -> u64 {
            metadata.ino()
        }
    } else {
        fn inode_of(_metadata: &fs::Metadata) -> u64 {
            0
        }
    }
}

/// The host path of the recorded entry `path` (repository-relative, Git's
/// raw bytes, possibly with the trailing `/` of a directory reported whole)
/// inside the repository at `base`.
///
/// `None` where the host cannot spell those bytes as a path, which on
/// Windows is any non-UTF-8 entry: the entry is then simply not recorded,
/// and disposal refuses over it rather than guessing.
pub fn entry_path(base: &Path, path: &[u8]) -> Option<std::path::PathBuf> {
    let trimmed = path.strip_suffix(b"/").unwrap_or(path);
    if trimmed.is_empty() {
        return None;
    }
    relative_of(trimmed).map(|relative| base.join(relative))
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        fn relative_of(bytes: &[u8]) -> Option<std::path::PathBuf> {
            Some(std::path::PathBuf::from(OsStr::from_bytes(bytes)))
        }
    } else {
        fn relative_of(bytes: &[u8]) -> Option<std::path::PathBuf> {
            std::str::from_utf8(bytes).ok().map(std::path::PathBuf::from)
        }
    }
}

/// A record that could not be read or decoded. The caller adds the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyRecordError(String);

impl CopyRecordError {
    fn new(detail: impl Into<String>) -> Self {
        Self(detail.into())
    }

    pub fn detail(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CopyRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CopyRecordError {}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordFile {
    schema: String,
    family_id: String,
    allocation_id: String,
    source_path: String,
    mode: String,
    #[serde(default)]
    repositories: Vec<RepositoryFile>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RepositoryFile {
    key: String,
    #[serde(default)]
    roots: Vec<RootFile>,
    #[serde(default)]
    entries: Vec<EntryFile>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RootFile {
    source: String,
    /// `sha1` or `sha256`; the object format is part of the id, so a record
    /// never has to guess it from the digest length.
    format: String,
    oid: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EntryFile {
    path: String,
    kind: String,
    size: u64,
    mtime_secs: i64,
    mtime_nanos: u32,
    inode: u64,
}

/// Encode `record` as the bytes that stand at [`COPY_RECORD_RELATIVE_PATH`].
pub fn encode(record: &CopyRecord) -> Result<Vec<u8>, CopyRecordError> {
    let file = RecordFile {
        schema: COPY_RECORD_SCHEMA.to_owned(),
        family_id: record.family_id.clone(),
        allocation_id: record.allocation_id.clone(),
        source_path: record.source_path.clone(),
        mode: record.mode.clone(),
        repositories: record
            .repositories
            .iter()
            .map(|repository| RepositoryFile {
                key: repository.key.to_string(),
                roots: repository
                    .roots
                    .iter()
                    .map(|root| RootFile {
                        source: encode_source(&root.source),
                        format: format_name(root.oid.format()).to_owned(),
                        oid: root.oid.to_hex(),
                    })
                    .collect(),
                entries: repository
                    .entries
                    .iter()
                    .map(|entry| EntryFile {
                        path: escape_path(&entry.path),
                        kind: kind_name(entry.kind).to_owned(),
                        size: entry.fingerprint.size,
                        mtime_secs: entry.fingerprint.mtime_secs,
                        mtime_nanos: entry.fingerprint.mtime_nanos,
                        inode: entry.fingerprint.inode,
                    })
                    .collect(),
            })
            .collect(),
    };
    serde_yaml::to_string(&file)
        .map(String::into_bytes)
        .map_err(|error| CopyRecordError::new(error.to_string()))
}

/// Decode the bytes at [`COPY_RECORD_RELATIVE_PATH`]. An unknown field or a
/// `schema:` that is not [`COPY_RECORD_SCHEMA`] refuses.
pub fn decode(bytes: &[u8]) -> Result<CopyRecord, CopyRecordError> {
    let file: RecordFile =
        serde_yaml::from_slice(bytes).map_err(|error| CopyRecordError::new(error.to_string()))?;
    if file.schema != COPY_RECORD_SCHEMA {
        return Err(CopyRecordError::new(format!(
            "`schema: {}` is not `{COPY_RECORD_SCHEMA}`; this file is not a copy record this \
             build reads",
            file.schema
        )));
    }
    let mut repositories = Vec::with_capacity(file.repositories.len());
    for repository in file.repositories {
        let key = decode_key(&repository.key);
        let mut roots = Vec::with_capacity(repository.roots.len());
        for root in repository.roots {
            let format = decode_format(&root.format)?;
            roots.push(CopiedRoot {
                source: decode_source(&root.source)?,
                oid: ObjectId::parse_hex(format, &root.oid).map_err(|error| {
                    CopyRecordError::new(format!("`{}`: `oid`: {error:?}", repository.key))
                })?,
            });
        }
        let mut entries = Vec::with_capacity(repository.entries.len());
        for entry in repository.entries {
            entries.push(CopiedEntry {
                path: unescape_path(&entry.path).ok_or_else(|| {
                    CopyRecordError::new(format!(
                        "`{}`: `path: {}` is not a valid escaped path",
                        repository.key, entry.path
                    ))
                })?,
                kind: decode_kind(&entry.kind).ok_or_else(|| {
                    CopyRecordError::new(format!(
                        "`{}`: `kind: {}` is not a known work kind",
                        repository.key, entry.kind
                    ))
                })?,
                fingerprint: Fingerprint {
                    size: entry.size,
                    mtime_secs: entry.mtime_secs,
                    mtime_nanos: entry.mtime_nanos,
                    inode: entry.inode,
                },
            });
        }
        repositories.push(RepositoryCopy {
            key,
            roots,
            entries,
        });
    }
    Ok(CopyRecord {
        family_id: file.family_id,
        allocation_id: file.allocation_id,
        source_path: file.source_path,
        mode: file.mode,
        repositories,
    })
}

/// Read the record of the lane at `lane`. `Ok(None)` is the R3 case -- no
/// record, so dispose makes the comparison itself -- and is returned only
/// when the file is genuinely absent; every other failure is an error, so
/// an unreadable record is never mistaken for an absent one.
pub fn read(lane: &Path) -> Result<Option<CopyRecord>, CopyRecordError> {
    let path = lane.join(COPY_RECORD_RELATIVE_PATH);
    match fs::read(&path) {
        Ok(bytes) => decode(&bytes)
            .map(Some)
            .map_err(|error| CopyRecordError::new(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CopyRecordError::new(format!("{}: {error}", path.display()))),
    }
}

/// Write `record` into the lane at `lane`, creating its `.gwz/` directory.
pub fn write(lane: &Path, record: &CopyRecord) -> Result<(), CopyRecordError> {
    let path = lane.join(COPY_RECORD_RELATIVE_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| CopyRecordError::new(format!("{}: {error}", parent.display())))?;
    }
    let bytes = encode(record)?;
    fs::write(&path, bytes)
        .map_err(|error| CopyRecordError::new(format!("{}: {error}", path.display())))
}

fn format_name(format: ObjectFormat) -> &'static str {
    match format {
        ObjectFormat::Sha1 => "sha1",
        ObjectFormat::Sha256 => "sha256",
    }
}

fn decode_format(name: &str) -> Result<ObjectFormat, CopyRecordError> {
    match name {
        "sha1" => Ok(ObjectFormat::Sha1),
        "sha256" => Ok(ObjectFormat::Sha256),
        other => Err(CopyRecordError::new(format!(
            "`format: {other}` is not a known object format"
        ))),
    }
}

fn kind_name(kind: WorkKind) -> &'static str {
    match kind {
        WorkKind::Staged => "staged",
        WorkKind::Unstaged => "unstaged",
        WorkKind::Untracked => "untracked",
        WorkKind::Ignored => "ignored",
        WorkKind::Conflict => "conflict",
        WorkKind::ModeChange => "mode-change",
        WorkKind::LinkChange => "link-change",
        WorkKind::Renamed => "renamed",
        WorkKind::Deleted => "deleted",
    }
}

fn decode_kind(name: &str) -> Option<WorkKind> {
    match name {
        "staged" => Some(WorkKind::Staged),
        "unstaged" => Some(WorkKind::Unstaged),
        "untracked" => Some(WorkKind::Untracked),
        "ignored" => Some(WorkKind::Ignored),
        "conflict" => Some(WorkKind::Conflict),
        "mode-change" => Some(WorkKind::ModeChange),
        "link-change" => Some(WorkKind::LinkChange),
        "renamed" => Some(WorkKind::Renamed),
        "deleted" => Some(WorkKind::Deleted),
        _ => None,
    }
}

/// `@root` for the root; the manifest id otherwise, exactly as `RepoKey`
/// displays itself, so a record reads like every other GWZ report.
fn decode_key(key: &str) -> RepoKey {
    if key == "@root" {
        return RepoKey::Root;
    }
    RepoKey::Member { id: key.to_owned() }
}

/// A root source as one line. A Git reference name contains no space and no
/// colon, so `<tag>:<rest>` parses back unambiguously for every named
/// source; `record` and `other` carry free text and are escaped.
fn encode_source(source: &RootSource) -> String {
    match source {
        RootSource::Head => "head".to_owned(),
        RootSource::Ref { name } => format!("ref:{name}"),
        RootSource::Reflog { reference, index } => format!("reflog:{index}:{reference}"),
        RootSource::Stash { index } => format!("stash:{index}"),
        RootSource::AnnotatedTag { name } => format!("tag:{name}"),
        RootSource::CoordinationRecord { record, object } => format!(
            "record:{}:{}",
            escape_path(object.as_bytes()),
            escape_path(record.as_bytes())
        ),
        RootSource::Other { detail } => format!("other:{}", escape_path(detail.as_bytes())),
    }
}

fn decode_source(text: &str) -> Result<RootSource, CopyRecordError> {
    let unknown = || CopyRecordError::new(format!("`source: {text}` is not a known root source"));
    if text == "head" {
        return Ok(RootSource::Head);
    }
    let (tag, rest) = text.split_once(':').ok_or_else(unknown)?;
    let text_of = |value: &str| -> Result<String, CopyRecordError> {
        let bytes = unescape_path(value).ok_or_else(unknown)?;
        String::from_utf8(bytes).map_err(|_| unknown())
    };
    match tag {
        "ref" => Ok(RootSource::Ref {
            name: rest.to_owned(),
        }),
        "tag" => Ok(RootSource::AnnotatedTag {
            name: rest.to_owned(),
        }),
        "stash" => Ok(RootSource::Stash {
            index: rest.parse().map_err(|_| unknown())?,
        }),
        "reflog" => {
            let (index, reference) = rest.split_once(':').ok_or_else(unknown)?;
            Ok(RootSource::Reflog {
                reference: reference.to_owned(),
                index: index.parse().map_err(|_| unknown())?,
            })
        }
        "record" => {
            let (object, record) = rest.split_once(':').ok_or_else(unknown)?;
            Ok(RootSource::CoordinationRecord {
                record: text_of(record)?,
                object: text_of(object)?,
            })
        }
        "other" => Ok(RootSource::Other {
            detail: text_of(rest)?,
        }),
        _ => Err(unknown()),
    }
}

/// Percent-escape a raw path: printable ASCII stays as it is, `%` and every
/// other byte become `%XX`. Lossless, and a path that is ordinary UTF-8
/// reads unchanged in the file.
pub fn escape_path(path: &[u8]) -> String {
    let mut escaped = String::with_capacity(path.len());
    for &byte in path {
        if byte == b'%' || !(0x20..0x7f).contains(&byte) {
            escaped.push_str(&format!("%{byte:02X}"));
        } else {
            escaped.push(byte as char);
        }
    }
    escaped
}

/// The inverse of [`escape_path`]; `None` when the text is not a valid
/// escaping.
pub fn unescape_path(text: &str) -> Option<BytePath> {
    let bytes = text.as_bytes();
    let mut path = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = bytes.get(index + 1..index + 3)?;
            let text = std::str::from_utf8(pair).ok()?;
            path.push(u8::from_str_radix(text, 16).ok()?);
            index += 3;
        } else {
            if !(0x20..0x7f).contains(&bytes[index]) {
                return None;
            }
            path.push(bytes[index]);
            index += 1;
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(byte: u8) -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[byte; 20]).unwrap()
    }

    fn sample() -> CopyRecord {
        CopyRecord {
            family_id: "fam_0001".to_owned(),
            allocation_id: "alloc_0001".to_owned(),
            source_path: ".".to_owned(),
            mode: "verbatim".to_owned(),
            repositories: vec![
                RepositoryCopy {
                    key: RepoKey::Root,
                    roots: vec![
                        CopiedRoot {
                            source: RootSource::Head,
                            oid: oid(0x11),
                        },
                        CopiedRoot {
                            source: RootSource::Reflog {
                                reference: "refs/heads/main".to_owned(),
                                index: 7,
                            },
                            oid: oid(0x22),
                        },
                        CopiedRoot {
                            source: RootSource::Stash { index: 2 },
                            oid: oid(0x33),
                        },
                    ],
                    entries: vec![CopiedEntry {
                        path: b"target/".to_vec(),
                        kind: WorkKind::Ignored,
                        fingerprint: Fingerprint {
                            size: 4096,
                            mtime_secs: 1_700_000_000,
                            mtime_nanos: 123,
                            inode: 99,
                        },
                    }],
                },
                RepositoryCopy {
                    key: RepoKey::Member {
                        id: "mem_app".to_owned(),
                    },
                    roots: vec![
                        CopiedRoot {
                            source: RootSource::Ref {
                                name: "refs/heads/work".to_owned(),
                            },
                            oid: oid(0x44),
                        },
                        CopiedRoot {
                            source: RootSource::AnnotatedTag {
                                name: "refs/tags/v1".to_owned(),
                            },
                            oid: oid(0x55),
                        },
                        CopiedRoot {
                            source: RootSource::CoordinationRecord {
                                record: "stash gwz_stash_0007".to_owned(),
                                object: "base".to_owned(),
                            },
                            oid: oid(0x66),
                        },
                        CopiedRoot {
                            source: RootSource::Other {
                                detail: "an unread: thing".to_owned(),
                            },
                            oid: oid(0x77),
                        },
                    ],
                    // A path Git can hold and YAML cannot: not UTF-8.
                    entries: vec![CopiedEntry {
                        path: vec![b'd', b'a', b't', b'a', b'/', 0xff, b'.', b'b', b'i', b'n'],
                        kind: WorkKind::Untracked,
                        fingerprint: Fingerprint::default(),
                    }],
                },
            ],
        }
    }

    /// Every source, every key and a non-UTF-8 path survive the round trip
    /// byte for byte, and the encoding is stable.
    #[test]
    fn a_record_round_trips_and_encodes_the_same_bytes_twice() {
        let record = sample();
        let bytes = encode(&record).unwrap();
        assert_eq!(decode(&bytes).unwrap(), record);
        assert_eq!(encode(&decode(&bytes).unwrap()).unwrap(), bytes);
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.starts_with("schema: gwz.local-clone-copy/v1\n"),
            "{text}"
        );
        assert!(text.contains("path: data/%FF.bin"), "{text}");
        assert!(text.contains("source: reflog:7:refs/heads/main"), "{text}");
    }

    /// An unknown schema, an unknown field and an unknown value each refuse
    /// rather than being accepted or upgraded, and the detail names what
    /// was wrong.
    #[test]
    fn an_unreadable_record_refuses_rather_than_reading_as_empty() {
        let future = b"schema: gwz.local-clone-copy/v2\nfamily_id: f\nallocation_id: a\n\
                       source_path: .\nmode: verbatim\n";
        let error = decode(future).unwrap_err();
        assert!(
            error.detail().contains("gwz.local-clone-copy/v2"),
            "{error}"
        );

        let extra = b"schema: gwz.local-clone-copy/v1\nfamily_id: f\nallocation_id: a\n\
                      source_path: .\nmode: verbatim\nsurprise: 1\n";
        assert!(decode(extra).is_err(), "an unknown field refuses");

        let bad_kind = b"schema: gwz.local-clone-copy/v1\nfamily_id: f\nallocation_id: a\n\
                         source_path: .\nmode: verbatim\nrepositories:\n- key: '@root'\n  \
                         entries:\n  - path: x\n    kind: invented\n    size: 0\n    \
                         mtime_secs: 0\n    mtime_nanos: 0\n    inode: 0\n";
        let error = decode(bad_kind).unwrap_err();
        assert!(error.detail().contains("invented"), "{error}");

        let bad_source = b"schema: gwz.local-clone-copy/v1\nfamily_id: f\nallocation_id: a\n\
                           source_path: .\nmode: verbatim\nrepositories:\n- key: '@root'\n  \
                           roots:\n  - source: invented\n    format: sha1\n    oid: '00'\n";
        assert!(
            decode(bad_source).is_err(),
            "an unknown root source refuses"
        );
    }

    /// Escaping is lossless in both directions, and a malformed escaping is
    /// refused rather than guessed at.
    #[test]
    fn paths_escape_losslessly_and_refuse_a_malformed_escaping() {
        for raw in [
            &b""[..],
            b"src/lib.rs",
            b"a b/c.txt",
            b"100%/of it",
            &[0x00, 0x7f, 0xff][..],
        ] {
            let escaped = escape_path(raw);
            assert!(escaped.is_ascii(), "{escaped}");
            assert_eq!(unescape_path(&escaped).as_deref(), Some(raw), "{escaped}");
        }
        assert_eq!(escape_path(b"100%"), "100%25");
        assert_eq!(unescape_path("%F"), None);
        assert_eq!(unescape_path("%ZZ"), None);
        assert_eq!(unescape_path("a\u{e9}b"), None);
    }

    /// An absent record is the R3 case and reads as `None`; an undecodable
    /// one is an error, never mistaken for an absent one. A written record
    /// reads back as itself.
    #[test]
    fn an_absent_record_is_none_and_a_broken_one_is_an_error() {
        let lane = std::env::temp_dir().join(format!(
            "gwz-copy-record-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&lane);
        fs::create_dir_all(&lane).unwrap();

        assert_eq!(read(&lane).unwrap(), None);

        let record = sample();
        write(&lane, &record).unwrap();
        assert_eq!(read(&lane).unwrap(), Some(record));

        fs::write(lane.join(COPY_RECORD_RELATIVE_PATH), b"schema: nope\n").unwrap();
        let error = read(&lane).unwrap_err();
        assert!(
            error.detail().contains(COPY_RECORD_RELATIVE_PATH),
            "{error}"
        );

        fs::remove_dir_all(&lane).unwrap();
    }

    /// The fingerprint is one `stat` that does not follow a symbolic link:
    /// a build tool's convenience link is fingerprinted as the link, not as
    /// whatever it points at outside the lane.
    #[test]
    fn a_fingerprint_describes_the_entry_and_never_its_link_target() {
        let dir = std::env::temp_dir().join(format!("gwz-fingerprint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data");
        fs::write(&file, b"twelve bytes").unwrap();
        let fingerprint = Fingerprint::of(&file).unwrap();
        assert_eq!(fingerprint.size, 12);

        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                let link = dir.join("bazel-out");
                std::os::unix::fs::symlink(&file, &link).unwrap();
                let linked = Fingerprint::of(&link).unwrap();
                assert_ne!(linked.size, fingerprint.size, "the link, not its target");
                assert_ne!(linked.inode, fingerprint.inode);
            }
        }

        assert_eq!(Fingerprint::of(&dir.join("absent")), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
