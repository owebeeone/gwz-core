//! F12: an ECDSA key whose private scalar OpenSSH writes shorter than its curve's width loads, on every curve.
use super::parse;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use russh::keys::{
    signature::{Signer as _, Verifier as _},
    ssh_encoding::{self, Encode as _},
    ssh_key::{self, Algorithm, EcdsaCurve, PrivateKey, Signature},
};

/// Each curve's key type and curve names, the width of its scalar, and its generator point (uncompressed, as
/// `openssl ecparam -param_enc explicit -text` prints it): the public key of the scalar 1.
const CURVES: [(EcdsaCurve, &str, &str, usize, &str); 3] = [
    (
        EcdsaCurve::NistP256,
        "ecdsa-sha2-nistp256",
        "nistp256",
        32,
        "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
    ),
    (
        EcdsaCurve::NistP384,
        "ecdsa-sha2-nistp384",
        "nistp384",
        48,
        "04aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab73617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
    ),
    (
        EcdsaCurve::NistP521,
        "ecdsa-sha2-nistp521",
        "nistp521",
        66,
        "0400c6858e06b70404e9cd9e3ecb662395b4429c648139053fb521f828af606b4d3dbaa14b5e77efe75928fe1dc127a2ffa8de3348b3c1856a429bf97e7e31c2e5bd66011839296a789a3bc0045c8a5fb42c7d1bd998f54449579b446817afbd17273e662c97ee72995ef42640c550b9013fad0761353c7086a272c24088be94769fd16650",
    ),
];

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    bytes.encode(out).unwrap();
}

/// An unencrypted OpenSSH private key file (OpenSSH's PROTOCOL.key) of the ECDSA key `kind` on `curve` with the public
/// `point` and the private `scalar`, written as the given bytes, and its public blob.
fn openssh_file(kind: &str, curve: &str, point: &[u8], scalar: &[u8]) -> (String, Vec<u8>) {
    let mut public = Vec::new();
    for field in [kind.as_bytes(), curve.as_bytes(), point] {
        string(&mut public, field);
    }
    let mut section = Vec::new();
    section.extend_from_slice(&[0x5a; 8]);
    for field in [kind.as_bytes(), curve.as_bytes(), point, scalar, b"f12"] {
        string(&mut section, field);
    }
    let mut pad = 1;
    while section.len() % 8 != 0 {
        section.push(pad);
        pad += 1;
    }
    let mut file = b"openssh-key-v1\0".to_vec();
    for field in [&b"none"[..], b"none", b""] {
        string(&mut file, field);
    }
    file.extend_from_slice(&1u32.to_be_bytes());
    string(&mut file, &public);
    string(&mut file, &section);
    let text = format!(
        "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
        STANDARD.encode(file)
    );
    (text, public)
}

/// `key` is the ECDSA key on `curve` whose public blob is `public`, and its signatures verify against that point.
fn assert_signs(key: &PrivateKey, curve: EcdsaCurve, public: &[u8]) {
    assert_eq!(key.algorithm(), Algorithm::Ecdsa { curve });
    assert_eq!(key.public_key().to_bytes().unwrap(), public, "{curve:?}");
    let signature: Signature = key.key_data().try_sign(b"data").unwrap();
    assert!(
        key.public_key()
            .key_data()
            .verify(b"data", &signature)
            .is_ok(),
        "{curve:?}: the loaded scalar signs for the file's point"
    );
}

#[test]
fn an_ecdsa_scalar_written_shorter_than_its_curve_loads_on_every_curve() {
    // The scalar 1, whose public key is the generator: OpenSSH writes it as the one-byte mpint 01, the shortest form a
    // scalar takes. ssh-keygen writes a P-256 scalar under 2^248 (about one key in 256) as a 31-byte mpint, which the
    // library refuses (F12, receipt root-3merge-20261031-c).
    for (curve, kind, name, _, generator) in CURVES {
        let (text, public) = openssh_file(kind, name, &hex(generator), &[1]);
        let key = parse(&text).unwrap_or_else(|e| panic!("{curve:?}: {e:?}"));
        assert_signs(&key, curve, &public);
    }
}

#[test]
fn an_ecdsa_scalar_written_at_its_curve_width_loads_as_before() {
    for (curve, kind, name, width, generator) in CURVES {
        let mut scalar = vec![0; width];
        scalar[width - 1] = 1;
        let (text, public) = openssh_file(kind, name, &hex(generator), &scalar);
        assert_signs(&parse(&text).unwrap(), curve, &public);
        // The library reads this form itself.
        assert_signs(&PrivateKey::from_openssh(&text).unwrap(), curve, &public);
    }
}

#[test]
fn the_library_alone_still_refuses_a_short_scalar() {
    // A tripwire for a dependency bump: when this fails, the library reads a short scalar itself, and `parse` can be
    // `PrivateKey::from_openssh` again.
    let (curve, kind, name, _, generator) = CURVES[0];
    let (text, _) = openssh_file(kind, name, &hex(generator), &[1]);
    assert!(
        matches!(
            PrivateKey::from_openssh(&text),
            Err(ssh_key::Error::Encoding(ssh_encoding::Error::Length))
        ),
        "{curve:?}: ssh-key now reads a short scalar; drop the widening in openssh_key::parse"
    );
}
