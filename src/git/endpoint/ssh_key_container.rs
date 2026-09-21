use super::agent_job;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::io;

const MAX_TEXT: usize = 1 << 20;
const PREFIX: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Label {
    Rsa,
    Dsa,
    Ec,
    Private,
    OpenSsh,
    Encrypted,
}

fn invalid() -> io::Error {
    io::ErrorKind::InvalidInput.into()
}

fn whitespace(byte: u8) -> bool {
    byte.is_ascii_whitespace()
}

fn line<'a>(bytes: &'a [u8], start: usize) -> Option<(&'a [u8], usize)> {
    if start > bytes.len() {
        return None;
    }
    let end = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset);
    let mut value = &bytes[start..end];
    if value.last() == Some(&b'\r') {
        value = &value[..value.len() - 1];
    }
    Some((value, (end < bytes.len()).then_some(end + 1).unwrap_or(end)))
}

fn begin_label(line: &[u8]) -> Option<Label> {
    let (prefix, suffix) = (b"-----BEGIN ", b"-----");
    if line.len() < prefix.len() + suffix.len()
        || !line.starts_with(prefix)
        || !line.ends_with(suffix)
    {
        return None;
    }
    match &line[prefix.len()..line.len() - suffix.len()] {
        b"RSA PRIVATE KEY" => Some(Label::Rsa),
        b"DSA PRIVATE KEY" => Some(Label::Dsa),
        b"EC PRIVATE KEY" => Some(Label::Ec),
        b"PRIVATE KEY" => Some(Label::Private),
        b"OPENSSH PRIVATE KEY" => Some(Label::OpenSsh),
        b"ENCRYPTED PRIVATE KEY" => Some(Label::Encrypted),
        _ => None,
    }
}

fn end_label(line: &[u8], label: Label) -> bool {
    let (prefix, suffix) = (b"-----END ", b"-----");
    if line.len() < prefix.len() + suffix.len()
        || !line.starts_with(prefix)
        || !line.ends_with(suffix)
    {
        return false;
    }
    let value = &line[prefix.len()..line.len() - suffix.len()];
    match label {
        Label::Rsa => value == b"RSA PRIVATE KEY",
        Label::Dsa => value == b"DSA PRIVATE KEY",
        Label::Ec => value == b"EC PRIVATE KEY",
        Label::Private => value == b"PRIVATE KEY",
        Label::OpenSsh => value == b"OPENSSH PRIVATE KEY",
        Label::Encrypted => value == b"ENCRYPTED PRIVATE KEY",
    }
}

fn decode(
    body: &[u8],
    control: &agent_job::Control,
    output: &mut [u8; PREFIX],
) -> io::Result<usize> {
    let mut quartet = [0; 4];
    let mut qlen = 0;
    let mut used = 0;
    let mut total = 0usize;
    let mut padded = false;
    for chunk in body.chunks(128) {
        control.check()?;
        for &byte in chunk {
            if whitespace(byte) {
                continue;
            }
            if padded {
                return Err(invalid());
            }
            quartet[qlen] = byte;
            qlen += 1;
            if qlen == quartet.len() {
                let mut decoded = [0; 3];
                let count = STANDARD
                    .decode_slice(quartet, &mut decoded)
                    .map_err(|_| invalid())?;
                total = total.checked_add(count).ok_or_else(invalid)?;
                let copy = count.min(PREFIX.saturating_sub(used));
                output[used..used + copy].copy_from_slice(&decoded[..copy]);
                used += copy;
                padded = quartet[2] == b'=' || quartet[3] == b'=';
                qlen = 0;
            }
        }
    }
    control.check()?;
    if qlen != 0 {
        return Err(invalid());
    }
    Ok(total)
}

fn der_header(data: &[u8], offset: usize, total: usize) -> io::Result<(u8, usize, usize)> {
    let first = *data.get(offset).ok_or_else(invalid)?;
    let marker = *data.get(offset + 1).ok_or_else(invalid)?;
    let (length, header) = if marker & 0x80 == 0 {
        (marker as usize, offset + 2)
    } else {
        let count = (marker & 0x7f) as usize;
        if count == 0 || count > 4 || offset.checked_add(2 + count).is_none() {
            return Err(invalid());
        }
        let end = offset + 2 + count;
        let mut value = 0usize;
        for byte in data.get(offset + 2..end).ok_or_else(invalid)? {
            value = value
                .checked_mul(256)
                .and_then(|v| v.checked_add(*byte as usize))
                .ok_or_else(invalid)?;
        }
        (value, end)
    };
    let end = header.checked_add(length).ok_or_else(invalid)?;
    if end > total {
        return Err(invalid());
    }
    Ok((first, end, header))
}

fn private_key(prefix: &[u8; PREFIX], total: usize) -> io::Result<()> {
    let (tag, outer, content) = der_header(prefix, 0, total)?;
    if tag != 0x30 || outer != total {
        return Err(invalid());
    }
    let (version_tag, version_end, version_data) = der_header(prefix, content, total)?;
    if version_tag != 0x02
        || version_end != version_data + 1
        || *prefix.get(version_data).ok_or_else(invalid)? > 1
    {
        return Err(invalid());
    }
    let (algorithm, algorithm_end, _) = der_header(prefix, version_end, total)?;
    if algorithm != 0x30 || algorithm_end > PREFIX {
        return Err(invalid());
    }
    let (key, key_end, _) = der_header(prefix, algorithm_end, total)?;
    if key != 0x04 || key_end != outer {
        return Err(invalid());
    }
    Ok(())
}

fn ssh_string<'a>(
    prefix: &'a [u8; PREFIX],
    offset: &mut usize,
    total: usize,
) -> io::Result<&'a [u8]> {
    let end = offset.checked_add(4).ok_or_else(invalid)?;
    let bytes = prefix.get(*offset..end).ok_or_else(invalid)?;
    let length = u32::from_be_bytes(bytes.try_into().unwrap()) as usize;
    let finish = end.checked_add(length).ok_or_else(invalid)?;
    if finish > total {
        return Err(invalid());
    }
    let value = prefix.get(end..finish).ok_or_else(invalid)?;
    *offset = finish;
    Ok(value)
}

fn openssh(prefix: &[u8; PREFIX], total: usize) -> io::Result<()> {
    let magic = b"openssh-key-v1\0";
    if total < magic.len() || &prefix[..magic.len()] != magic {
        return Err(invalid());
    }
    let mut offset = magic.len();
    let cipher = ssh_string(prefix, &mut offset, total)?;
    let kdf = ssh_string(prefix, &mut offset, total)?;
    let options = ssh_string(prefix, &mut offset, total)?;
    if cipher != b"none" || kdf != b"none" || !options.is_empty() {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn check(text: &str, control: &agent_job::Control) -> io::Result<()> {
    if text.is_empty() || text.len() > MAX_TEXT || text.as_bytes().contains(&0) {
        return Err(invalid());
    }
    control.check()?;
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() && whitespace(bytes[start]) {
        start += 1;
    }
    let (begin, mut cursor) = line(bytes, start).ok_or_else(invalid)?;
    let label = begin_label(begin).ok_or_else(invalid)?;
    if label == Label::Encrypted {
        return Err(invalid());
    }
    let body_start = cursor;
    let body_end;
    loop {
        control.check()?;
        let (current, next) = line(bytes, cursor).ok_or_else(invalid)?;
        if end_label(current, label) {
            body_end = cursor;
            cursor = next;
            break;
        }
        if current.starts_with(b"-----END ") || current.starts_with(b"-----BEGIN ") {
            return Err(invalid());
        }
        if next == cursor {
            return Err(invalid());
        }
        cursor = next;
    }
    for chunk in bytes[cursor..].chunks(128) {
        control.check()?;
        if chunk.iter().any(|byte| !whitespace(*byte)) {
            return Err(invalid());
        }
    }
    let mut prefix = [0; PREFIX];
    let total = decode(&bytes[body_start..body_end], control, &mut prefix)?;
    if total == 0 {
        return Err(invalid());
    }
    match label {
        Label::Rsa | Label::Dsa | Label::Ec => Ok(()),
        Label::Private => private_key(&prefix, total),
        Label::OpenSsh => openssh(&prefix, total),
        Label::Encrypted => Err(invalid()),
    }
}
