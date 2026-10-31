//! The member lock manager of the host context through the served core (step 2.3c; core session design §5.1): two
//! sessions of one host context never run one member's step at once, a wait wakes on `operation.cancel`, and each
//! `CoreDispatch` arm that updates a member's refs from the network takes the host's manager.

use super::*;

/// What the member-step handlers of `LockingDispatch` saw, across every session that shares them.
#[derive(Default)]
struct Steps {
    active: AtomicUsize,
    peak: AtomicUsize,
    /// A step holding its member waits here until released.
    hold: AtomicBool,
    inside: AtomicBool,
    /// Steps whose wait for the member was refused (woken by a cancel).
    refused: AtomicUsize,
}

/// A `fetch` that takes the member lock through the session's host context, as `CoreDispatch` does, around a step
/// that takes a moment (or, with `hold`, until released); every other method is `CoreDispatch`'s.
struct LockingDispatch {
    core: CoreDispatch,
    steps: Arc<Steps>,
}

impl Dispatch for LockingDispatch {
    fn run(
        &self,
        method: &str,
        request: &[u8],
        start: &Path,
        worker: &Worker<ServedSession>,
        events: &dyn EventSink,
    ) -> ModelResult<Vec<u8>> {
        if method != "fetch" {
            return self.core.run(method, request, start, worker, events);
        }
        let request = crate::FetchRequest::decode(request).unwrap();
        let locks = super::super::dispatch::member_locks(worker)?;
        let _guard =
            crate::operation::MemberSteps::enter(&locks, start, "mem_app").inspect_err(|_| {
                self.steps.refused.fetch_add(1, Ordering::SeqCst);
            })?;
        let active = self.steps.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.steps.peak.fetch_max(active, Ordering::SeqCst);
        self.steps.inside.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(20);
        if self.steps.hold.load(Ordering::SeqCst) {
            while self.steps.hold.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
        } else {
            std::thread::sleep(Duration::from_millis(200));
        }
        self.steps.active.fetch_sub(1, Ordering::SeqCst);
        let response = crate::FetchResponse {
            response: crate::ResponseEnvelope {
                meta: crate::ResponseMeta {
                    request_id: request.meta.request_id,
                    action: crate::ActionKind::Fetch,
                    aggregate_status: crate::AggregateStatus::Ok,
                    ..crate::ResponseMeta::default()
                },
                members: Vec::new(),
                errors: Vec::new(),
            },
            repos: None,
        };
        Ok(crate::encode(&response.to_cbor()))
    }
}

fn locking(steps: &Arc<Steps>) -> Arc<dyn Dispatch> {
    Arc::new(LockingDispatch {
        core: CoreDispatch::new(crate::git::Git2Backend::new()),
        steps: Arc::clone(steps),
    })
}

/// Red row (step 2.3c): two sessions of one host context never update a member's refs at once.
#[test]
fn two_sessions_of_one_host_context_never_run_one_members_step_at_once() {
    let (_temp, root) = workspace();
    let host = HostContext::new();
    let steps = Arc::new(Steps::default());
    let mut first = Session::on(&host, &root, locking(&steps));
    let mut second = Session::on(&host, &root, locking(&steps));
    let (a, b) = (first.fetch("fetch-a", None), second.fetch("fetch-b", None));
    first.reply(a).unwrap();
    second.reply(b).unwrap();
    assert_eq!(steps.peak.load(Ordering::SeqCst), 1);
}

/// The control of the row above: sessions of two host contexts share no manager, so their steps do overlap.
#[test]
fn sessions_of_two_host_contexts_share_no_member_lock() {
    let (_temp, root) = workspace();
    let steps = Arc::new(Steps::default());
    let mut first = Session::on(&HostContext::new(), &root, locking(&steps));
    let mut second = Session::on(&HostContext::new(), &root, locking(&steps));
    let (a, b) = (first.fetch("fetch-a", None), second.fetch("fetch-b", None));
    first.reply(a).unwrap();
    second.reply(b).unwrap();
    assert_eq!(steps.peak.load(Ordering::SeqCst), 2);
}

/// Red row (step 2.3c): a member step waiting for another session's step wakes on `operation.cancel` and answers
/// `cancelled`, and the step it waited for finishes untouched.
#[test]
fn a_member_step_waiting_for_another_session_wakes_on_cancel() {
    let (_temp, root) = workspace();
    let host = HostContext::new();
    let steps = Arc::new(Steps::default());
    steps.hold.store(true, Ordering::SeqCst);
    let mut holder = Session::on(&host, &root, locking(&steps));
    let mut waiter = Session::on(&host, &root, locking(&steps));
    let held = holder.fetch("fetch-held", None);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !steps.inside.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the first step never ran");
        std::thread::sleep(Duration::from_millis(2));
    }
    let waiting = waiter.fetch("fetch-waiting", None);
    assert!(
        host.members().wait_for_waiters(1, Duration::from_secs(10)),
        "the second step is not waiting for the member"
    );
    assert_eq!(
        steps.active.load(Ordering::SeqCst),
        1,
        "the second step ran"
    );
    let cancel = waiter.cancel(waiting);
    let error = waiter.reply(waiting).unwrap_err();
    assert_eq!(error.code, crate::GwzErrorCode::Cancelled);
    waiter.reply(cancel).unwrap();
    // The handler's own wait ended with the cancel, while the other session still held the member.
    let deadline = Instant::now() + Duration::from_secs(10);
    while steps.refused.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "the wait did not wake on the cancel"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(steps.active.load(Ordering::SeqCst), 1);
    assert_eq!(steps.peak.load(Ordering::SeqCst), 1);
    steps.hold.store(false, Ordering::SeqCst);
    holder.reply(held).unwrap();
}

/// The served workspace with its root repository committed and an `origin` (an empty bare repository beside it), so
/// that a fetch or push of `@root` has a member step to take.
pub(super) fn rooted_workspace() -> (TempDir, PathBuf) {
    use crate::git::GitBackend;
    let (temp, root) = workspace();
    let remote = temp.path().join("root.git");
    git2::Repository::init_bare(&remote)
        .unwrap()
        .set_head("refs/heads/main")
        .unwrap();
    let backend = crate::git::Git2Backend::without_credential_helpers();
    backend
        .add_remote(&root, "origin", remote.to_str().unwrap())
        .unwrap();
    let mut config = git2::Repository::open(&root).unwrap().config().unwrap();
    config.set_str("user.name", "GWZ Test").unwrap();
    config.set_str("user.email", "gwz@example.invalid").unwrap();
    backend.stage_paths(&root, &["gwz.conf"]).unwrap();
    backend.commit(&root, "captured", false).unwrap();
    (temp, root)
}

pub(super) fn root_only() -> crate::Selection {
    crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    }
}

/// Waits until the call's handler has started its work (its first event) and is parked in the host's `members`
/// manager waiting for a member, so a cancel after it meets a handler that waits. The wait reads the manager's own
/// waiter count; a handler that never waits (a regression) fails here, not on a time window.
pub(super) fn handler_started(
    session: &mut Session,
    operation_id: &str,
    members: &MemberLockManager,
) {
    let read = session.read_events(operation_id, 0);
    session.reply(read).unwrap();
    assert!(
        members.wait_for_waiters(1, Duration::from_secs(10)),
        "the call's handler is not waiting for a member"
    );
}

/// Red row (step 2.3c): `CoreDispatch`'s fetch takes the member lock from its session's host context, so a step
/// waiting for the member another holder has wakes on `operation.cancel` and fails that member's row `cancelled`;
/// once the holder lets go the same call runs.
#[test]
fn core_dispatch_fetch_waits_on_the_hosts_member_lock_and_wakes_on_cancel() {
    let (_temp, root) = rooted_workspace();
    let host = HostContext::new();
    let holder = host.members().lock(&root, "@root", None).unwrap();
    let core: Arc<dyn Dispatch> = Arc::new(CoreDispatch::new(crate::git::Git2Backend::new()));
    let mut session = Session::on(&host, &root, core);
    let request = crate::FetchRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            ..meta(&root, "fetch-root")
        },
    };
    let fetch = session.send("fetch", None, crate::encode(&request.to_cbor()), Lane::Call);
    handler_started(&mut session, "op-1", host.members());
    let cancel = session.cancel(fetch);
    let response = crate::FetchResponse::decode(&session.reply(fetch).unwrap()).unwrap();
    session.reply(cancel).unwrap();
    let row = &response.response.members[0];
    assert_eq!(row.member_id, "@root");
    assert_eq!(row.status, crate::MemberStatus::Failed);
    assert_eq!(
        row.error.as_ref().map(|error| error.code),
        Some(crate::GwzErrorCode::Cancelled)
    );
    assert_eq!(host.members().held_count(), 1, "the holder lost its member");
    drop(holder);
    assert_eq!(host.members().held_count(), 0);
    let again = crate::FetchRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            ..meta(&root, "fetch-root-2")
        },
    };
    let call = session.send("fetch", None, crate::encode(&again.to_cbor()), Lane::Call);
    let response = crate::FetchResponse::decode(&session.reply(call).unwrap()).unwrap();
    assert_eq!(response.response.members[0].error, None);
    assert_eq!(host.members().held_count(), 0);
}

/// As above for push: its root step takes the host context's member lock.
#[test]
fn core_dispatch_push_waits_on_the_hosts_member_lock_and_wakes_on_cancel() {
    let (_temp, root) = rooted_workspace();
    let host = HostContext::new();
    let holder = host.members().lock(&root, "@root", None).unwrap();
    let core: Arc<dyn Dispatch> = Arc::new(CoreDispatch::new(crate::git::Git2Backend::new()));
    let mut session = Session::on(&host, &root, core);
    let push_request = |request_id: &str| crate::PushRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            ..meta(&root, request_id)
        },
        remote: None,
        refspec: None,
        remote_check: None,
    };
    let push = session.send(
        "push",
        None,
        crate::encode(&push_request("push-root").to_cbor()),
        Lane::Call,
    );
    handler_started(&mut session, "op-1", host.members());
    let cancel = session.cancel(push);
    let response = crate::PushResponse::decode(&session.reply(push).unwrap()).unwrap();
    session.reply(cancel).unwrap();
    let row = &response.response.members[0];
    assert_eq!(row.member_id, "@root");
    assert_eq!(row.status, crate::MemberStatus::Failed);
    assert_eq!(
        row.error.as_ref().map(|error| error.code),
        Some(crate::GwzErrorCode::Cancelled)
    );
    assert_eq!(host.members().held_count(), 1, "the holder lost its member");
    drop(holder);
    let again = session.send(
        "push",
        None,
        crate::encode(&push_request("push-root-2").to_cbor()),
        Lane::Call,
    );
    let response = crate::PushResponse::decode(&session.reply(again).unwrap()).unwrap();
    assert_eq!(response.response.members[0].status, crate::MemberStatus::Ok);
    assert_eq!(host.members().held_count(), 0);
}

/// A `pull_head` of the root alone under `sync`, held for the root's member lock by another holder: the pull waits,
/// `operation.cancel` wakes it, and it answers `cancelled` before its fetch ran; without the hold the same call
/// reaches its fetch and fails on the empty remote (`missing_remote`).
fn root_pull_waits_on_the_hosts_member_lock(sync: crate::SyncBehavior) {
    let (_temp, root) = rooted_workspace();
    let host = HostContext::new();
    let holder = host.members().lock(&root, "@root", None).unwrap();
    let core: Arc<dyn Dispatch> = Arc::new(CoreDispatch::new(crate::git::Git2Backend::new()));
    let mut session = Session::on(&host, &root, core);
    let pull = crate::PullHeadRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            policy: Some(crate::OperationPolicy {
                sync: Some(sync),
                ..Default::default()
            }),
            ..meta(&root, "pull-root")
        },
    };
    let call = session.send(
        "pull_head",
        None,
        crate::encode(&pull.to_cbor()),
        Lane::Call,
    );
    handler_started(&mut session, "op-1", host.members());
    let cancel = session.cancel(call);
    let error = session.reply(call).unwrap_err();
    session.reply(cancel).unwrap();
    assert_eq!(error.code, crate::GwzErrorCode::Cancelled, "{error:?}");
    assert_eq!(host.members().held_count(), 1, "the holder lost its member");
    drop(holder);
    let again = session.send(
        "pull_head",
        None,
        crate::encode(&pull.to_cbor()),
        Lane::Call,
    );
    let error = session.reply(again).unwrap_err();
    assert_eq!(
        error.code,
        crate::GwzErrorCode::MissingRemote,
        "the released pull should have reached its fetch: {error:?}"
    );
    assert_eq!(host.members().held_count(), 0);
}

/// Red row (step 2.3c): the root fetch of a `pull_head` takes the host context's member lock.
#[test]
fn core_dispatch_pull_head_root_fetch_waits_on_the_hosts_member_lock() {
    root_pull_waits_on_the_hosts_member_lock(crate::SyncBehavior::FfOnly);
}

/// As above for the merge plan's root fetch (`sync = merge`).
#[test]
fn core_dispatch_pull_head_root_merge_plan_fetch_waits_on_the_hosts_member_lock() {
    root_pull_waits_on_the_hosts_member_lock(crate::SyncBehavior::Merge);
}

/// Red row (step 2.3c): `CoreDispatch`'s tag push takes the host context's member lock for the root's transfer;
/// without the hold the same call pushes the tag.
#[test]
fn core_dispatch_tag_push_waits_on_the_hosts_member_lock_and_wakes_on_cancel() {
    let (_temp, root) = rooted_workspace();
    {
        let repository = git2::Repository::open(&root).unwrap();
        let head = repository.head().unwrap().peel_to_commit().unwrap();
        repository
            .tag_lightweight("v1", head.as_object(), false)
            .unwrap();
    }
    let host = HostContext::new();
    let holder = host.members().lock(&root, "@root", None).unwrap();
    let core: Arc<dyn Dispatch> = Arc::new(CoreDispatch::new(crate::git::Git2Backend::new()));
    let mut session = Session::on(&host, &root, core);
    let tag = |request_id: &str| crate::TagRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            ..meta(&root, request_id)
        },
        op: crate::TagOp::Push,
        name: Some("v1".to_owned()),
        ..Default::default()
    };
    let call = session.send(
        "tag",
        None,
        crate::encode(&tag("tag-root").to_cbor()),
        Lane::Call,
    );
    handler_started(&mut session, "op-1", host.members());
    let cancel = session.cancel(call);
    let error = session.reply(call).unwrap_err();
    session.reply(cancel).unwrap();
    assert_eq!(error.code, crate::GwzErrorCode::Cancelled, "{error:?}");
    assert_eq!(host.members().held_count(), 1, "the holder lost its member");
    drop(holder);
    let again = session.send(
        "tag",
        None,
        crate::encode(&tag("tag-root-2").to_cbor()),
        Lane::Call,
    );
    session.reply(again).unwrap();
    assert_eq!(host.members().held_count(), 0);
}

/// Red row (step 2.3c): `CoreDispatch`'s tag fetch takes the host context's member lock for the root's tag exchange.
/// A held member makes the call wait, and a cancel wakes it with `cancelled`. Without the hold the call goes straight
/// to the remote, which has no `origin` here, and ends in a different error.
#[test]
fn core_dispatch_tag_fetch_waits_on_the_hosts_member_lock_and_wakes_on_cancel() {
    let (_temp, root) = rooted_workspace();
    let host = HostContext::new();
    let holder = host.members().lock(&root, "@root", None).unwrap();
    let core: Arc<dyn Dispatch> = Arc::new(CoreDispatch::new(crate::git::Git2Backend::new()));
    let mut session = Session::on(&host, &root, core);
    let tag = |request_id: &str| crate::TagRequest {
        meta: crate::RequestMeta {
            selection: Some(root_only()),
            ..meta(&root, request_id)
        },
        op: crate::TagOp::Fetch,
        ..Default::default()
    };
    let call = session.send(
        "tag",
        None,
        crate::encode(&tag("tag-fetch-root").to_cbor()),
        Lane::Call,
    );
    handler_started(&mut session, "op-1", host.members());
    let cancel = session.cancel(call);
    let error = session.reply(call).unwrap_err();
    session.reply(cancel).unwrap();
    assert_eq!(error.code, crate::GwzErrorCode::Cancelled, "{error:?}");
    assert_eq!(host.members().held_count(), 1, "the holder lost its member");
    drop(holder);
    assert_eq!(host.members().held_count(), 0);
}
