#[allow(
    clippy::disallowed_methods,
    reason = "build-time source discovery requires the host filesystem"
)]
#[path = "build_support/provenance.rs"]
mod provenance;
fn main() {
    println!("cargo:rustc-check-cfg=cfg(gwz_transport_candidate)");
    provenance::emit();
}
