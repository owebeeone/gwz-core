// GENERATED conformance vectors + byte-parity test (tautc corpus) — do not edit.
// Requires the crate root to re-export its taut types + `Cbor`/`encode`/`decode`,
// e.g. `pub use generated::*; pub use cbor::{Cbor, encode, decode};`.
#![allow(dead_code)]

#[rustfmt::skip]
pub static VECTORS: &[(&str, &str, &str)] = &[
    ("CheckedActionCapacityReservationV1", "CheckedActionCapacityReservationV1", "a40143010102024302010203a401260281a201430101020219012c03810004430401020443040102"),
    ("CheckedActionDirectoryAdmissionV1", "CheckedActionDirectoryAdmissionV1", "a70101024302010203430301020443040102054305010206430601020743070102"),
    ("CheckedActionScheduleV1", "CheckedActionScheduleV1", "a401260281a201430101020219012c0381000443040102"),
    ("CheckedBarrierIntentV1", "CheckedBarrierIntentV1", "ab01430101020243020102034303010204182a05a80101024302010203000443040102054305010206430601020743070102084308010206a801010243020102030004430401020543050102064306010207430701020843080102074307010208a80101024302010203000443040102054305010206430601020743070102084308010209a10181a30143010102020003430301020a430a01020b430b0102"),
    ("CheckedCanonicalComponentV1", "CheckedCanonicalComponentV1", "a3014301010202000343030102"),
    ("CheckedCanonicalPathIdentityV1", "CheckedCanonicalPathIdentityV1", "a10181a3014301010202000343030102"),
    ("CheckedCleanupRowV1", "CheckedCleanupRowV1", "a2010102a301a80101024302010203000443040102054305010206430601020743070102084308010202430201020343030102"),
    ("CheckedCleanupWorklistV1", "CheckedCleanupWorklistV1", "a40143010102024302010203430301020481a2010102a301a80101024302010203000443040102054305010206430601020743070102084308010202430201020343030102"),
    ("CheckedDurableLeafFingerprintV1", "CheckedDurableLeafFingerprintV1", "a301a80101024302010203000443040102054305010206430601020743070102084308010202430201020343030102"),
    ("CheckedDurableObjectIdentityV1", "CheckedDurableObjectIdentityV1", "a801010243020102030004430401020543050102064306010207430701020843080102"),
    ("CheckedManagedBootstrapInputV1", "CheckedManagedBootstrapInputV1", "a201430101020219012c"),
];

/// Decode->re-encode dispatch by message name, over this crate's generated types.
pub fn reencode(message: &str, c: &crate::Cbor) -> crate::Cbor {
    match message {
        "CheckedActionCapacityReservationV1" => crate::CheckedActionCapacityReservationV1::from_cbor(c).expect("corpus decode: CheckedActionCapacityReservationV1").to_cbor(),
        "CheckedActionDirectoryAdmissionV1" => crate::CheckedActionDirectoryAdmissionV1::from_cbor(c).expect("corpus decode: CheckedActionDirectoryAdmissionV1").to_cbor(),
        "CheckedActionScheduleV1" => crate::CheckedActionScheduleV1::from_cbor(c).expect("corpus decode: CheckedActionScheduleV1").to_cbor(),
        "CheckedBarrierIntentV1" => crate::CheckedBarrierIntentV1::from_cbor(c).expect("corpus decode: CheckedBarrierIntentV1").to_cbor(),
        "CheckedCanonicalComponentV1" => crate::CheckedCanonicalComponentV1::from_cbor(c).expect("corpus decode: CheckedCanonicalComponentV1").to_cbor(),
        "CheckedCanonicalPathIdentityV1" => crate::CheckedCanonicalPathIdentityV1::from_cbor(c).expect("corpus decode: CheckedCanonicalPathIdentityV1").to_cbor(),
        "CheckedCleanupRowV1" => crate::CheckedCleanupRowV1::from_cbor(c).expect("corpus decode: CheckedCleanupRowV1").to_cbor(),
        "CheckedCleanupWorklistV1" => crate::CheckedCleanupWorklistV1::from_cbor(c).expect("corpus decode: CheckedCleanupWorklistV1").to_cbor(),
        "CheckedDurableLeafFingerprintV1" => crate::CheckedDurableLeafFingerprintV1::from_cbor(c).expect("corpus decode: CheckedDurableLeafFingerprintV1").to_cbor(),
        "CheckedDurableObjectIdentityV1" => crate::CheckedDurableObjectIdentityV1::from_cbor(c).expect("corpus decode: CheckedDurableObjectIdentityV1").to_cbor(),
        "CheckedManagedBootstrapInputV1" => crate::CheckedManagedBootstrapInputV1::from_cbor(c).expect("corpus decode: CheckedManagedBootstrapInputV1").to_cbor(),
        other => panic!("reencode: unknown message {other}"),
    }
}

#[cfg(test)]
mod conformance {
    use super::reencode;
    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
    fn hexof(b: &[u8]) -> String {
        use std::fmt::Write as _;
        b.iter().fold(String::new(), |mut s, x| { let _ = write!(s, "{x:02x}"); s })
    }
    /// Parity == correctness: every golden vector (bytes from taut's Python codec)
    /// must decode and re-encode to the identical bytes via this crate's codec.
    #[test]
    fn corpus_byte_parity() {
        assert!(!super::VECTORS.is_empty(), "empty corpus");
        for (name, message, golden) in super::VECTORS {
            let out = hexof(&crate::encode(&reencode(message, &crate::decode(&unhex(golden)))));
            assert_eq!(&out, golden, "byte mismatch for {name} ({message})");
        }
    }
}
