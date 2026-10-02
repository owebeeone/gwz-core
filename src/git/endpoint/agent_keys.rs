//! The agent key types and signature algorithms the transport signs with
//! (TR2.8, dev-docs/GwzTransportSshKeyTypes.md): every one 1.0.17's libssh2
//! 1.11.1 authenticates with through an agent, and the security keys and
//! OpenSSH certificates of those types. A key of any other type is never
//! offered. Each agent signature is checked against the requested algorithm
//! and the key before libssh2 sees it, and none is logged.
use std::{io, slice};

const CERTIFICATE: &str = "-cert-v01@openssh.com";
/// RSA's algorithms, in libssh2's order of preference.
const RSA: &[&str] = &["rsa-sha2-512", "rsa-sha2-256", "ssh-rsa"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Ed25519,
    /// ECDSA over the NIST curve whose field has this many bytes.
    Ecdsa(usize),
    Rsa,
    Dsa,
    SkEd25519,
    SkEcdsa,
}

const TYPES: &[(&str, Family)] = &[
    ("ssh-ed25519", Family::Ed25519),
    ("ecdsa-sha2-nistp256", Family::Ecdsa(32)),
    ("ecdsa-sha2-nistp384", Family::Ecdsa(48)),
    ("ecdsa-sha2-nistp521", Family::Ecdsa(66)),
    ("ssh-rsa", Family::Rsa),
    ("ssh-dss", Family::Dsa),
    ("sk-ssh-ed25519@openssh.com", Family::SkEd25519),
    ("sk-ecdsa-sha2-nistp256@openssh.com", Family::SkEcdsa),
];

/// What the agent's signature gives libssh2.
pub(crate) enum Signed {
    /// The bytes libssh2 frames and sends.
    Signature(Vec<u8>),
    /// The agent answered a `rsa-sha2-*` request with an `ssh-rsa`
    /// signature. libssh2 then offers the key once more as `ssh-rsa`, as
    /// 1.0.17's libssh2 does (the list's §3, case 3).
    Downgraded,
}

/// The type of an agent key the transport signs with: a listed type, or an
/// OpenSSH certificate of one other than DSA, whose certificate neither
/// libssh2 nor the transport signs with.
pub(crate) struct KeyType {
    name: &'static str,
    family: Family,
    certificate: bool,
}

impl KeyType {
    /// The type `blob` names, or `None` for a key the transport skips.
    pub(crate) fn of(blob: &[u8]) -> Option<Self> {
        let named = field(&mut &blob[..]).ok()?;
        let certified = named.strip_suffix(CERTIFICATE.as_bytes());
        TYPES.iter().find_map(|&(name, family)| {
            let certificate = family != Family::Dsa && certified == Some(stem(name).as_bytes());
            (certificate || named == name.as_bytes()).then_some(Self {
                name,
                family,
                certificate,
            })
        })
    }

    /// The signature algorithm libssh2's `method` asks the agent for: one of
    /// the type's, under a certificate's suffix for a certificate.
    pub(crate) fn algorithm(&self, method: &str) -> io::Result<&'static str> {
        let (wanted, shown): (_, fn(&str) -> &str) = if self.certificate {
            (method.strip_suffix(CERTIFICATE).ok_or_else(invalid)?, stem)
        } else {
            (method, |name| name)
        };
        let algorithms = if self.family == Family::Rsa {
            RSA
        } else {
            slice::from_ref(&self.name)
        };
        algorithms
            .iter()
            .copied()
            .find(|name| shown(name) == wanted)
            .ok_or_else(invalid)
    }

    /// Checks the agent's `reply`, a signature blob, against `algorithm`,
    /// libssh2's `method` and the key `blob`, and returns what libssh2 sends:
    /// the signature, or for a security key the signature string, the flags
    /// and the counter, which libssh2 sends unframed.
    pub(crate) fn signature(
        &self,
        blob: &[u8],
        method: &str,
        algorithm: &str,
        reply: &[u8],
    ) -> io::Result<Signed> {
        let mut input = reply;
        let named = field(&mut input)?;
        if named == b"ssh-rsa" && algorithm.starts_with("rsa-sha2-") {
            let downgraded = !field(&mut input)?.is_empty() && input.is_empty();
            return if downgraded {
                Ok(Signed::Downgraded)
            } else {
                Err(invalid())
            };
        }
        if named != algorithm.as_bytes() && named != method.as_bytes() {
            return Err(invalid());
        }
        let modulus = self.key(blob)?;
        let body = input;
        let raw = field(&mut input)?;
        let sk = matches!(self.family, Family::SkEd25519 | Family::SkEcdsa);
        let valid = match self.family {
            Family::Ed25519 | Family::SkEd25519 => raw.len() == 64,
            Family::Ecdsa(size) => ecdsa(raw, size),
            Family::SkEcdsa => ecdsa(raw, 32),
            Family::Rsa => raw.len() == modulus,
            Family::Dsa => raw.len() == 40,
        };
        if !valid || input.len() != if sk { 5 } else { 0 } {
            return Err(invalid());
        }
        Ok(Signed::Signature(if sk { body } else { raw }.to_vec()))
    }

    /// Checks the key blob's shape, and returns an RSA key's modulus length,
    /// or zero for any other type.
    fn key(&self, blob: &[u8]) -> io::Result<usize> {
        let mut input = blob;
        field(&mut input)?;
        if self.certificate {
            field(&mut input)?; // nonce
        }
        let mut modulus = 0;
        match self.family {
            Family::Ed25519 | Family::SkEd25519 => {
                if field(&mut input)?.len() != 32 {
                    return Err(invalid());
                }
            }
            Family::Ecdsa(size) => point(&mut input, size)?,
            Family::SkEcdsa => point(&mut input, 32)?,
            Family::Rsa => {
                let exponent = field(&mut input)?;
                let n = field(&mut input)?;
                modulus = n.strip_prefix(&[0]).unwrap_or(n).len();
                if exponent.is_empty() || modulus == 0 {
                    return Err(invalid());
                }
            }
            Family::Dsa => {
                for _ in 0..4 {
                    field(&mut input)?;
                }
            }
        }
        if matches!(self.family, Family::SkEd25519 | Family::SkEcdsa) {
            field(&mut input)?; // application
        }
        if self.certificate {
            // Serial and type; key ID and principals; validity; critical
            // options, extensions, reserved, signature key and signature.
            take(&mut input, 12)?;
            for _ in 0..2 {
                field(&mut input)?;
            }
            take(&mut input, 16)?;
            for _ in 0..5 {
                field(&mut input)?;
            }
        }
        if input.is_empty() {
            Ok(modulus)
        } else {
            Err(invalid())
        }
    }
}

/// The agent's flags for `algorithm`: RFC 8332's SHA-2 requests for RSA.
pub(crate) fn flags(algorithm: &str) -> u32 {
    match algorithm {
        "rsa-sha2-256" => 2,
        "rsa-sha2-512" => 4,
        _ => 0,
    }
}

fn stem(name: &str) -> &str {
    name.strip_suffix("@openssh.com").unwrap_or(name)
}

/// An ECDSA key's curve name and uncompressed point.
fn point(input: &mut &[u8], size: usize) -> io::Result<()> {
    let curve: &[u8] = match size {
        32 => b"nistp256",
        48 => b"nistp384",
        _ => b"nistp521",
    };
    if field(input)? != curve {
        return Err(invalid());
    }
    let q = field(input)?;
    if q.len() != 1 + 2 * size || q[0] != 4 {
        return Err(invalid());
    }
    Ok(())
}

/// An ECDSA signature: two positive mpints in their shortest form, each no
/// longer than the field.
fn ecdsa(mut raw: &[u8], size: usize) -> bool {
    for _ in 0..2 {
        let positive = field(&mut raw).is_ok_and(|value| match value {
            [] => false,
            [0, rest @ ..] => {
                rest.first().is_some_and(|byte| byte & 0x80 != 0) && rest.len() <= size
            }
            [first, ..] => first & 0x80 == 0 && value.len() <= size,
        });
        if !positive {
            return false;
        }
    }
    raw.is_empty()
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> io::Result<&'a [u8]> {
    if length > input.len() {
        return Err(invalid());
    }
    let (value, rest) = input.split_at(length);
    *input = rest;
    Ok(value)
}

/// One SSH string.
pub(crate) fn field<'a>(input: &mut &'a [u8]) -> io::Result<&'a [u8]> {
    let length = u32::from_be_bytes(take(input, 4)?.try_into().map_err(|_| invalid())?) as usize;
    take(input, length)
}

fn invalid() -> io::Error {
    io::ErrorKind::InvalidData.into()
}
