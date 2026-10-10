//! The agent's keys and the wire encoding around them (step 1.7): an [`Identity`] is a listed public blob and the
//! signer for it, and signs `rsa-sha2-256` and `rsa-sha2-512` with the `ring` signer that `rustls` wraps.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rustls::{
    SignatureScheme,
    crypto::ring::{default_provider, sign::any_supported_type},
    pki_types::{PrivateKeyDer, PrivatePkcs1KeyDer},
    sign::SigningKey,
};
use std::{fs, path::Path, sync::Arc};

/// A key the agent lists: its public blob, and the signer for it.
#[derive(Clone)]
pub(crate) struct Identity {
    blob: Vec<u8>,
    public_der: Vec<u8>,
    signer: Arc<dyn SigningKey>,
}

impl Identity {
    /// The key at `path` (a PEM RSA key) and `path`.pub.
    pub(crate) fn from_key_file(path: &Path) -> Self {
        let pem = fs::read_to_string(path).unwrap();
        let body: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        assert!(
            pem.contains("BEGIN RSA PRIVATE KEY"),
            "the fixture signs with a traditional PEM RSA key, which ssh-keygen -m PEM writes"
        );
        let der = STANDARD.decode(body).unwrap();
        let signer =
            any_supported_type(&PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(der))).unwrap();
        let public = fs::read_to_string(path.with_extension("pub")).unwrap();
        let blob = STANDARD
            .decode(public.split_whitespace().nth(1).unwrap())
            .unwrap();
        let public_der = public_key_der(&blob);
        Self {
            blob,
            public_der,
            signer,
        }
    }

    pub(crate) fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// Whether `signature`, a signature blob, is this key's signature of `data` under `flags`.
    pub(crate) fn verify(&self, flags: u32, data: &[u8], signature: &[u8]) -> bool {
        let Some((scheme, name)) = algorithm(flags) else {
            return false;
        };
        let Some((found, raw)) = split_signature(signature) else {
            return false;
        };
        if found != name.as_bytes() {
            return false;
        }
        let provider = default_provider();
        provider
            .signature_verification_algorithms
            .mapping
            .iter()
            .filter(|(candidate, _)| *candidate == scheme)
            .flat_map(|(_, algorithms)| algorithms.iter())
            .any(|algorithm| {
                algorithm
                    .verify_signature(&self.public_der, data, raw)
                    .is_ok()
            })
    }

    pub(super) fn sign(&self, flags: u32, data: &[u8]) -> Option<Vec<u8>> {
        let (scheme, name) = algorithm(flags)?;
        let raw = self.signer.choose_scheme(&[scheme])?.sign(data).ok()?;
        Some(signature_blob(name.as_bytes(), &raw))
    }
}

/// The scheme and SSH algorithm name a sign request's flags ask for: 2 is `rsa-sha2-256`, 4 is `rsa-sha2-512`.
pub(super) fn algorithm(flags: u32) -> Option<(SignatureScheme, &'static str)> {
    match flags {
        2 => Some((SignatureScheme::RSA_PKCS1_SHA256, "rsa-sha2-256")),
        4 => Some((SignatureScheme::RSA_PKCS1_SHA512, "rsa-sha2-512")),
        _ => None,
    }
}

pub(super) fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

pub(super) fn signature_blob(name: &[u8], raw: &[u8]) -> Vec<u8> {
    let mut blob = Vec::new();
    string(&mut blob, name);
    string(&mut blob, raw);
    blob
}

/// The two strings of a signature blob, when it is exactly those.
pub(super) fn split_signature(blob: &[u8]) -> Option<(&[u8], &[u8])> {
    let (name, rest) = take_string(blob)?;
    let (raw, rest) = take_string(rest)?;
    rest.is_empty().then_some((name, raw))
}

pub(super) fn take_string(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let length = u32::from_be_bytes(input.get(..4)?.try_into().ok()?) as usize;
    let end = 4usize.checked_add(length)?;
    Some((input.get(4..end)?, input.get(end..)?))
}

/// The PKCS#1 `RSAPublicKey` of an `ssh-rsa` public blob: a SEQUENCE of the modulus and the exponent.
fn public_key_der(blob: &[u8]) -> Vec<u8> {
    let (kind, rest) = take_string(blob).unwrap();
    assert_eq!(kind, b"ssh-rsa");
    let (exponent, rest) = take_string(rest).unwrap();
    let (modulus, _) = take_string(rest).unwrap();
    let mut body = Vec::new();
    for integer in [modulus, exponent] {
        body.push(0x02);
        der_length(&mut body, integer.len());
        body.extend_from_slice(integer);
    }
    let mut der = vec![0x30];
    der_length(&mut der, body.len());
    der.extend(body);
    der
}

fn der_length(out: &mut Vec<u8>, length: usize) {
    if length < 0x80 {
        out.push(length as u8);
    } else {
        let bytes = length.to_be_bytes();
        let used: Vec<u8> = bytes
            .iter()
            .copied()
            .skip_while(|byte| *byte == 0)
            .collect();
        out.push(0x80 | used.len() as u8);
        out.extend(used);
    }
}
