//! The family-merge wrapper: `gwz merge --remote <name> [<ref>]`.
//!
//! Order (design §3.2, §6.2; architecture §7):
//!
//! 1. attribution and request shape ([`super::request::validate_family_merge`]),
//!    including the unsupported family `dry_run`;
//! 2. workspace discovery (read-only);
//! 3. the family observation through the store contract (never creates the
//!    lock file);
//! 4. resolution through `gwz_family_model::resolve_remote_token` with
//!    `Verb::Merge` (family-only; no Git fallback);
//! 5. under the family lock only: pair every selected participant, capture
//!    source ids, fetch through [`super::transport::BackendLocalTransport`]
//!    into `refs/gwz/local-imports/<transfer-id>`, verify the received
//!    vector;
//! 6. clear the selector, set the common import ref as `source_ref`, and call
//!    the public [`crate::workspace_ops::handle_merge_with_events`] once. The
//!    engine takes its own locks; the wrapper holds only the family lock.
//!
//! LCM1.0c checkpoint, as of W2: steps 1-4 run for real, step 3 against the
//! real store (lane S). Nothing after step 4 executes -- the import and the
//! delegation are lane X's steps 5-6 -- so no lock file, ref or record is
//! created on any path through this wrapper. Step 4
//! is [`resolve_family_merge`]: on an observed view, a token that names no
//! ready family member is the design's `UnknownLocal` --
//! `GwzErrorCode.unknown_local` (62) with the state detail in the message
//! (operator ruling 2026-09-05, design §6/§7, §11 item 13) -- and a bound
//! member is reported unsupported until steps 5-6 land with lane X.

use std::path::Path;

use gwz_family_model::{
    BoundMember, FamilyView, RemoteToken, Resolution, Verb, resolve_remote_token,
};
use gwz_family_store_contract::{FamilyLocation, FamilyStore};

use super::errors;
use super::request::validate_family_merge;
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OperationRequest};
use crate::workspace_ops::resolve_workspace_root;

/// Step 4: resolve the family selector against the observed family
/// (`None` when the workspace holds neither an index nor a pointer), with
/// `Verb::Merge` -- family-only, no Git-remote fallback. A miss is
/// [`errors::unknown_local`] carrying the row's state when a row exists.
pub(crate) fn resolve_family_merge(
    view: Option<&FamilyView>,
    token: &RemoteToken,
) -> ModelResult<BoundMember> {
    match resolve_remote_token(view, Some(token), Verb::Merge) {
        Resolution::Bound(member) => Ok(member),
        Resolution::UnknownLocal { token, state } => Err(errors::unknown_local(&token, state)),
        // `Verb::Merge` has no lifecycle split and no Git-remote candidate,
        // and the token was validated present: the resolver never answers a
        // merge this way. Reaching here is a resolver contract violation,
        // not a request or family problem.
        Resolution::LifecycleRefusal { .. }
        | Resolution::GitRemoteCandidate { .. }
        | Resolution::NoToken => Err(ModelError::new(
            ErrorCode::InternalError,
            format!(
                "local family merge: the family resolver answered `{}` with a pull/push \
                 outcome",
                token.as_str()
            ),
        )),
    }
}

/// The store implementation core composes for family observations.
pub(crate) fn family_store() -> gwz_family_store::YamlFamilyStore {
    gwz_family_store::YamlFamilyStore::new()
}

pub(crate) fn handle<B>(
    _backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: String,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse>
where
    B: MergeAuthorityBackend,
{
    let context = OperationRequest::Merge(request.clone()).context(operation_id)?;
    let emitter = EventEmitter::new(&context, events, 0);
    emitter.operation_started();
    let result = (|| {
        let selector = validate_family_merge(&request)?;
        let what = format!("local family merge from `{}`", selector.token.as_str());
        let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
        let observation = family_store()
            .read_view(&FamilyLocation::new(&root))
            .map_err(|error| errors::store_in(&what, &error))?;
        let _bound = resolve_family_merge(observation.view(), &selector.token)?;
        // Steps 5-6 (locked import, engine delegation) are lane X's work;
        // reaching here with a real store and a bound member is the next
        // checkpoint. Until then a bound selector is reported unsupported.
        Err(errors::unsupported(&what))
    })();
    emitter.operation_finished();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_family_model::{
        AllocationId, CloneMode, FamilyId, MemberKind, MemberName, MemberRow, MemberState,
        ROOT_NAME,
    };

    fn row(path: &str, state: MemberState) -> MemberRow {
        MemberRow {
            path: path.to_owned(),
            kind: MemberKind::Checkout,
            state,
            allocation_id: AllocationId::new(format!("alloc-{path}")).unwrap(),
            source_path: ".".to_owned(),
            mode: CloneMode::Verbatim,
            last_error: None,
        }
    }

    /// A ready `A`, a creating `B` and a disposing `C`.
    fn view() -> FamilyView {
        let mut view = FamilyView::founded(
            FamilyId::new("fam_test").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        );
        for (name, path, state) in [
            ("A", "../ws-A", MemberState::Ready),
            ("B", "../ws-B", MemberState::Creating),
            ("C", "../ws-C", MemberState::Disposing),
        ] {
            view.members
                .insert(MemberName::parse(name).unwrap(), row(path, state));
        }
        view
    }

    /// Design §6/§7 (operator ruling 2026-09-05): on an observed family, a
    /// ready member and `root` bind; a creating or disposing row, an absent
    /// name and the reserved `origin` are `unknown_local`, the not-ready
    /// cases naming their state. Outside any family every token misses.
    #[test]
    fn a_family_merge_selector_binds_a_ready_member_and_misses_as_unknown_local() {
        let view = view();
        let bound = resolve_family_merge(Some(&view), &RemoteToken::new("A")).unwrap();
        assert_eq!((bound.name.as_str(), bound.path.as_str()), ("A", "../ws-A"));
        assert!(!bound.is_root);
        let root = resolve_family_merge(Some(&view), &RemoteToken::new(ROOT_NAME)).unwrap();
        assert!(root.is_root);

        for (token, state) in [
            ("B", Some(MemberState::Creating)),
            ("C", Some(MemberState::Disposing)),
            ("D", None),
            ("origin", None),
        ] {
            let error = resolve_family_merge(Some(&view), &RemoteToken::new(token)).unwrap_err();
            assert_eq!(error.code, ErrorCode::UnknownLocal, "{token}");
            assert!(error.message.contains(token), "{token}: {}", error.message);
            match state {
                Some(state) => assert!(
                    error.message.contains(state.as_str()),
                    "{token}: {}",
                    error.message
                ),
                None => assert!(
                    error.message.contains("no ready family member"),
                    "{token}: {}",
                    error.message
                ),
            }
        }

        let outside = resolve_family_merge(None, &RemoteToken::new("A")).unwrap_err();
        assert_eq!(outside.code, ErrorCode::UnknownLocal);
    }
}
