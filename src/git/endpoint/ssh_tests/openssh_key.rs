//! An OpenSSH private key file loaded into the SSH key library `russh` uses (its `ssh-key`), for the fixtures that sign
//! with a key themselves (the Windows pipe agent, `agent_fixture_pipe::Identity`).
//!
//! OpenSSH writes an ECDSA key's private scalar as an mpint, which drops the scalar's leading zero bytes, so a key whose
//! scalar has one is written shorter than its curve's width: about one P-256 key in 256 that `ssh-keygen` makes has a
//! 31-byte scalar. ssh-key 0.7.0-rc.11 refuses a scalar under 32 bytes (`Encoding(Length)`), which turned the Windows
//! row `key_types::ecdsa_keys_of_each_curve_authenticate` red on a valid key (findings F12, receipt
//! root-3merge-20261031-c). [`parse`] writes each ECDSA scalar at its curve's full width, which the library reads as
//! the same number, before the library parses the file; every other file reaches the library unchanged.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use russh::keys::{
    PrivateKey,
    ssh_encoding::{Decode as _, Encode as _, Reader as _},
    ssh_key,
};

const MAGIC: &[u8] = b"openssh-key-v1\0";

/// The key in `text`, an OpenSSH private key file.
pub(crate) fn parse(text: &str) -> ssh_key::Result<PrivateKey> {
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let Ok(file) = STANDARD.decode(body.trim()) else {
        return PrivateKey::from_openssh(text);
    };
    match widened(&file) {
        Some(file) => PrivateKey::from_bytes(&file),
        None => PrivateKey::from_openssh(text),
    }
}

/// The scalar width of the ECDSA key type `kind`.
fn width(kind: &[u8]) -> Option<usize> {
    match kind {
        b"ecdsa-sha2-nistp256" => Some(32),
        b"ecdsa-sha2-nistp384" => Some(48),
        b"ecdsa-sha2-nistp521" => Some(66),
        _ => None,
    }
}

/// `file` (OpenSSH's PROTOCOL.key) with its ECDSA scalar written at its curve's width, when it is an unencrypted ECDSA
/// key whose scalar is shorter than that; otherwise `None`.
fn widened(file: &[u8]) -> Option<Vec<u8>> {
    let mut reader = file.strip_prefix(MAGIC)?;
    let cipher = Vec::<u8>::decode(&mut reader).ok()?;
    let kdf = Vec::<u8>::decode(&mut reader).ok()?;
    let options = Vec::<u8>::decode(&mut reader).ok()?;
    let count = u32::decode(&mut reader).ok()?;
    let public = Vec::<u8>::decode(&mut reader).ok()?;
    let private = Vec::<u8>::decode(&mut reader).ok()?;
    if cipher != b"none" || count != 1 || !reader.is_finished() {
        return None;
    }
    let mut section = private.as_slice();
    let checks = <[u8; 8]>::decode(&mut section).ok()?;
    let kind = Vec::<u8>::decode(&mut section).ok()?;
    let width = width(&kind)?;
    let curve = Vec::<u8>::decode(&mut section).ok()?;
    let point = Vec::<u8>::decode(&mut section).ok()?;
    let scalar = Vec::<u8>::decode(&mut section).ok()?;
    let comment = Vec::<u8>::decode(&mut section).ok()?;
    // The rest is the padding; a file whose padding is wrong goes to the library as it is, which refuses it.
    let padded = section.iter().zip(1..).all(|(byte, at)| *byte == at);
    if scalar.len() >= width || section.len() >= 8 || !padded {
        return None;
    }
    let mut wide = vec![0; width - scalar.len()];
    wide.extend_from_slice(&scalar);
    let mut rewritten = checks.to_vec();
    for field in [&kind, &curve, &point, &wide, &comment] {
        field.as_slice().encode(&mut rewritten).ok()?;
    }
    // PROTOCOL.key's padding: 1, 2, 3, ... up to the cipher's block size, 8 for `none`.
    let mut pad = 1;
    while rewritten.len() % 8 != 0 {
        rewritten.push(pad);
        pad += 1;
    }
    let mut out = MAGIC.to_vec();
    for field in [&cipher, &kdf, &options] {
        field.as_slice().encode(&mut out).ok()?;
    }
    count.encode(&mut out).ok()?;
    for field in [&public, &rewritten] {
        field.as_slice().encode(&mut out).ok()?;
    }
    Some(out)
}

mod tests;
