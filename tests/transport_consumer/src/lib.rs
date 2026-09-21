//! Test-only core consumer. No production transport capability is advertised.
pub use gwz_transport::cbor;
#[path = "../candidate/admission.rs"]
pub mod candidate_admission;
#[path = "../candidate/candidate_generated.rs"]
pub mod candidate_generated;
pub mod generated;
#[path = "../candidate/retained_old_generated.rs"]
pub mod retained_old_generated;
