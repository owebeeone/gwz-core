//! What only a tree-aware owner can be asked: a helper whose leader has exited is not retired while a member of
//! its job is alive (Windows learns that from the job; Unix cannot), and `reap_ready` ends such a member.
use super::*;
use crate::git::endpoint::https_auth::helper_fixture::{Behavior, Fixture};
use owner::PendingChild;

#[tokio::test]
async fn a_retained_tree_with_a_live_member_keeps_its_slot_and_reap_ready_ends_it() {
    let fixture = Fixture::new(Behavior::ExitsLeavingDescendantOnOutput);
    let owner = AuthOwner::new(HelperSlots::new());
    let permits = Arc::new(owner::AdmissionPermits {
        _helper_slot: owner
            .inner
            .helper_slots
            .0
            .clone()
            .acquire_owned()
            .await
            .unwrap(),
        _endpoint_slot: None,
    });
    let command = process_tree::HelperCommand {
        program: fixture.config.executable.clone(),
        args: fixture.argv().iter().map(OsString::from).collect(),
        directory: environment::working_directory().unwrap(),
        environment: environment::Environment::snapshot(&fixture.config.environment).into_pairs(),
    };
    let (mut child, tree) = process_tree::spawn(&command).unwrap();
    // The leader exits at once; its descendant keeps running and is a member of the job. Nothing has ended it.
    child.wait().await.unwrap();
    fixture.started().await;
    owner.retain_pending(PendingChild {
        child,
        tree: Some(tree),
        _permits: permits,
    });
    owner.reap_ready();
    assert_eq!(
        owner.inner.helper_slots.available(),
        7,
        "a slot must not be released while a member of the helper's tree is alive"
    );
    assert_eq!(owner.pending_cleanup_count(), 1);
    // reap_ready has ended the member, so that the tree drains and the next pass releases the slot.
    let until = Instant::now() + Duration::from_secs(10);
    while owner.pending_cleanup_count() > 0 {
        assert!(Instant::now() < until, "reap_ready never ended the retained tree");
        owner.reap_ready();
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(owner.inner.helper_slots.available(), 8);
    fixture
        .assert_stopped("the retained tree's member must have been ended")
        .await;
}
