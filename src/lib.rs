// The taut-generated runtime imports through `alloc::` paths (it is written to
// be no_std-friendly); a std crate must link `alloc` explicitly for those paths
// to resolve.
extern crate alloc;

#[rustfmt::skip]
#[path = "cbor.rs"]
// taut-generated runtime; not held to clippy style (cf. `#[allow(clippy::redundant_closure)]`
// on `pub mod generated`). The 0.6.0 float encoder uses a nested `if let { if .. }`.
#[allow(clippy::collapsible_if)]
pub mod cbor;

pub mod artifact;
pub mod diff;
pub mod git;
pub mod model;
pub mod operation;
pub mod protocol;
pub mod runtime;
pub mod stash;
pub mod status;
pub mod workspace;
pub mod workspace_ops;

pub use cbor::{Cbor, decode, encode};
pub use protocol::generated::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
#[rustfmt::skip]
#[path = "../protocol/corpus/rust/vectors.rs"]
mod protocol_corpus;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
