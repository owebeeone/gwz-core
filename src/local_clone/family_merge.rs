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
//!    `Verb::Merge` (family-only; no Git fallback); then, still read-only,
//!    the addressed workspace's manifest and lock, the verb's selection and
//!    the open-merge envelope, so a start the engine would refuse before
//!    planning is refused before any fetch;
//! 5. under the family lock only: the locked family view re-resolved, the
//!    source workspace's lock read, every selected participant paired by
//!    lock member id (the root separately, root with root), source ids
//!    captured, fetched through [`super::transport::BackendLocalTransport`]
//!    into one fresh `refs/gwz/local-imports/<transfer-id>` in every paired
//!    receiver, and the received vector verified
//!    (`gwz_local_import::prepare_import`);
//! 6. clear the selector, set the common import ref as `source_ref`, and call
//!    the public [`crate::workspace_ops::handle_merge_with_events`] once. The
//!    engine takes its own locks; the wrapper holds only the family lock,
//!    across the import and the delegation (design §3.2), and preholds no
//!    receiver workspace lock.
//!
//! LCM1.2 (lane C, 2026-09-06): steps 5-6 run for real. Step 4 is
//! [`resolve_family_merge`]: on an observed view, a token that names no
//! ready family member is the design's `UnknownLocal` --
//! `GwzErrorCode.unknown_local` (62) with the state detail in the message
//! (operator ruling 2026-09-05, design §6/§7, §11 item 13). An import
//! failure is typed at [`super::errors::import_error_code`] and its message
//! names every retained import ref: import refs are ordinary Git refs that
//! nothing here prunes (design §6.2; the transport port has no removal
//! method), and a retry mints a fresh transfer id. The engine's own
//! refusals after the import travel unchanged, with the retained refs named
//! after them.
//!
//! Events: the wrapper emits `OperationStarted` once, before the
//! observation and the import; on a refusal before delegation it emits
//! `OperationFinished` itself, and on delegation the engine's own
//! `OperationStarted` is the one event the wrapper withholds, so a driver
//! sees exactly one operation lifecycle either way.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gwz_family_model::{
    BoundMember, FamilyView, RemoteToken, Resolution, Verb, resolve_remote_token,
};
use gwz_family_store_contract::{
    FamilyLocation, FamilyObservation, FamilySession, FamilyStore, StoreError,
};
use gwz_local_import::{
    ImportEffect, ImportError, ImportRequest, ImportedSource, NeverCancelled, Participant,
    SourceSelector, pair_participants, prepare_import,
};
use gwz_repo_contract::RepoKey;

use super::adapters::member_paths::mint_transfer_id;
use super::errors;
use super::request::validate_family_merge;
use super::transport::BackendLocalTransport;
use crate::artifact::{self, LockArtifact, ManifestArtifact};
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OperationRequest};
use crate::workspace_ops::{
    CommandDefaultTargets, RootSelectionPolicy, SelectedTarget, assert_workspace_id,
    handle_merge_with_events, open_merge_probe, resolve_targets, resolve_workspace_root,
};

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

/// The source selector the import library receives, qualified once
/// (design §6.1): no ref, or `HEAD`, is the source's `HEAD` commit; a name
/// already under `refs/` is used verbatim; any other name is a branch,
/// `refs/heads/<name>`. The selector reaches the port unchanged for both
/// the capture and the fetch refspec, so a short name is never left for
/// libgit2 to guess at -- a tag is spelled `refs/tags/<name>`.
pub(crate) fn qualify_selector(source_ref: Option<&str>) -> SourceSelector {
    match source_ref.map(str::trim) {
        None | Some("HEAD") => SourceSelector::Head,
        Some(name) if name.starts_with("refs/") => SourceSelector::Ref(name.to_owned()),
        Some(name) => SourceSelector::Ref(format!("refs/heads/{name}")),
    }
}

/// The receivers this merge selected, by identity, exactly as the engine's
/// planner selects them (`workspace_ops::merge::plan`): the verb's default
/// is every active member, and the root joins only when the selection names
/// `@root` explicitly -- `@all` alone never selects it.
pub(crate) fn selected_keys(
    manifest: &ManifestArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<RepoKey>> {
    let targets = resolve_targets(
        manifest,
        selection,
        CommandDefaultTargets::Members,
        RootSelectionPolicy::Allow,
    )?;
    let explicitly_selected_root = selection.is_some_and(|selection| {
        selection
            .member_ids
            .iter()
            .chain(&selection.paths)
            .chain(&selection.targets)
            .any(|target| target == "@root")
    });
    Ok(targets
        .into_iter()
        .filter_map(|target| match target {
            SelectedTarget::Member(member) => Some(RepoKey::Member {
                id: member.id.clone(),
            }),
            SelectedTarget::Root => explicitly_selected_root.then_some(RepoKey::Root),
        })
        .collect())
}

/// Every repository a workspace's lock records, in lock order, plus its
/// root: the participant set the import pairs by member id (design §6).
pub(crate) fn participants_of(root: &Path, lock: &LockArtifact) -> Vec<Participant> {
    lock.members
        .iter()
        .map(|(id, entry)| {
            Participant::new(
                RepoKey::Member { id: id.clone() },
                entry.path.clone(),
                root.join(&entry.path),
            )
        })
        .chain(std::iter::once(Participant::root(root)))
        .collect()
}

/// Member ids both locks record under different source identities: the
/// same id names a different repository on the two sides, which the
/// pairing by id alone cannot see.
pub(crate) fn identity_mismatches(
    receiver: &LockArtifact,
    source: &LockArtifact,
) -> Vec<(String, String, String)> {
    receiver
        .members
        .iter()
        .filter_map(|(id, entry)| {
            let other = source.members.get(id)?;
            match (&entry.source_id, &other.source_id) {
                (Some(here), Some(there)) if here != there => {
                    Some((id.clone(), here.clone(), there.clone()))
                }
                _ => None,
            }
        })
        .collect()
}

/// A retained import ref, named for a message: `<key> <ref> = <id>`.
fn retained_clause(effects: &[ImportEffect]) -> String {
    if effects.is_empty() {
        return "no import ref was created; nothing was written".to_owned();
    }
    let refs: Vec<String> = effects
        .iter()
        .map(|effect| match effect {
            ImportEffect::RefCreated {
                key,
                import_ref,
                oid,
            } => format!("{key} {import_ref} = {}", oid.to_hex()),
        })
        .collect();
    format!(
        "retained import refs (ordinary Git refs, never pruned by gwz): {}",
        refs.join(", ")
    )
}

/// An import failure as a `ModelError`: the code follows the typed cause
/// (`errors::import_error_code`), the message names the step, the source,
/// the import name, the cause, every retained import ref and the fact that
/// the engine was not entered.
fn import_error(
    what: &str,
    source: &BoundMember,
    selector: &SourceSelector,
    import_ref: &str,
    error: &ImportError,
) -> ModelError {
    ModelError::new(
        errors::import_error_code(error),
        format!(
            "{what}: import of {} from `{}` as {import_ref} failed: {error}; {}; the merge engine \
             was not entered; a retry mints a fresh transfer id",
            describe_selector(selector),
            source.name,
            retained_clause(error.effects())
        ),
    )
}

fn describe_selector(selector: &SourceSelector) -> String {
    match selector {
        SourceSelector::Head => "HEAD".to_owned(),
        SourceSelector::Ref(name) => name.clone(),
    }
}

fn canonical(path: &Path) -> ModelResult<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        ModelError::new(
            ErrorCode::IoError,
            format!("{} does not resolve: {error}", path.display()),
        )
    })
}

/// Everything step 5 produced: the family lock (held until the delegation
/// returns), the request the engine receives, and the clauses that name the
/// import in the response or after an engine refusal.
struct Prepared {
    session: <gwz_family_store::YamlFamilyStore as FamilyStore>::Session,
    projected: crate::MergeRequest,
    summary: String,
    retained: String,
}

/// Steps 1-5.
fn prepare<B: MergeAuthorityBackend>(
    backend: &B,
    start: &Path,
    request: &crate::MergeRequest,
) -> ModelResult<Prepared> {
    let selector = validate_family_merge(request)?;
    let what = format!("local family merge from `{}`", selector.token.as_str());
    let store_error = |error: &StoreError| errors::store_in(&what, error);
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let store = family_store();
    let observation = store
        .read_view(&FamilyLocation::new(&root))
        .map_err(|error| store_error(&error))?;
    let bound = resolve_family_merge(observation.view(), &selector.token)?;
    let FamilyObservation::Family {
        root: family_root, ..
    } = &observation
    else {
        return Err(ModelError::new(
            ErrorCode::InternalError,
            format!("{what}: `{}` bound outside any family", bound.name),
        ));
    };
    let family_root = canonical(family_root)?;

    // The receiving side, read-only and before the lock: what the engine's
    // planner will read again after the import, checked here so a start it
    // would refuse before planning refuses before any fetch (design §6.2).
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let receiver_lock = artifact::read_lock(&root)?;
    let selected = selected_keys(&manifest, request.meta.selection.as_ref())?;
    if let Some(merge_id) = open_merge_probe(&root)? {
        return Err(ModelError::new(
            ErrorCode::OpenOperation,
            format!(
                "{what}: merge '{merge_id}' is open in this workspace; use merge status, merge \
                 continue, or merge abort; nothing was imported"
            ),
        ));
    }

    // Step 5: the family lock, held from here through the delegation.
    let mut session = store
        .try_lock(&FamilyLocation::new(&root))
        .map_err(|error| store_error(&error))?;
    let locked_view = session.reread().map_err(|error| store_error(&error))?;
    let bound = resolve_family_merge(locked_view.as_ref(), &selector.token)?;
    let source_root = if bound.is_root {
        family_root.clone()
    } else {
        canonical(&family_root.join(&bound.path))?
    };
    let source_lock = artifact::read_lock(&source_root).map_err(|error| {
        ModelError::new(
            error.code,
            format!(
                "{what}: the lock of `{}` at {}: {}; nothing was imported",
                bound.name,
                source_root.display(),
                error.message
            ),
        )
    })?;
    let transfer = mint_transfer_id()?;
    let import_ref = transfer.import_ref();
    let import_request = ImportRequest {
        transfer,
        receivers: participants_of(&root, &receiver_lock),
        sources: participants_of(&source_root, &source_lock),
        selected,
        selector: qualify_selector(selector.source_ref.as_deref()),
    };
    let refuse = |error: &ImportError| {
        import_error(&what, &bound, &import_request.selector, &import_ref, error)
    };
    // Pairing first (pure: no transport call), so a set mismatch is reported
    // whole; then the identity the pairing by id cannot see.
    pair_participants(&import_request).map_err(|error| refuse(&error))?;
    let mismatched = identity_mismatches(&receiver_lock, &source_lock);
    if !mismatched.is_empty() {
        let detail: Vec<String> = mismatched
            .iter()
            .map(|(id, here, there)| format!("{id} records source {here} here and {there} there"))
            .collect();
        return Err(ModelError::new(
            ErrorCode::PairingMismatch,
            format!(
                "{what}: import pairing is incomplete; {}; refused before any fetch; nothing \
                 was written",
                detail.join("; ")
            ),
        ));
    }
    let mut transport = BackendLocalTransport::new(backend);
    let imported = prepare_import(&import_request, &mut transport, &NeverCancelled)
        .map_err(|error| refuse(&error))?;

    // Step 6's request: the selector cleared, the common import ref as
    // `source_ref`; everything else exactly as the driver sent it.
    let projected = crate::MergeRequest {
        local_source_name: None,
        source_ref: Some(imported.import_ref.clone()),
        ..request.clone()
    };
    let summary = summary(&bound, &import_request.selector, &imported);
    let retained = retained_clause(&effects_of(&imported));
    Ok(Prepared {
        session,
        projected,
        summary,
        retained,
    })
}

/// The verified import as effects, for the clause an engine refusal carries.
fn effects_of(imported: &ImportedSource) -> Vec<ImportEffect> {
    imported
        .vector
        .iter()
        .map(|commit| ImportEffect::RefCreated {
            key: commit.key.clone(),
            import_ref: imported.import_ref.clone(),
            oid: commit.oid.clone(),
        })
        .collect()
}

/// One line for the response envelope: the source, the selector, the
/// import name and every receiver's captured id.
fn summary(source: &BoundMember, selector: &SourceSelector, imported: &ImportedSource) -> String {
    let ids: Vec<String> = imported
        .vector
        .iter()
        .map(|commit| format!("{}={}", commit.key, commit.oid.to_hex()))
        .collect();
    format!(
        "imported {} of family member `{}` as {} ({}); the import ref is retained in every \
         receiver and never pruned by gwz",
        describe_selector(selector),
        source.name,
        imported.import_ref,
        ids.join(", ")
    )
}

/// The engine's sink with its `OperationStarted` withheld: the wrapper has
/// already emitted that event for this operation, before the observation
/// and the import, and the engine numbers its own events from the same
/// origin, so the stream a driver sees is one operation's.
struct AfterStarted<'a> {
    inner: &'a dyn EventSink,
    withheld: AtomicBool,
}

impl EventSink for AfterStarted<'_> {
    fn deliver(&self, event: crate::OperationEvent) {
        if event.kind == crate::EventKind::OperationStarted
            && !self.withheld.swap(true, Ordering::SeqCst)
        {
            return;
        }
        self.inner.deliver(event);
    }
}

pub(crate) fn handle<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: String,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse>
where
    B: MergeAuthorityBackend,
{
    let context = OperationRequest::Merge(request.clone()).context(operation_id.clone())?;
    let emitter = EventEmitter::new(&context, events, 0);
    emitter.operation_started();
    let prepared = match prepare(backend, start, &request) {
        Ok(prepared) => prepared,
        Err(error) => {
            emitter.operation_finished();
            return Err(error);
        }
    };
    let Prepared {
        session,
        projected,
        summary,
        retained,
    } = prepared;
    // Step 6: one delegation to the public engine entry, under the family
    // lock still; the engine takes its own workspace lock (design §3.2).
    let sink = AfterStarted {
        inner: events,
        withheld: AtomicBool::new(false),
    };
    let result = handle_merge_with_events(backend, start, projected, operation_id, &sink);
    drop(session);
    match result {
        Ok(mut response) => {
            if response.response.meta.message.is_none() {
                response.response.meta.message = Some(summary);
            }
            Ok(response)
        }
        Err(mut error) => {
            // The engine's refusal travels unchanged; what the import left
            // behind is named after it (design §6.2: engine-state failures
            // after import leave retained refs).
            error.message = format!("{}; {retained}", error.message);
            Err(error)
        }
    }
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

    /// Design §6.1: `merge --remote A` is A's HEAD, `merge --remote A <ref>`
    /// is `<ref>` resolved in A -- qualified once, here, so the port sees a
    /// full name for both the capture and the refspec.
    #[test]
    fn the_source_selector_is_qualified_once() {
        assert_eq!(qualify_selector(None), SourceSelector::Head);
        assert_eq!(qualify_selector(Some("HEAD")), SourceSelector::Head);
        assert_eq!(qualify_selector(Some(" HEAD ")), SourceSelector::Head);
        assert_eq!(
            qualify_selector(Some("lane/agent-17")),
            SourceSelector::Ref("refs/heads/lane/agent-17".to_owned())
        );
        assert_eq!(
            qualify_selector(Some("refs/heads/main")),
            SourceSelector::Ref("refs/heads/main".to_owned())
        );
        assert_eq!(
            qualify_selector(Some("refs/tags/v1")),
            SourceSelector::Ref("refs/tags/v1".to_owned())
        );
        assert_eq!(
            qualify_selector(Some("refs/gwz/local-imports/xfer_1")),
            SourceSelector::Ref("refs/gwz/local-imports/xfer_1".to_owned())
        );
    }

    /// The message of a refused import names every retained ref, or says
    /// that nothing was written, and never claims the engine ran.
    #[test]
    fn a_refused_import_names_what_it_left_behind() {
        use gwz_local_import::{MovedMember, SourceProblem};
        use gwz_repo_contract::{ObjectFormat, ObjectId};
        let app = RepoKey::Member {
            id: "mem_app".to_owned(),
        };
        let bound = BoundMember {
            name: "A".to_owned(),
            path: "../ws-A".to_owned(),
            kind: MemberKind::Checkout,
            is_root: false,
        };
        let import_ref = "refs/gwz/local-imports/xfer_1";
        let oid = ObjectId::parse_hex(ObjectFormat::Sha1, &"ab".repeat(20)).unwrap();

        let pairing = import_error(
            "local family merge from `A`",
            &bound,
            &SourceSelector::Head,
            import_ref,
            &ImportError::PairingIncomplete {
                missing: vec![RepoKey::Member {
                    id: "mem_lib".to_owned(),
                }],
                moved: vec![MovedMember {
                    key: app.clone(),
                    receiver_path: "app".to_owned(),
                    source_path: "src/app".to_owned(),
                }],
            },
        );
        assert_eq!(pairing.code, ErrorCode::PairingMismatch);
        assert!(pairing.message.contains("mem_lib"), "{}", pairing.message);
        assert!(
            pairing
                .message
                .contains("mem_app is at app here and src/app there"),
            "{}",
            pairing.message
        );
        assert!(
            pairing
                .message
                .contains("no import ref was created; nothing was written"),
            "{}",
            pairing.message
        );
        assert!(
            pairing.message.contains("the merge engine was not entered"),
            "{}",
            pairing.message
        );

        let missing = import_error(
            "local family merge from `A`",
            &bound,
            &SourceSelector::Ref("refs/heads/lane/x".to_owned()),
            import_ref,
            &ImportError::SourceMissing {
                missing: vec![SourceProblem {
                    key: app.clone(),
                    detail: "ref refs/heads/lane/x does not resolve".to_owned(),
                }],
            },
        );
        assert_eq!(missing.code, ErrorCode::MergeValidationFailed);
        assert!(
            missing
                .message
                .contains("import of refs/heads/lane/x from `A`"),
            "{}",
            missing.message
        );

        let partial = import_error(
            "local family merge from `A`",
            &bound,
            &SourceSelector::Head,
            import_ref,
            &ImportError::TransferFailed {
                key: RepoKey::Member {
                    id: "mem_lib".to_owned(),
                },
                detail: "fetch failed".to_owned(),
                effects: vec![ImportEffect::RefCreated {
                    key: app,
                    import_ref: import_ref.to_owned(),
                    oid: oid.clone(),
                }],
            },
        );
        assert_eq!(partial.code, ErrorCode::ImportIncomplete);
        assert!(
            partial.message.contains(&format!(
                "retained import refs (ordinary Git refs, never pruned by gwz): mem_app \
                 {import_ref} = {}",
                oid.to_hex()
            )),
            "{}",
            partial.message
        );
        assert!(
            partial
                .message
                .contains("a retry mints a fresh transfer id"),
            "{}",
            partial.message
        );
    }
}
