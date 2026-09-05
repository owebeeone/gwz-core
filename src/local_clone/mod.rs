//! Local clone family: thin composition adapters (LCM1.0c checkpoint).
//!
//! Library logic lives in the independently compiled crates under
//! `crates/` (gwz-dev `dev-docs/GwzLocalCloneLibraryBoundaries.md` §2).
//! This module keeps only what core must own: request-shape validation and
//! the typed refusal order, the translation of library errors to
//! `ModelError`, the [`transport::BackendLocalTransport`] adapter that
//! implements `gwz_local_import::LocalTransport` over the anonymous local
//! ports of [`crate::git::GitBackend`], the family-merge wrapper that
//! resolves and imports under the family lock before delegating once to the
//! public merge engine entry, and the [`list`] projection of the model's
//! observation-only listing onto the `LocalFamilyResponse.members` payload.
//!
//! Refusal order, which every dispatch slot preserves (design §6.2, plan
//! LCM1.0c): attribution and request shape first; an unsupported family
//! `dry_run` next; then the family observation through the store contract;
//! only then any lock file, metadata reservation, copy or import. Since W2
//! (lane S) the store implementation is real, so the observation is a real
//! read of the family metadata rather than a refusal, and what stops an
//! operation is now the operation itself: `dispose` and `disband` refuse
//! `unsupported_operation`; `list` answers with an empty member list outside
//! a family and refuses inside one at [`list::observe_members`]; a family
//! merge refuses `unknown_local` for a token that names no ready member and
//! `unsupported_operation` for one that does, until lane X lands the import.
//! Every one of those paths still creates nothing -- observing a family is a
//! read. Later adapters (copy, installation, disposal, evidence) are added
//! here by the integration lane as their libraries land.
//!
//! # History-check adapter rule (lane H proposal H2, LCM1.0c follow-up 2)
//!
//! The disposal port's `check_history` adapter, when it lands in this
//! module, calls `gwz_history_check::check_history` **once per witness
//! store**: each call gets one `ObjectReader` that serves exactly one
//! surviving family repository's object store, and the adapter combines
//! the per-witness outcomes (a protected root is preserved when some single
//! witness preserves it whole). It never hands the verifier a union reader
//! spanning several witnesses, because a union reader could complete one
//! witness's graph with another witness's objects and so certify a root as
//! preserved in a repository that does not hold its whole subgraph -- the
//! design's "surviving family repositories" proof (§5.1, §11 item 9) is
//! per repository. The verifier's signature does not enforce this yet;
//! the rule lives here and in the LCM1.0c checkpoint §11 until it does.

pub mod adapters;
pub mod create;
pub mod dispose;
pub mod errors;
pub mod family_merge;
pub mod list;
pub mod request;
pub mod transport;

#[cfg(test)]
mod tests;
