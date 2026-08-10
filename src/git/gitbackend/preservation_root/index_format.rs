use super::super::*;

use sha1::Sha1;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RawEntry {
    pub path: Vec<u8>,
    pub object_id: String,
    pub mode: u32,
    pub stage: u8,
    pub flags: u16,
    pub extended_flags: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheTreeNode {
    path: Vec<u8>,
    entry_count: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RawIndex {
    pub version: u32,
    pub entries: Vec<RawEntry>,
    cache_tree: Option<Vec<CacheTreeNode>>,
}

pub(super) fn read(repo: &git2::Repository) -> ModelResult<RawIndex> {
    let bytes = std::fs::read(repo.path().join("index")).map_err(crate::git::io_error)?;
    parse(&bytes, repo.object_format())
}

pub(super) fn require_managed_tree_invalidation(
    index: &RawIndex,
    managed_paths: &[&[u8]],
) -> ModelResult<()> {
    let Some(nodes) = &index.cache_tree else {
        return Ok(());
    };
    for node in nodes {
        if managed_paths
            .iter()
            .any(|path| node.path.is_empty() || is_ancestor(&node.path, path))
            && node.entry_count != -1
        {
            return Err(evidence_error(
                "cache-tree root or managed ancestor was not invalidated",
            ));
        }
    }
    Ok(())
}

fn parse(bytes: &[u8], format: git2::ObjectFormat) -> ModelResult<RawIndex> {
    let oid_width = oid_width(format);
    if bytes.len() < 12 + oid_width {
        return Err(evidence_error("Git index is truncated"));
    }
    let trailer_at = bytes.len() - oid_width;
    let (body, trailer) = bytes.split_at(trailer_at);
    let digest = match format {
        git2::ObjectFormat::Sha1 => Sha1::digest(body).to_vec(),
        git2::ObjectFormat::Sha256 => Sha256::digest(body).to_vec(),
    };
    if digest != trailer {
        return Err(evidence_error("Git index trailer checksum does not match"));
    }
    let mut reader = Reader::new(body);
    if reader.take(4)? != b"DIRC" {
        return Err(evidence_error("Git index signature is not DIRC"));
    }
    let version = reader.u32()?;
    if !matches!(version, 2..=4) {
        return Err(evidence_error(format!(
            "Git index version {version} is unsupported"
        )));
    }
    let count = reader.u32()? as usize;
    let mut entries = Vec::with_capacity(count);
    let mut previous_path = Vec::new();
    for _ in 0..count {
        let entry_start = reader.position();
        let entry = parse_entry(&mut reader, version, format, &previous_path, entry_start)?;
        if !canonical_path(&entry.path)
            || entry.stage != 0
            || entry.flags & 0xc000 != 0
            || entry.extended_flags != 0
            || !matches!(entry.mode, 0o100644 | 0o100755 | 0o120000 | 0o160000)
        {
            return Err(evidence_error(
                "Git index contains a conflicted, flagged, or noncanonical entry",
            ));
        }
        if !previous_path.is_empty() && entry.path <= previous_path {
            return Err(evidence_error("Git index entries are not strictly ordered"));
        }
        previous_path = entry.path.clone();
        entries.push(entry);
    }
    let mut cache_tree = None;
    while !reader.is_empty() {
        let signature = reader.take(4)?;
        let size = reader.u32()? as usize;
        let payload = reader.take(size)?;
        if signature != b"TREE" || cache_tree.is_some() {
            let name = String::from_utf8_lossy(signature);
            return Err(evidence_error(format!(
                "Git index extension '{name}' is unsupported"
            )));
        }
        cache_tree = Some(parse_cache_tree(payload, oid_width)?);
    }
    Ok(RawIndex {
        version,
        entries,
        cache_tree,
    })
}

fn parse_entry(
    reader: &mut Reader<'_>,
    version: u32,
    format: git2::ObjectFormat,
    previous_path: &[u8],
    entry_start: usize,
) -> ModelResult<RawEntry> {
    let mut mode = 0;
    for field in 0..10 {
        let value = reader.u32()?;
        if field == 6 {
            mode = value;
        }
    }
    let object_id = hex(reader.take(oid_width(format))?);
    let flags = reader.u16()?;
    let extended_flags = if flags & 0x4000 != 0 {
        if version == 2 {
            return Err(evidence_error("version 2 index has extended flags"));
        }
        reader.u16()?
    } else {
        0
    };
    let path = if version == 4 {
        let remove = reader.varint()?;
        if remove > previous_path.len() {
            return Err(evidence_error("version 4 index path prefix is invalid"));
        }
        let mut path = previous_path[..previous_path.len() - remove].to_vec();
        path.extend(reader.nul_terminated()?);
        path
    } else {
        let path = reader.nul_terminated()?.to_vec();
        while !(reader.position() - entry_start).is_multiple_of(8) {
            if reader.byte()? != 0 {
                return Err(evidence_error("Git index entry padding is not zero"));
            }
        }
        path
    };
    if usize::from(flags & 0x0fff) != path.len().min(0x0fff) {
        return Err(evidence_error("Git index path length flag does not match"));
    }
    Ok(RawEntry {
        path,
        object_id,
        mode,
        stage: ((flags >> 12) & 3) as u8,
        flags,
        extended_flags,
    })
}

fn parse_cache_tree(payload: &[u8], oid_width: usize) -> ModelResult<Vec<CacheTreeNode>> {
    let mut reader = Reader::new(payload);
    let mut nodes = Vec::new();
    parse_tree_node(&mut reader, oid_width, &[], true, &mut nodes)?;
    if !reader.is_empty() {
        return Err(evidence_error("cache-tree payload has trailing bytes"));
    }
    Ok(nodes)
}

fn parse_tree_node(
    reader: &mut Reader<'_>,
    oid_width: usize,
    parent: &[u8],
    root: bool,
    nodes: &mut Vec<CacheTreeNode>,
) -> ModelResult<()> {
    let name = reader.nul_terminated()?.to_vec();
    if root && !name.is_empty()
        || !root
            && (name.is_empty() || name.contains(&b'/') || matches!(name.as_slice(), b"." | b".."))
    {
        return Err(evidence_error("cache-tree node path is noncanonical"));
    }
    let entry_count = decimal(reader.until(b' ')?)?;
    if entry_count < -1 {
        return Err(evidence_error("cache-tree entry count is invalid"));
    }
    let subtree_count = decimal(reader.until(b'\n')?)?;
    let subtree_count = usize::try_from(subtree_count)
        .map_err(|_| evidence_error("cache-tree subtree count is invalid"))?;
    if entry_count >= 0 {
        reader.take(oid_width)?;
    }
    let path = if root {
        Vec::new()
    } else if parent.is_empty() {
        name
    } else {
        [parent, b"/", name.as_slice()].concat()
    };
    nodes.push(CacheTreeNode {
        path: path.clone(),
        entry_count,
    });
    for _ in 0..subtree_count {
        parse_tree_node(reader, oid_width, &path, false, nodes)?;
    }
    Ok(())
}

fn decimal(bytes: &[u8]) -> ModelResult<i32> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| evidence_error("cache-tree count is not ASCII"))?;
    text.parse::<i32>()
        .map_err(|_| evidence_error("cache-tree count is invalid"))
}

fn canonical_path(path: &[u8]) -> bool {
    !path.is_empty()
        && !path.starts_with(b"/")
        && path
            .split(|byte| *byte == b'/')
            .all(|part| !part.is_empty() && !matches!(part, b"." | b".."))
}

fn is_ancestor(parent: &[u8], path: &[u8]) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with(b"/"))
}

fn oid_width(format: git2::ObjectFormat) -> usize {
    match format {
        git2::ObjectFormat::Sha1 => 20,
        git2::ObjectFormat::Sha256 => 32,
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn position(&self) -> usize {
        self.cursor
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }

    fn take(&mut self, count: usize) -> ModelResult<&'a [u8]> {
        let end = self
            .cursor
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| evidence_error("Git index framing is truncated"))?;
        let value = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(value)
    }

    fn byte(&mut self) -> ModelResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> ModelResult<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> ModelResult<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn nul_terminated(&mut self) -> ModelResult<&'a [u8]> {
        self.until(0)
    }

    fn until(&mut self, delimiter: u8) -> ModelResult<&'a [u8]> {
        let rest = &self.bytes[self.cursor..];
        let length = rest
            .iter()
            .position(|byte| *byte == delimiter)
            .ok_or_else(|| evidence_error("Git index delimiter is missing"))?;
        let value = self.take(length)?;
        self.byte()?;
        Ok(value)
    }

    fn varint(&mut self) -> ModelResult<usize> {
        let mut value = 0usize;
        loop {
            let byte = self.byte()?;
            value = value
                .checked_shl(7)
                .and_then(|value| value.checked_add(usize::from(byte & 0x7f)))
                .ok_or_else(|| evidence_error("version 4 index varint overflows"))?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            value = value
                .checked_add(1)
                .ok_or_else(|| evidence_error("version 4 index varint overflows"))?;
        }
    }
}

fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}
