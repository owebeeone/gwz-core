mod git_host;
mod git_transfer_progress;
mod gitbackend;
cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        pub(crate) mod endpoint;
    }
}
pub(crate) mod regular_file;
pub(crate) mod spawn_gate;
#[cfg(test)]
mod tests;
mod url_scheme;
mod verify_checkout_state;

pub use git_host::*;
pub(crate) use git_transfer_progress::*;
pub use gitbackend::*;
pub use url_scheme::*;
pub(crate) use verify_checkout_state::*;
