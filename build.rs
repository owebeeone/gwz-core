#[allow(
    clippy::disallowed_methods,
    reason = "build-time source discovery requires the host filesystem"
)]
#[path = "build_support/provenance.rs"]
mod provenance;
fn main() {
    provenance::emit();
}
