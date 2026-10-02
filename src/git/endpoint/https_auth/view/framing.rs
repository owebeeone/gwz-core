//! Git output framing and its process-lifetime parameter quoting, not config grammar.
use super::super::*;
use super::PREPARATION_LIMIT;
use std::{os::unix::ffi::OsStrExt, path::Path};

pub(super) struct Entry {
    pub(super) name: SecretBuffer,
    pub(super) value: Option<SecretBuffer>,
}
impl PartialEq for Entry {
    fn eq(&self, rhs: &Self) -> bool {
        self.name.0 == rhs.name.0
            && self.value.as_ref().map(|v| &v.0) == rhs.value.as_ref().map(|v| &v.0)
    }
}
pub(super) enum Root {
    File(SecretBuffer),
    Command(Entry),
}

fn fields(bytes: &[u8]) -> Result<Vec<&[u8]>, AuthError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.last() != Some(&0) {
        return Err(AuthError::ConfigurationRefused);
    }
    Ok(bytes[..bytes.len() - 1].split(|byte| *byte == 0).collect())
}
fn entry(bytes: &[u8]) -> Result<Entry, AuthError> {
    let at = bytes.iter().position(|b| *b == b'\n');
    let name = &bytes[..at.unwrap_or(bytes.len())];
    if name.is_empty() {
        return Err(AuthError::ConfigurationRefused);
    }
    Ok(Entry {
        name: SecretBuffer(name.to_vec()),
        value: at.map(|i| SecretBuffer(bytes[i + 1..].to_vec())),
    })
}
pub(super) fn entries(bytes: &[u8]) -> Result<Vec<Entry>, AuthError> {
    let fields = fields(bytes)?;
    if fields.len() > 4096 {
        return Err(AuthError::ConfigurationRefused);
    }
    fields.into_iter().map(entry).collect()
}
pub(super) fn discovery(bytes: &[u8]) -> Result<Vec<Root>, AuthError> {
    let fields = fields(bytes)?;
    if fields.len() % 3 != 0 || fields.len() / 3 > 4096 {
        return Err(AuthError::ConfigurationRefused);
    }
    let mut roots = Vec::new();
    let mut known = false;
    let mut last: Option<(SecretBuffer, SecretBuffer)> = None;
    for triple in fields.chunks(3) {
        if triple[0] == b"command" && triple[1] == b"command line:" {
            known = true;
            last = None;
            roots.push(Root::Command(entry(triple[2])?));
        } else if matches!(triple[0], b"system" | b"global") || (triple[0] == b"unknown" && !known)
        {
            let path = triple[1]
                .strip_prefix(b"file:")
                .ok_or(AuthError::ConfigurationRefused)?;
            known |= triple[0] != b"unknown";
            let duplicate_run = last
                .as_ref()
                .is_some_and(|(scope, origin)| scope.0 == triple[0] && origin.0 == triple[1]);
            if !duplicate_run {
                roots.push(Root::File(absolute(b"/", path)?));
                last = Some((
                    SecretBuffer(triple[0].to_vec()),
                    SecretBuffer(triple[1].to_vec()),
                ));
            }
        } else {
            return Err(AuthError::ConfigurationRefused);
        }
    }
    Ok(roots)
}

pub(super) fn absolute(anchor: &[u8], value: &[u8]) -> Result<SecretBuffer, AuthError> {
    if !Path::new(OsStr::from_bytes(anchor)).is_absolute() || value.contains(&0) {
        return Err(AuthError::ConfigurationRefused);
    }
    if Path::new(OsStr::from_bytes(value)).is_absolute() {
        return Ok(SecretBuffer(value.to_vec()));
    }
    let size = anchor
        .len()
        .checked_add(value.len() + 1)
        .ok_or(AuthError::ConfigurationRefused)?;
    if size > PREPARATION_LIMIT {
        return Err(AuthError::ConfigurationRefused);
    }
    let mut bytes = SecretBuffer(Vec::with_capacity(size));
    bytes.0.extend_from_slice(anchor);
    if !anchor.ends_with(b"/") {
        bytes.0.push(b'/');
    }
    bytes.0.extend_from_slice(value);
    Ok(bytes)
}
pub(super) fn parameters(entries: &[Entry]) -> Result<SecretBuffer, AuthError> {
    let mut size = 0usize;
    for entry in entries {
        for bytes in std::iter::once(&entry.name.0).chain(entry.value.iter().map(|v| &v.0)) {
            if bytes.contains(&0) {
                return Err(AuthError::ConfigurationRefused);
            }
            size = size
                .checked_add(bytes.len() + 2 + bytes.iter().filter(|b| **b == b'\'').count() * 3)
                .ok_or(AuthError::ConfigurationRefused)?;
        }
        size = size.checked_add(2).ok_or(AuthError::ConfigurationRefused)?;
    }
    if size > PREPARATION_LIMIT {
        return Err(AuthError::ConfigurationRefused);
    }
    let mut output = SecretBuffer(Vec::with_capacity(size));
    for entry in entries {
        if !output.0.is_empty() {
            output.0.push(b' ');
        }
        quoted(&entry.name.0, &mut output.0);
        if let Some(value) = &entry.value {
            output.0.push(b'=');
            quoted(&value.0, &mut output.0);
        }
    }
    Ok(output)
}
fn quoted(bytes: &[u8], output: &mut Vec<u8>) {
    output.push(b'\'');
    for byte in bytes {
        if *byte == b'\'' {
            output.extend_from_slice(b"'\\''");
        } else {
            output.push(*byte);
        }
    }
    output.push(b'\'');
}
