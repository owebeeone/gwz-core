// GENERATED conformance vectors + byte-parity test (tautc corpus) — do not edit.
// Requires the crate root to re-export its taut types + `Cbor`/`encode`/`decode`,
// e.g. `pub use generated::*; pub use cbor::{Cbor, encode, decode};`.
#![allow(dead_code)]

#[rustfmt::skip]
pub static VECTORS: &[(&str, &str, &str)] = &[
    ("CheckedActionCapacityReservationV1", "CheckedActionCapacityReservationV1", "a40143010102024302010203a501260281a201430101020219012c038100044304010205430501020443040102"),
    ("CheckedActionDirectoryAdmissionV1", "CheckedActionDirectoryAdmissionV1", "a801010243020102034303010204430401020543050102064306010207430701020843080102"),
    ("CheckedActionScheduleV1", "CheckedActionScheduleV1", "a501260281a201430101020219012c03810004430401020543050102"),
    ("CheckedAuthorityV1", "CheckedAuthorityV1", "aa014301010202430201020343030102044304010205a10181a601430101020200034303010204a8010102430201020300044304010205430501020643060102074307010208430801020543050102064306010206a80101024302010203000443040102054305010206430601020743070102084308010207a301a80101024302010203000443040102054305010206430601020743070102084308010202430201020343030102084308010209430901020a430a0102"),
    ("CheckedBarrierIntentV1", "CheckedBarrierIntentV1", "ac01430101020243020102034303010204182a05a80101024302010203000443040102054305010206430601020743070102084308010206a801010243020102030004430401020543050102064306010207430701020843080102074307010208a80101024302010203000443040102054305010206430601020743070102084308010209a10181a601430101020200034303010204a801010243020102030004430401020543050102064306010207430701020843080102054305010206430601020a430a01020b430b01020c430c0102"),
    ("CheckedCanonicalComponentV1", "CheckedCanonicalComponentV1", "a601430101020200034303010204a80101024302010203000443040102054305010206430601020743070102084308010205430501020643060102"),
    ("CheckedCanonicalPathIdentityV1", "CheckedCanonicalPathIdentityV1", "a10181a601430101020200034303010204a80101024302010203000443040102054305010206430601020743070102084308010205430501020643060102"),
    ("CheckedCatalogBootstrapV1", "CheckedCatalogBootstrapV1", "ad01010202034303010204430401020543050102064306010207a80101024302010203000443040102054305010206430601020743070102084308010208a10181a601430101020200034303010204a8010102430201020300044304010205430501020643060102074307010208430801020543050102064306010209430901020a430a01020b430b01020c430c01020d430d0102"),
    ("CheckedCleanupRowV1", "CheckedCleanupRowV1", "a2010102a301a80101024302010203000443040102054305010206430601020743070102084308010202430201020343030102"),
    ("CheckedCleanupWorklistV1", "CheckedCleanupWorklistV1", "a50143010102024302010203430301020481a2010102a301a801010243020102030004430401020543050102064306010207430701020843080102024302010203430301020543050102"),
    ("CheckedDurableLeafFingerprintV1", "CheckedDurableLeafFingerprintV1", "a301a80101024302010203000443040102054305010206430601020743070102084308010202430201020343030102"),
    ("CheckedDurableObjectIdentityV1", "CheckedDurableObjectIdentityV1", "a801010243020102030004430401020543050102064306010207430701020843080102"),
    ("CheckedInfrastructureV1", "CheckedInfrastructureV1", "aa012602a80101024302010203000443040102054305010206430601020743070102084308010203a80101024302010203000443040102054305010206430601020743070102084308010204a80101024302010203000443040102054305010206430601020743070102084308010205a80101024302010203000443040102054305010206430601020743070102084308010206430601020743070102084308010209430901020a430a0102"),
    ("CheckedManagedBootstrapComponentV1", "CheckedManagedBootstrapComponentV1", "a70143010102024302010203430301020443040102052606430601020743070102"),
    ("CheckedManagedBootstrapInputV1", "CheckedManagedBootstrapInputV1", "a201430101020219012c"),
    ("CheckedManagedParentBootstrapIntentV1", "CheckedManagedParentBootstrapIntentV1", "b4014301010202430201020343030102044304010205430501020619012c070008182a09260aa8010102430201020300044304010205430501020643060102074307010208430801020b010ca10181a601430101020200034303010204a801010243020102030004430401020543050102064306010207430701020843080102054305010206430601020d81a701430101020243020102034303010204430401020526064306010207430701020e430e01020f430f010210011126124312010213031443140102"),
    ("CheckedOwnershipMarkerV1", "CheckedOwnershipMarkerV1", "ac014301010202430201020343030102044304010205260619012c0700084308010209430901020a430a01020b430b01020c430c0102"),
];

/// Decode->re-encode dispatch by message name, over this crate's generated types.
pub fn reencode(message: &str, c: &crate::Cbor) -> crate::Cbor {
    match message {
        "CheckedActionCapacityReservationV1" => crate::CheckedActionCapacityReservationV1::from_cbor(c).expect("corpus decode: CheckedActionCapacityReservationV1").to_cbor(),
        "CheckedActionDirectoryAdmissionV1" => crate::CheckedActionDirectoryAdmissionV1::from_cbor(c).expect("corpus decode: CheckedActionDirectoryAdmissionV1").to_cbor(),
        "CheckedActionScheduleV1" => crate::CheckedActionScheduleV1::from_cbor(c).expect("corpus decode: CheckedActionScheduleV1").to_cbor(),
        "CheckedAuthorityV1" => crate::CheckedAuthorityV1::from_cbor(c).expect("corpus decode: CheckedAuthorityV1").to_cbor(),
        "CheckedBarrierIntentV1" => crate::CheckedBarrierIntentV1::from_cbor(c).expect("corpus decode: CheckedBarrierIntentV1").to_cbor(),
        "CheckedCanonicalComponentV1" => crate::CheckedCanonicalComponentV1::from_cbor(c).expect("corpus decode: CheckedCanonicalComponentV1").to_cbor(),
        "CheckedCanonicalPathIdentityV1" => crate::CheckedCanonicalPathIdentityV1::from_cbor(c).expect("corpus decode: CheckedCanonicalPathIdentityV1").to_cbor(),
        "CheckedCatalogBootstrapV1" => crate::CheckedCatalogBootstrapV1::from_cbor(c).expect("corpus decode: CheckedCatalogBootstrapV1").to_cbor(),
        "CheckedCleanupRowV1" => crate::CheckedCleanupRowV1::from_cbor(c).expect("corpus decode: CheckedCleanupRowV1").to_cbor(),
        "CheckedCleanupWorklistV1" => crate::CheckedCleanupWorklistV1::from_cbor(c).expect("corpus decode: CheckedCleanupWorklistV1").to_cbor(),
        "CheckedDurableLeafFingerprintV1" => crate::CheckedDurableLeafFingerprintV1::from_cbor(c).expect("corpus decode: CheckedDurableLeafFingerprintV1").to_cbor(),
        "CheckedDurableObjectIdentityV1" => crate::CheckedDurableObjectIdentityV1::from_cbor(c).expect("corpus decode: CheckedDurableObjectIdentityV1").to_cbor(),
        "CheckedInfrastructureV1" => crate::CheckedInfrastructureV1::from_cbor(c).expect("corpus decode: CheckedInfrastructureV1").to_cbor(),
        "CheckedManagedBootstrapComponentV1" => crate::CheckedManagedBootstrapComponentV1::from_cbor(c).expect("corpus decode: CheckedManagedBootstrapComponentV1").to_cbor(),
        "CheckedManagedBootstrapInputV1" => crate::CheckedManagedBootstrapInputV1::from_cbor(c).expect("corpus decode: CheckedManagedBootstrapInputV1").to_cbor(),
        "CheckedManagedParentBootstrapIntentV1" => crate::CheckedManagedParentBootstrapIntentV1::from_cbor(c).expect("corpus decode: CheckedManagedParentBootstrapIntentV1").to_cbor(),
        "CheckedOwnershipMarkerV1" => crate::CheckedOwnershipMarkerV1::from_cbor(c).expect("corpus decode: CheckedOwnershipMarkerV1").to_cbor(),
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
