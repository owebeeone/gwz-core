//! Remediation round 1 (S-1 of the State review, P2-1 of the Code review): the member lock is keyed by the workspace's
//! filesystem identity, so a fetch served through another spelling of the same workspace root waits for the member a
//! holder of the resolved spelling holds. Each row is a `CoreDispatch` fetch with real git: it answers `cancelled`
//! while the holder holds the member (a lock keyed by the spelling lets it finish, and the cancel then meets a
//! completed call).

use super::member_locks::{handler_started, root_only, rooted_workspace};
use super::*;

/// A fetch of `@root` served through `served` (a spelling of the workspace at `root`) while another holder holds the
/// `@root` member under the resolved spelling `root`: it waits, and `operation.cancel` fails its row `cancelled`.
fn fetch_waits_for_the_holder_through(root: &Path, served: &Path) {
    let host = HostContext::new();
    let holder = host.members().lock(root, "@root", None).unwrap();
    let core: Arc<dyn Dispatch> = Arc::new(CoreDispatch::new(crate::git::Git2Backend::new()));
    let mut session = Session::on(&host, served, core);
    let request = crate::FetchRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            ..meta(served, "fetch-spelling")
        },
    };
    let fetch = session.send("fetch", None, crate::encode(&request.to_cbor()), Lane::Call);
    handler_started(&mut session, "op-1", host.members());
    let cancel = session.cancel(fetch);
    let response = crate::FetchResponse::decode(&session.reply(fetch).unwrap()).unwrap();
    session.reply(cancel).unwrap();
    let row = &response.response.members[0];
    assert_eq!(row.member_id, "@root");
    assert_eq!(row.status, crate::MemberStatus::Failed, "{row:?}");
    assert_eq!(
        row.error.as_ref().map(|error| error.code),
        Some(crate::GwzErrorCode::Cancelled)
    );
    assert_eq!(host.members().held_count(), 1, "the holder lost its member");
    drop(holder);
    assert_eq!(host.members().held_count(), 0);
}

/// The control of the two rows below: the resolved spelling itself.
#[test]
fn core_dispatch_fetch_through_the_resolved_spelling_waits_for_the_holder() {
    let (_temp, root) = rooted_workspace();
    fetch_waits_for_the_holder_through(&root, &root);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Red row: a symlink to the workspace root is the same workspace, so the fetch served through it waits.
        #[test]
        fn core_dispatch_fetch_through_a_symlink_spelling_waits_for_the_holder() {
            let (temp, root) = rooted_workspace();
            let link = temp.path().join("ws-link");
            crate::test_support::symlink_spelling(&root, &link);
            fetch_waits_for_the_holder_through(&root, &link);
        }
    }
}

/// Red row: a case variant of the root is the same workspace on a volume that folds case (Windows, macOS APFS), so
/// the fetch served through it waits. On a case-sensitive volume the row says why and does not run.
#[test]
fn core_dispatch_fetch_through_a_case_variant_spelling_waits_for_the_holder() {
    let (_temp, root) = rooted_workspace();
    match crate::test_support::case_variant(&root) {
        Ok(variant) => fetch_waits_for_the_holder_through(&root, &variant),
        Err(reason) => eprintln!("SKIPPED ROW: {reason}"),
    }
}
