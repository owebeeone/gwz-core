//! Local clone family: thin composition adapters (LCM1.0c checkpoint).
//!
//! Library logic lives in the independently compiled crates under
//! `crates/` (gwz-dev `dev-docs/GwzLocalCloneLibraryBoundaries.md` §2).
//! This module keeps only what core must own: request-shape validation and
//! the typed refusal order, the translation of library errors to
//! `ModelError`, the [`transport::BackendLocalTransport`] adapter that
//! implements `gwz_local_import::LocalTransport` over the anonymous local
//! ports of [`crate::git::GitBackend`], and the family-merge wrapper that
//! resolves and imports under the family lock before delegating once to the
//! public merge engine entry.
//!
//! Refusal order, which every dispatch slot preserves (design §6.2, plan
//! LCM1.0c): attribution and request shape first; an unsupported family
//! `dry_run` next; then the family observation through the store contract;
//! only then any lock file, metadata reservation, copy or import. At this
//! checkpoint the store implementation refuses `Unimplemented`, so every
//! local-family operation stops there with `unsupported_operation` and no
//! effect. Later adapters (copy, installation, disposal, evidence) are added
//! here by the integration lane as their libraries land.

pub mod errors;
pub mod family_merge;
pub mod request;
pub mod transport;

#[cfg(test)]
mod tests;
