//! A DSA key for the pipe agent (the twin of `key_agent.py`'s `dsa` entry): `ssh-dss` with the fixed 1024-bit domain
//! parameters that file holds, signing and verifying with SHA-1 over a small Montgomery big-integer routine. Standard
//! library only: neither `ring` nor `russh`'s SSH key crate signs DSA, and a test must not add a dependency for a
//! legacy key type that OpenSSH 10's own agent no longer holds. Test code, and not constant-time.
use std::cmp::Ordering;

const P: &str = "9cd3b7ba4d5059eeb05a8ef65a7c83181a1e323d3c1986f17752c783d7754cd691a3ce19b64f53d7eb6a225961341b4ba9921189539403072446f4cafc36cd4b5dcc260426cb278bf4d81def0deb20e4c648068aaafe362d5d2a890e47d4cf3023191f75570a3df6437c67063f74c6391d9b6af735c1cad1cc55e4865eaf3ee9";
const Q: &str = "cc47e2885c7005f2cae1de266d6dc4efad520447";
const G: &str = "955e08a4686ef18adce5f3444c001c4a82a46c252e4996a4a165b4148bffacc53aebc47a490c92fa487199dccc23c0196880bf2696525b03618d4bb6136434bb7754fe7a542f2dea049e727c2ca510b7eeb24a396befe6d1778b489be01874641c454bbd03cbdea67c50dc01e953ff6331e438cc8d2e6b4e33d18b4420daf440";

/// Fills a buffer with random bytes.
pub(super) type Random<'a> = &'a dyn Fn(&mut [u8]);

/// Little-endian 64-bit limbs of the big-endian `bytes`, padded or kept to `limbs`.
fn limbs(bytes: &[u8], limbs: usize) -> Vec<u64> {
    let mut out = vec![0_u64; limbs];
    for (i, byte) in bytes.iter().rev().enumerate() {
        out[i / 8] |= u64::from(*byte) << (8 * (i % 8));
    }
    out
}

fn from_hex(text: &str, limbs_wanted: usize) -> Vec<u64> {
    let bytes: Vec<u8> = (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    limbs(&bytes, limbs_wanted)
}

/// The big-endian bytes of `value`, `length` of them.
fn to_be(value: &[u64], length: usize) -> Vec<u8> {
    (0..length)
        .rev()
        .map(|i| (value[i / 8] >> (8 * (i % 8))) as u8)
        .collect()
}

fn compare(a: &[u64], b: &[u64]) -> Ordering {
    a.iter().rev().cmp(b.iter().rev())
}

fn is_zero(a: &[u64]) -> bool {
    a.iter().all(|limb| *limb == 0)
}

/// `a - b` in place, for `a >= b` (or with the carry out that a caller holds).
fn subtract(a: &mut [u64], b: &[u64]) {
    let mut borrow = false;
    for (x, y) in a.iter_mut().zip(b) {
        let (d, b1) = x.overflowing_sub(*y);
        let (d, b2) = d.overflowing_sub(u64::from(borrow));
        *x = d;
        borrow = b1 || b2;
    }
}

/// `a + b` in place; returns the carry out.
fn add(a: &mut [u64], b: &[u64]) -> bool {
    let mut carry = false;
    for (x, y) in a.iter_mut().zip(b) {
        let (s, c1) = x.overflowing_add(*y);
        let (s, c2) = s.overflowing_add(u64::from(carry));
        *x = s;
        carry = c1 || c2;
    }
    carry
}

/// Arithmetic modulo an odd `modulus`, in Montgomery form (CIOS).
struct Modulus {
    m: Vec<u64>,
    /// `-m^-1 mod 2^64`.
    n0: u64,
    /// `2^(128 n) mod m`.
    r2: Vec<u64>,
}

impl Modulus {
    fn new(m: Vec<u64>) -> Self {
        let n = m.len();
        let mut inverse = m[0];
        for _ in 0..6 {
            inverse = inverse.wrapping_mul(2_u64.wrapping_sub(m[0].wrapping_mul(inverse)));
        }
        let mut r2 = vec![0_u64; n];
        r2[0] = 1;
        for _ in 0..128 * n {
            let carry = add_self(&mut r2);
            if carry || compare(&r2, &m) != Ordering::Less {
                subtract(&mut r2, &m);
            }
        }
        Self {
            m,
            n0: inverse.wrapping_neg(),
            r2,
        }
    }

    fn mont_mul(&self, a: &[u64], b: &[u64]) -> Vec<u64> {
        let n = self.m.len();
        let mut t = vec![0_u64; n + 2];
        for &bi in b.iter().take(n) {
            let mut carry = 0_u128;
            for j in 0..n {
                let s = u128::from(t[j]) + u128::from(a[j]) * u128::from(bi) + carry;
                t[j] = s as u64;
                carry = s >> 64;
            }
            let s = u128::from(t[n]) + carry;
            t[n] = s as u64;
            t[n + 1] = (s >> 64) as u64;
            let q = t[0].wrapping_mul(self.n0);
            let s = u128::from(t[0]) + u128::from(q) * u128::from(self.m[0]);
            let mut carry = s >> 64;
            for j in 1..n {
                let s = u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + carry;
                t[j - 1] = s as u64;
                carry = s >> 64;
            }
            let s = u128::from(t[n]) + carry;
            t[n - 1] = s as u64;
            t[n] = t[n + 1] + (s >> 64) as u64;
        }
        let top = t[n];
        t.truncate(n);
        if top != 0 || compare(&t, &self.m) != Ordering::Less {
            subtract(&mut t, &self.m);
        }
        t
    }

    /// `a * b mod m`, for `a` and `b` below `m`.
    fn mul(&self, a: &[u64], b: &[u64]) -> Vec<u64> {
        self.mont_mul(&self.mont_mul(a, b), &self.r2)
    }

    /// `base ^ exponent mod m`, for `base` below `m`.
    fn pow(&self, base: &[u64], exponent: &[u64]) -> Vec<u64> {
        let n = self.m.len();
        let mut one = vec![0_u64; n];
        one[0] = 1;
        let base = self.mont_mul(base, &self.r2);
        let mut accumulator = self.mont_mul(&one, &self.r2);
        for limb in exponent.iter().rev() {
            for bit in (0..64).rev() {
                accumulator = self.mont_mul(&accumulator, &accumulator);
                if (limb >> bit) & 1 == 1 {
                    accumulator = self.mont_mul(&accumulator, &base);
                }
            }
        }
        self.mont_mul(&accumulator, &one)
    }

    /// `a + b mod m`, for `a` and `b` below `m`.
    fn add(&self, a: &[u64], b: &[u64]) -> Vec<u64> {
        let mut sum = a.to_vec();
        let carry = add(&mut sum, b);
        if carry || compare(&sum, &self.m) != Ordering::Less {
            subtract(&mut sum, &self.m);
        }
        sum
    }

    /// `a mod m`, for any `a`, by shifting its bits in.
    fn reduce(&self, a: &[u64]) -> Vec<u64> {
        let mut remainder = vec![0_u64; self.m.len()];
        for limb in a.iter().rev() {
            for bit in (0..64).rev() {
                let carry = add_self(&mut remainder);
                remainder[0] |= (limb >> bit) & 1;
                if carry || compare(&remainder, &self.m) != Ordering::Less {
                    subtract(&mut remainder, &self.m);
                }
            }
        }
        remainder
    }

    /// `a^-1 mod m` for a prime `m`, by Fermat.
    fn invert(&self, a: &[u64]) -> Vec<u64> {
        let mut exponent = self.m.clone();
        let mut two = vec![0_u64; exponent.len()];
        two[0] = 2;
        subtract(&mut exponent, &two);
        self.pow(a, &exponent)
    }
}

/// `a = 2a` in place (the shift of the bit-serial reduction); returns the bit shifted out.
fn add_self(a: &mut [u64]) -> bool {
    let copy = a.to_vec();
    add(a, &copy)
}

struct Domain {
    p: Modulus,
    q: Modulus,
    g: Vec<u64>,
}

fn domain() -> Domain {
    Domain {
        p: Modulus::new(from_hex(P, 16)),
        q: Modulus::new(from_hex(Q, 3)),
        g: from_hex(G, 16),
    }
}

/// SHA-1 of `data` (RFC 3174), which `ssh-dss` signs.
pub(super) fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0_u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5A82_7999),
                1 => (b ^ c ^ d, 0x6ED9_EBA1),
                2 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *state = state.wrapping_add(value);
        }
    }
    let mut digest = [0_u8; 20];
    for (chunk, word) in digest.chunks_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// An SSH `mpint`'s bytes for the big-endian `value`: no leading zeros, one zero byte before a high bit.
fn mpint(value: &[u8]) -> Vec<u8> {
    let start = value
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(value.len());
    let mut body = Vec::new();
    if value.get(start).is_some_and(|byte| byte & 0x80 != 0) {
        body.push(0);
    }
    body.extend_from_slice(&value[start..]);
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend(body);
    out
}

/// A DSA key pair over the fixed domain.
pub(super) struct DsaKey {
    /// The private exponent, 20 big-endian bytes.
    x: Vec<u8>,
    /// The public value, 128 big-endian bytes.
    y: Vec<u8>,
}

impl DsaKey {
    /// A new key from `random`.
    pub(super) fn generate(random: Random<'_>) -> Self {
        let domain = domain();
        let x = random_below_q(&domain, random);
        let y = domain.p.pow(&domain.g, &x);
        Self {
            x: to_be(&x, 20),
            y: to_be(&y, 128),
        }
    }

    /// The key from its private exponent `x` (20 big-endian bytes).
    #[cfg(test)]
    fn from_exponent(x: &[u8]) -> Self {
        let domain = domain();
        let x = domain.q.reduce(&limbs(x, 3));
        let y = domain.p.pow(&domain.g, &x);
        Self {
            x: to_be(&x, 20),
            y: to_be(&y, 128),
        }
    }

    /// The `ssh-dss` public key blob.
    pub(super) fn blob(&self) -> Vec<u8> {
        let (p, q, g) = (from_hex(P, 16), from_hex(Q, 3), from_hex(G, 16));
        let mut blob = (7_u32).to_be_bytes().to_vec();
        blob.extend_from_slice(b"ssh-dss");
        for value in [
            to_be(&p, 128),
            to_be(&q, 20),
            to_be(&g, 128),
            self.y.clone(),
        ] {
            blob.extend(mpint(&value));
        }
        blob
    }

    /// The 40-byte signature (`r` then `s`) of `data`.
    pub(super) fn sign(&self, data: &[u8], random: Random<'_>) -> Vec<u8> {
        self.sign_with(data, || random_below_q(&domain(), random))
    }

    fn sign_with(&self, data: &[u8], mut nonce: impl FnMut() -> Vec<u64>) -> Vec<u8> {
        let domain = domain();
        let digest = domain.q.reduce(&limbs(&sha1(data), 3));
        let x = limbs(&self.x, 3);
        loop {
            let k = nonce();
            let r = domain.q.reduce(&domain.p.pow(&domain.g, &k));
            if is_zero(&r) {
                continue;
            }
            let sum = domain.q.add(&digest, &domain.q.mul(&x, &r));
            let s = domain.q.mul(&domain.q.invert(&k), &sum);
            if is_zero(&s) {
                continue;
            }
            let mut signature = to_be(&r, 20);
            signature.extend(to_be(&s, 20));
            return signature;
        }
    }
}

/// Whether `signature` (`r` then `s`, 20 bytes each) is `blob`'s key's signature of `data`.
pub(super) fn verify(blob: &[u8], data: &[u8], signature: &[u8]) -> bool {
    let domain = domain();
    let Some(y) = public_value(blob) else {
        return false;
    };
    if signature.len() != 40 {
        return false;
    }
    let (r, s) = (limbs(&signature[..20], 3), limbs(&signature[20..], 3));
    let below_q = |value: &[u64]| !is_zero(value) && compare(value, &domain.q.m) == Ordering::Less;
    if !below_q(&r) || !below_q(&s) {
        return false;
    }
    let w = domain.q.invert(&s);
    let u1 = domain.q.mul(&domain.q.reduce(&limbs(&sha1(data), 3)), &w);
    let u2 = domain.q.mul(&r, &w);
    let v = domain.p.mul(
        &domain.p.pow(&domain.g, &u1),
        &domain.p.pow(&limbs(&y, 16), &u2),
    );
    domain.q.reduce(&v) == r
}

/// The public value of an `ssh-dss` blob over the fixed domain.
fn public_value(blob: &[u8]) -> Option<Vec<u8>> {
    let mut rest = blob;
    let mut fields = Vec::new();
    for _ in 0..5 {
        let length = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        fields.push(rest.get(4..4 + length)?);
        rest = &rest[4 + length..];
    }
    let trimmed = |field: &[u8]| {
        field
            .iter()
            .skip_while(|byte| **byte == 0)
            .copied()
            .collect::<Vec<u8>>()
    };
    let domain_matches = trimmed(fields[1]) == to_be(&from_hex(P, 16), 128)
        && trimmed(fields[2]) == to_be(&from_hex(Q, 3), 20)
        && trimmed(fields[3]) == to_be(&from_hex(G, 16), 128);
    (fields[0] == b"ssh-dss" && rest.is_empty() && domain_matches).then(|| trimmed(fields[4]))
}

/// A random value in `[1, q - 1]` from 24 random bytes (the bias is of no matter to a test).
fn random_below_q(domain: &Domain, random: Random<'_>) -> Vec<u64> {
    loop {
        let mut bytes = [0_u8; 24];
        random(&mut bytes);
        let value = domain.q.reduce(&limbs(&bytes, 3));
        if !is_zero(&value) {
            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Python's `pow`, `hashlib` and the arithmetic of `key_agent.py`'s `dsa_sign`, which made these values.
    const X: &str = "0123456789abcdef0123456789abcdef01234567";
    const Y: &str = "715d493ae57f08738ea648d83ac937fd7f9bf4d4f26f35071b12e0c272b9e5eab483204a676c1bb45defce05f6b3fe4a6f78eb9ddb9078aef2be8604171623f81578e538cbdfff37a7c213b7826feb0bd1b71466547c45af10d10335e6b04530958cf4447228280ce69a3d5ecc8775a91460f1ee09edc24228e6232959e4faea";
    const DATA: &[u8] = b"data the client wants signed";
    const R: &str = "592a694ad03b042de0f4ac02055af902843855d7";
    const S: &str = "8cba4c64bb9d3ee282aec62951a439591579c726";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn counter() -> impl Fn(&mut [u8]) {
        let state = std::cell::Cell::new(7_u8);
        move |buffer: &mut [u8]| {
            for byte in buffer {
                state.set(state.get().wrapping_mul(31).wrapping_add(17));
                *byte = state.get();
            }
        }
    }

    #[test]
    fn sha1_matches_the_published_vectors() {
        let hex_of = |digest: [u8; 20]| {
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex_of(sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex_of(sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hex_of(sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn modular_arithmetic_matches_python() {
        let domain = domain();
        let exponent = limbs(&hex("deadbeefcafebabe0123456789abcdef01234567"), 3);
        assert_eq!(
            to_be(&domain.p.pow(&domain.g, &exponent), 128),
            hex(
                "96cb516345da07d8b0f4a19f64dfc7e427d3de1483134014f3a3f18d7d5ad7c5127ec756c73bfe72cec225eda731003215c4849bb7c2b891b2ae50f815c26649b19df63d331111121fb8b3abae9e12cb4a109ea0bb4d4a80b81127ed407b095e7f1429c5e706850ea579a94bdc404607839d4ef438f7085674bbfe89ab20354c"
            )
        );
        assert_eq!(
            to_be(&domain.q.invert(&limbs(&hex("1234567890abcdef"), 3)), 20),
            hex("a48365b64cf8e239f0e544a4ca27c104a76c5b74")
        );
        // g has order q.
        assert_eq!(domain.p.pow(&domain.g, &domain.q.m)[0], 1);
    }

    #[test]
    fn the_public_value_and_a_signature_match_python() {
        let key = DsaKey::from_exponent(&hex(X));
        assert_eq!(key.y, hex(Y));
        let k = limbs(&hex("1111111111111111222222222222222233333333"), 3);
        let signature = key.sign_with(DATA, || k.clone());
        assert_eq!(signature, [hex(R), hex(S)].concat());
        assert!(verify(&key.blob(), DATA, &signature));
    }

    #[test]
    fn a_signature_verifies_only_for_its_data_and_key() {
        let random = counter();
        let (key, other) = (DsaKey::generate(&random), DsaKey::generate(&random));
        let signature = key.sign(DATA, &random);
        assert_eq!(signature.len(), 40);
        assert!(verify(&key.blob(), DATA, &signature));
        assert!(!verify(&key.blob(), b"other data", &signature));
        assert!(!verify(&other.blob(), DATA, &signature));
        let mut damaged = signature.clone();
        damaged[39] ^= 1;
        assert!(!verify(&key.blob(), DATA, &damaged));
        assert!(!verify(&key.blob(), DATA, &signature[..39]));
        assert_ne!(
            key.sign(DATA, &random),
            signature,
            "a fresh nonce each time"
        );
    }

    #[test]
    fn the_blob_is_an_ssh_dss_key() {
        let blob = DsaKey::from_exponent(&hex(X)).blob();
        assert_eq!(&blob[..11], b"\0\0\0\x07ssh-dss");
        assert_eq!(public_value(&blob).unwrap(), hex(Y));
        // p's high bit is set, so its mpint has a zero byte first: 129 bytes.
        assert_eq!(&blob[11..15], &129_u32.to_be_bytes());
    }
}
