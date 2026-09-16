//! `gwz-local-disposal`: explicit keep and one-shot disposal (lane D).
//!
//! Disband has no entry point here and never will: it removes pointers and
//! the index and no directory contents, so it is core's own composition over
//! `gwz_family_model::FamilyChange::Disband` and the store session, and it
//! must never route a remaining row through this crate's removal path
//! (design §5.2, last paragraph).
//!
//! [`dispose`] is the only local-clone service that removes directory
//! contents (gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §8, design §5). Under the family lock it validates an intact ready
//! target, gathers fresh work evidence through [`DisposalPorts`] and
//! classifies it with `gwz-work-detector`, asks the history port whether
//! every protected root is preserved elsewhere, refuses dirty, unpreserved
//! or unknown evidence unless a named [`HazardWaiver`] covers it, writes
//! `disposing` through the store session, then removes the validated
//! directory once. `--keep` detaches the matching row and pointer and
//! retains every file. On any error it stops and reports what remains; there
//! is no rollback or replay. The hazard vocabulary is owned here; core uses
//! it to reject unknown request hazards before any effect.
//!
//! **The refusal is the product.** The operator's standing default (design
//! §5, 2026-09-05) is that deletion refuses unless history is verifiably
//! preserved elsewhere, so every uncertain answer refuses: `Unknown` from
//! the work detector or the history port is never waivable by any force
//! name, and a path mismatch, an incomplete create and an interrupted
//! deletion are not forceable at all. A named waiver is an operator loss
//! waiver over an intact ready tree, never crash recovery. On every refusal
//! path the removal port is not called.

#![forbid(unsafe_code)]

use std::path::{Component, Path, PathBuf};

use gwz_family_model::{MemberState, Refusal, classify_target};
use gwz_family_store_contract::{FamilySession, StoreError};
use gwz_repo_contract::UnknownReason;
use gwz_work_detector::{GwzEvidence, Hazard};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

mod hazard;
mod inspect;
mod policy;
mod ports;
mod report;
mod run;
mod validate;

#[cfg(test)]
mod tests;

pub use hazard::*;
pub use policy::*;
pub use ports::*;
pub use report::*;

pub(crate) use inspect::*;
pub(crate) use run::*;
pub(crate) use validate::*;

/// Keep, or check and remove, one family member.
///
/// The design §5.2 sequence, in order, with the port each step consults:
///
/// 1. **Validate name, pointer and path** — pure, over the session's own
///    reread view. Refuses an unrecognised name, an unusable recorded path,
///    the root, a target containing the working directory, a target
///    overlapping the root or another member, and a `root` the held lock
///    does not belong to ([`DisposeError::PathMismatch`], the moved-root
///    case). No port is consulted.
/// 2. **`--keep`** — [`FamilySession::remove_pointer`] then `RemoveRow`
///    with [`RemovalReason::Keep`]. No evidence, history or removal call:
///    every file stays, including an incomplete or interrupted tree the
///    ordinary path refuses.
/// 3. **Fresh checks** — one [`DisposalPorts::observe_target`], then per
///    repository `gwz_work_detector::classify_observed_work` (pure) and one
///    [`DisposalPorts::check_history`]. `Unknown` from either dominates and
///    is never waivable; a *known* hazard refuses unless its
///    [`HazardWaiver`] was named. An absent target takes the stale-row exit
///    (step 5) instead, and every other observed state is a `PathMismatch`.
/// 4. **Remove** — `MarkDisposing` through the session, its result checked,
///    then exactly one [`DisposalPorts::remove_directory`] on the validated
///    target. An error stops and reports what remains; nothing is rolled
///    back or replayed.
/// 5. **Detach** — `remove_pointer` then `RemoveRow`, so a pointer the
///    store cannot remove is reported as the pointer, not as the row.
pub fn dispose(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
    ports: &mut dyn DisposalPorts,
) -> Result<DisposeReport, DisposeFailure> {
    let mut effects = Vec::new();
    match run(request, session, ports, &mut effects) {
        Ok(()) => Ok(DisposeReport { effects }),
        Err(error) => Err(DisposeFailure { error, effects }),
    }
}

/// One lexical resolution of a host path, matching the family-store
/// contract's reference resolution (`contract_tests::resolve`), so this
/// library, the model and the store agree on which directory a spelling
/// names. It opens nothing: symlink equivalence is the store's to close by
/// canonicalising before it hands the paths over (see [`DisposeRequest`]).
fn resolve(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match resolved.components().next_back() {
                Some(Component::Normal(_)) => {
                    resolved.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => resolved.push(".."),
            },
            other => resolved.push(other),
        }
    }
    resolved
}
