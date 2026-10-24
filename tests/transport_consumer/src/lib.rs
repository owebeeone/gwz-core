//! Test-only core consumer. No production transport capability is advertised.
pub use gwz_transport::cbor;
#[path = "../candidate/admission.rs"]
pub mod candidate_admission;
// gwz-core's own protocol, which composes the owner's export: the module this proof reads the retained old reader against.
#[path = "../../../src/protocol/generated.rs"]
pub mod core_generated;
pub mod generated;
#[path = "../candidate/retained_old_generated.rs"]
pub mod retained_old_generated;
