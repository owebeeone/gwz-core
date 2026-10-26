//! The agent's keys and the wire encoding around them (step 1.7): an [`Identity`] is a listed public blob and the
//! signer for it. An RSA key signs `rsa-sha2-256` and `rsa-sha2-512` with the `ring` signer that `rustls` wraps; an
//! Ed25519 or ECDSA key (any of the three curves) signs as its own algorithm, with the SSH key library `russh` uses
//! (the candidate's dev-dependency for the controlled server), since `ring` has no ECDSA P-521. A security key is signed
//! by a software authenticator, as `key_agent.py` does for the Unix rows, and a DSA key by [`super::dsa`].
use super::dsa::{self, DsaKey};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use russh::keys::{
    PrivateKey,
    signature::{Signer as _, Verifier as _},
    ssh_encoding::{Decode as _, Encode as _},
    ssh_key::{Algorithm, EcdsaCurve, Signature},
};
use rustls::{
    SignatureScheme, SupportedCipherSuite,
    crypto::ring::{
        cipher_suite::TLS13_AES_128_GCM_SHA256, default_provider, sign::any_supported_type,
    },
    pki_types::{PrivateKeyDer, PrivatePkcs1KeyDer},
    sign::SigningKey,
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

/// The authenticator's flags byte: user presence (OpenSSH's PROTOCOL.u2f).
const USER_PRESENT: u8 = 1;

/// What signs for a key the agent lists.
#[derive(Clone)]
enum Backing {
    /// An RSA key from a traditional PEM file, and its `RSAPublicKey` for checking what it signed.
    Rsa {
        public_der: Vec<u8>,
        signer: Arc<dyn SigningKey>,
    },
    /// An Ed25519 or ECDSA key from an OpenSSH private key file.
    Openssh(Arc<PrivateKey>),
    /// A security key: the software authenticator signs for it with the plain Ed25519 or ECDSA key that backs it, and
    /// counts its signatures.
    SecurityKey {
        plain: Arc<PrivateKey>,
        counter: Arc<AtomicU32>,
    },
    /// An `ssh-dss` key the agent made.
    Dsa(Arc<DsaKey>),
    /// A key the agent lists and cannot sign for: a security key whose device is absent, a type no key backs, or an RSA
    /// key the signer refuses (under 2048 bits).
    Unbacked,
}

/// A key the agent lists: its public blob, and the signer for it.
#[derive(Clone)]
pub(crate) struct Identity {
    blob: Vec<u8>,
    backing: Backing,
}

impl Identity {
    /// The key at `path` and `path`.pub: a PEM RSA key, or an Ed25519 or ECDSA key in OpenSSH's own form.
    pub(crate) fn from_key_file(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap();
        let public = fs::read_to_string(path.with_extension("pub")).unwrap();
        let blob = STANDARD
            .decode(public.split_whitespace().nth(1).unwrap())
            .unwrap();
        if text.contains("BEGIN OPENSSH PRIVATE KEY") {
            let key = PrivateKey::from_openssh(&text).unwrap();
            assert!(
                !key.algorithm().is_rsa(),
                "an RSA key is signed from its PEM file, which ssh-keygen -m PEM writes"
            );
            assert_eq!(
                key.public_key().to_bytes().unwrap(),
                blob,
                "{} holds a different key than its .pub",
                path.display()
            );
            return Self {
                blob,
                backing: Backing::Openssh(Arc::new(key)),
            };
        }
        let body: String = text
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        assert!(
            text.contains("BEGIN RSA PRIVATE KEY"),
            "the fixture signs an RSA key from a traditional PEM file, which ssh-keygen -m PEM writes"
        );
        let der = STANDARD.decode(body).unwrap();
        let signer = match any_supported_type(&PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(der)))
        {
            Ok(signer) => signer,
            Err(_) => {
                // The signer refuses RSA under 2048 bits. The rows that load such a key assert that the agent is
                // never asked to sign with it, so the fixture lists it and refuses any signature.
                return Self {
                    blob,
                    backing: Backing::Unbacked,
                };
            }
        };
        let public_der = public_key_der(&blob);
        Self {
            blob,
            backing: Backing::Rsa { public_der, signer },
        }
    }

    /// This key as the certificate `blob` certifies it: the agent lists the certificate, and signs for it with the key,
    /// as OpenSSH's agent does for a key loaded with its `-cert.pub`.
    pub(crate) fn certified_by(&self, blob: Vec<u8>) -> Self {
        Self {
            blob,
            backing: self.backing.clone(),
        }
    }

    /// The security key (or the certificate of one) `blob`, whose point is this plain Ed25519 or ECDSA P-256 key's,
    /// with the application `ssh:`: signed as an authenticator holding this key would, with user presence asserted.
    pub(crate) fn security_key(&self, blob: Vec<u8>) -> Self {
        let Backing::Openssh(plain) = &self.backing else {
            panic!("a security key is backed by an Ed25519 or ECDSA key from an OpenSSH file");
        };
        security_key_name(plain);
        Self {
            blob,
            backing: Backing::SecurityKey {
                plain: plain.clone(),
                counter: Arc::new(AtomicU32::new(0)),
            },
        }
    }

    /// A new `ssh-dss` key: OpenSSH 10's agent no longer holds one, so the fixture makes and signs with its own.
    pub(crate) fn dsa() -> Self {
        let key = DsaKey::generate(&random);
        Self {
            blob: key.blob(),
            backing: Backing::Dsa(Arc::new(key)),
        }
    }

    /// A listed key with the public `blob`, for which nothing signs.
    pub(crate) fn unbacked(blob: Vec<u8>) -> Self {
        Self {
            blob,
            backing: Backing::Unbacked,
        }
    }

    pub(crate) fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// Whether `signature`, a signature blob, is this key's signature of `data` under `flags`.
    pub(crate) fn verify(&self, flags: u32, data: &[u8], signature: &[u8]) -> bool {
        match &self.backing {
            Backing::Rsa { public_der, .. } => verify_rsa(public_der, flags, data, signature),
            Backing::SecurityKey { plain, .. } => {
                // The algorithm, the raw signature, the flags and the counter.
                let Some((name, rest)) = take_string(signature) else {
                    return false;
                };
                let Some((raw, tail)) = take_string(rest) else {
                    return false;
                };
                let Ok(raw) = Signature::new(plain.algorithm(), raw) else {
                    return false;
                };
                name == security_key_name(plain).as_bytes()
                    && tail.len() == 5
                    && tail[0] == USER_PRESENT
                    && flags == 0
                    && plain
                        .public_key()
                        .key_data()
                        .verify(
                            &authenticator_input(
                                data,
                                u32::from_be_bytes(tail[1..].try_into().unwrap()),
                            ),
                            &raw,
                        )
                        .is_ok()
            }
            Backing::Dsa(_) => {
                matches!(split_signature(signature), Some((b"ssh-dss", raw)) if dsa::verify(&self.blob, data, raw))
            }
            Backing::Unbacked => false,
            Backing::Openssh(key) => {
                let mut input = signature;
                let Ok(signature) = Signature::decode(&mut input) else {
                    return false;
                };
                flags == 0
                    && input.is_empty()
                    && key.public_key().key_data().verify(data, &signature).is_ok()
            }
        }
    }

    pub(super) fn sign(&self, flags: u32, data: &[u8]) -> Option<Vec<u8>> {
        match &self.backing {
            Backing::Rsa { signer, .. } => {
                let (scheme, name) = algorithm(flags)?;
                let raw = signer.choose_scheme(&[scheme])?.sign(data).ok()?;
                Some(signature_blob(name.as_bytes(), &raw))
            }
            Backing::SecurityKey { plain, counter } => {
                // The flags select a hash for RSA only; the authenticator signs whatever is asked.
                let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
                let signature: Signature = plain
                    .key_data()
                    .try_sign(&authenticator_input(data, count))
                    .ok()?;
                let mut blob = Vec::new();
                string(&mut blob, security_key_name(plain).as_bytes());
                string(&mut blob, signature.as_bytes());
                blob.push(USER_PRESENT);
                blob.extend_from_slice(&count.to_be_bytes());
                Some(blob)
            }
            Backing::Dsa(key) => Some(signature_blob(b"ssh-dss", &key.sign(data, &random))),
            Backing::Unbacked => None,
            Backing::Openssh(key) => {
                // These keys sign as their own algorithm; the flags select a hash only for RSA.
                if flags != 0 {
                    return None;
                }
                let signature: Signature = key.key_data().try_sign(data).ok()?;
                let mut blob = Vec::new();
                signature.encode(&mut blob).ok()?;
                Some(blob)
            }
        }
    }
}

/// The security key type of the plain key that backs it.
fn security_key_name(plain: &PrivateKey) -> &'static str {
    match plain.algorithm() {
        Algorithm::Ed25519 => "sk-ssh-ed25519@openssh.com",
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        } => "sk-ecdsa-sha2-nistp256@openssh.com",
        other => panic!("no security key is backed by {other}"),
    }
}

/// What an authenticator signs for `data` (OpenSSH's PROTOCOL.u2f): SHA-256 of the application `ssh:`, the flags, the
/// counter and SHA-256 of the data.
fn authenticator_input(data: &[u8], counter: u32) -> Vec<u8> {
    let mut input = sha256(b"ssh:");
    input.push(USER_PRESENT);
    input.extend_from_slice(&counter.to_be_bytes());
    input.extend(sha256(data));
    input
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let SupportedCipherSuite::Tls13(suite) = TLS13_AES_128_GCM_SHA256 else {
        unreachable!("a TLS 1.3 suite");
    };
    suite.common.hash_provider.hash(data).as_ref().to_vec()
}

fn random(buffer: &mut [u8]) {
    default_provider().secure_random.fill(buffer).unwrap();
}

/// Whether `signature` is the RSA key `public_der`'s signature of `data` under `flags`.
fn verify_rsa(public_der: &[u8], flags: u32, data: &[u8], signature: &[u8]) -> bool {
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
        .any(|algorithm| algorithm.verify_signature(public_der, data, raw).is_ok())
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
