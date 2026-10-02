use super::agent_job;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::io;

const MAX_TEXT: usize = 1 << 20;
/// Decoded bytes kept for the structure checks: room before the key for a
/// PKCS#8 EC key's explicit curve parameters, which LibreSSL's `ssh-keygen`
/// writes (TR2.8).
const PREFIX: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Label {
    Rsa,
    Dsa,
    Ec,
    Private,
    OpenSsh,
}

fn invalid() -> io::Error {
    io::ErrorKind::InvalidInput.into()
}

fn whitespace(byte: u8) -> bool {
    byte.is_ascii_whitespace()
}

fn checked_scan<F>(
    bytes: &[u8],
    control: &agent_job::Control,
    mut stop: F,
) -> io::Result<Option<usize>>
where
    F: FnMut(u8) -> bool,
{
    for (base, chunk) in bytes.chunks(128).enumerate() {
        control.check()?;
        for (offset, byte) in chunk.iter().copied().enumerate() {
            if stop(byte) {
                return Ok(Some(base * 128 + offset));
            }
        }
    }
    Ok(None)
}

fn line<'a>(
    bytes: &'a [u8],
    start: usize,
    control: &agent_job::Control,
) -> io::Result<Option<(&'a [u8], usize)>> {
    if start > bytes.len() {
        return Ok(None);
    }
    // libssh2's in-memory PEM/OpenSSH reader treats both CR and LF as
    // boundaries. Admission must see every block native parsing can select.
    let end = checked_scan(&bytes[start..], control, |byte| {
        matches!(byte, b'\r' | b'\n')
    })?
    .map_or(bytes.len(), |offset| start + offset);
    let next = if bytes.get(end) == Some(&b'\r') && bytes.get(end + 1) == Some(&b'\n') {
        end + 2
    } else if end < bytes.len() {
        end + 1
    } else {
        end
    };
    Ok(Some((&bytes[start..end], next)))
}

/// The label of an armor line, `-----BEGIN <label>-----` for `edge`
/// `-----BEGIN ` or `-----END <label>-----` for `-----END `.
fn armor<'a>(line: &'a [u8], edge: &[u8]) -> Option<&'a [u8]> {
    line.strip_prefix(edge)?
        .strip_suffix(b"-----")
        .filter(|label| !label.is_empty())
}

/// The private key a block's label names, if any. Any other label ending in
/// `PRIVATE KEY`, `ENCRYPTED PRIVATE KEY` among them, is refused: libssh2
/// reads the file without a passphrase.
fn key_label(label: &[u8]) -> io::Result<Option<Label>> {
    Ok(Some(match label {
        b"RSA PRIVATE KEY" => Label::Rsa,
        b"DSA PRIVATE KEY" => Label::Dsa,
        b"EC PRIVATE KEY" => Label::Ec,
        b"PRIVATE KEY" => Label::Private,
        b"OPENSSH PRIVATE KEY" => Label::OpenSsh,
        _ if label.ends_with(b"PRIVATE KEY") => return Err(invalid()),
        _ => return Ok(None),
    }))
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
    // PKCS#8's optional attributes, and a v2 public key, may follow.
    let (key, _, _) = der_header(prefix, algorithm_end, total)?;
    if key != 0x04 {
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

/// Checks that `text` holds exactly one unencrypted private key, in a PEM or
/// OpenSSH armor, before libssh2, which gets no passphrase, reads it. Text
/// and other blocks around the key's, such as `openssl ecparam -genkey`'s
/// `EC PARAMETERS`, are skipped, as OpenSSL's PEM reader and libssh2's
/// OpenSSH reader skip them (TR2.8). No block may carry a PEM header, which
/// OpenSSL decrypts whatever the block's label.
pub(crate) fn check(text: &str, control: &agent_job::Control) -> io::Result<()> {
    if text.is_empty() || text.len() > MAX_TEXT {
        return Err(invalid());
    }
    control.check()?;
    let bytes = text.as_bytes();
    if checked_scan(bytes, control, |byte| byte == 0)?.is_some() {
        return Err(invalid());
    }
    let mut key = None;
    let mut block: Option<(&[u8], usize)> = None;
    let mut cursor = 0;
    while let Some((current, next)) = line(bytes, cursor, control)? {
        control.check()?;
        match block {
            None => {
                if let Some(label) = armor(current, b"-----BEGIN ") {
                    key_label(label)?;
                    block = Some((label, next));
                }
            }
            Some((label, start)) if armor(current, b"-----END ") == Some(label) => {
                if let Some(kind) = key_label(label)?
                    && key.replace((kind, start, cursor)).is_some()
                {
                    return Err(invalid());
                }
                block = None;
            }
            Some(_) => {
                let base64 = |byte: u8| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte);
                if checked_scan(current, control, |byte| !base64(byte) && !whitespace(byte))?
                    .is_some()
                {
                    return Err(invalid());
                }
            }
        }
        if next == cursor {
            break;
        }
        cursor = next;
    }
    let (Some((label, start, end)), None) = (key, block) else {
        return Err(invalid());
    };
    let mut prefix = [0; PREFIX];
    let total = decode(&bytes[start..end], control, &mut prefix)?;
    if total == 0 {
        return Err(invalid());
    }
    match label {
        Label::Rsa | Label::Dsa | Label::Ec => Ok(()),
        Label::Private => private_key(&prefix, total),
        Label::OpenSsh => openssh(&prefix, total),
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) fn scan_for_test<F>(
            bytes: &[u8],
            control: &agent_job::Control,
            stop: F,
        ) -> io::Result<Option<usize>>
        where
            F: FnMut(u8) -> bool,
        {
            checked_scan(bytes, control, stop)
        }
    }
}
