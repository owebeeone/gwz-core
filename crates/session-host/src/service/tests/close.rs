use super::*;

#[test]
fn close_settles_cancels_detaches_and_reports_and_the_detached_record_refuses_writes() {
    let limits = Limits {
        running_operations: 1,
        close_wait: Duration::from_millis(100),
        ..Limits::default()
    };
    let detached = DetachedWorkers::new();
    let mut host = Host::shared(limits.clone(), detached.clone());
    let stubborn = host.call("commit", false, "hold-stubborn");
    host.wait_ran("hold-stubborn");
    let queued = host.call("fetch", false, "queued");
    let close = host.call("session.close", false, "");
    let after = host.call("status", false, "after");
    assert!(
        host.sink
            .wait(after)
            .unwrap_err()
            .contains("session closing")
    );
    assert!(host.sink.wait(queued).unwrap_err().starts_with("Cancelled"));
    assert!(host.sink.wait(stubborn).unwrap_err().contains("detached"));
    assert_eq!(host.sink.wait(close), Ok("cleanup 1 false".to_owned()));
    // The close report is ordinary: exempt from priority.
    assert_eq!(host.sink.lane(close), (Lane::Control, false));
    assert!(host.service.wait_ended(Duration::from_secs(5)));
    assert!(*crate::lock(&host.sink.closed));
    assert!(!host.ran("queued"));

    // Another session of the same host: a write or push beside the detached
    // worker is refused; a fetch is not.
    assert!(detached.running_on("/ws"));
    let mut other = Host::shared(Limits::default(), detached.clone());
    let write = other.call("commit", false, "w");
    assert!(
        other
            .sink
            .wait(write)
            .unwrap_err()
            .starts_with("WorkspaceBusy")
    );
    let push = other.call("push", false, "p");
    assert!(
        other
            .sink
            .wait(push)
            .unwrap_err()
            .starts_with("WorkspaceBusy")
    );
    let fetch = other.call("fetch", false, "f");
    other.sink.wait(fetch).unwrap();
    host.script.release("hold-stubborn");
    let deadline = Instant::now() + Duration::from_secs(20);
    while detached.running_on("/ws") {
        assert!(Instant::now() < deadline, "the detached worker never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn channel_closure_cancels_running_work_without_replies() {
    let mut host = Host::new(Limits::default());
    let fetch = host.call("fetch", false, "hold-c");
    host.wait_ran("hold-c");
    host.service.channel_closed(Ending::Channel);
    assert!(host.service.wait_ended(Duration::from_secs(5)));
    // The worker saw its token cancelled; its terminal was still settled.
    assert_eq!(
        host.sink.wait(fetch),
        Err("Cancelled: cancelled".to_owned())
    );
}

/// One delayed-sink run: a unary call's terminal reply is held in the sink
/// while `session.close` waits for its worker; the reply must reach the sink
/// before the close report, and the channel close comes last.
fn a_settled_reply_precedes_the_close_report(delay: Duration, yields: u32) {
    let mut host = Host::new(Limits::default());
    let fetch = host.call("fetch", false, "hold-order");
    host.wait_ran("hold-order");
    *crate::lock(&host.sink.slow) = Some((fetch, delay, yields));
    let close = host.call("session.close", false, "");
    assert!(host.service.wait_ended(Duration::from_secs(20)));
    let order = crate::lock(&host.sink.order).clone();
    let reply = order
        .iter()
        .position(|entry| *entry == format!("reply {fetch}"));
    let report = order
        .iter()
        .position(|entry| *entry == format!("reply {close}"));
    let closed = order.iter().position(|entry| entry == "closed");
    assert!(
        reply.is_some() && reply < report && report < closed,
        "order {order:?}"
    );
    assert_eq!(host.service.dropped_replies(), 0);
}

/// C-P2-3 = S-P2-2: sends keep the order of settlement; close waits for no
/// reply batch in flight before its report and the channel's close.
#[test]
fn a_settled_reply_is_never_overtaken_by_the_close_report() {
    a_settled_reply_precedes_the_close_report(Duration::from_millis(200), 0);
    for _ in 0..1000 {
        a_settled_reply_precedes_the_close_report(Duration::ZERO, 50);
    }
}

/// C-P2-4 = S-P2-1: a detached submitted operation's record does not keep the
/// workspace recorded once its worker has ended.
#[test]
fn a_detached_submitted_operation_releases_the_workspace_when_its_worker_ends() {
    let limits = Limits {
        close_wait: Duration::from_millis(100),
        ..Limits::default()
    };
    let detached = DetachedWorkers::new();
    let mut host = Host::shared(limits, detached.clone());
    let stubborn = host.call("commit", true, "hold-stubborn-submit");
    assert_eq!(host.sink.wait(stubborn), Ok("accepted op-1".to_owned()));
    host.wait_ran("hold-stubborn-submit");
    let close = host.call("session.close", false, "");
    assert_eq!(host.sink.wait(close), Ok("cleanup 1 false".to_owned()));
    assert!(detached.running_on("/ws"));
    host.script.release("hold-stubborn-submit");
    let deadline = Instant::now() + Duration::from_secs(20);
    while detached.running_on("/ws") {
        assert!(
            Instant::now() < deadline,
            "the detached record outlived its worker"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // `host` is still alive, holding the retained record.
    let mut other = Host::shared(Limits::default(), detached.clone());
    let commit = other.call("commit", false, "w-after");
    assert_eq!(
        other.sink.wait(commit),
        Ok("done commit w-after".to_owned())
    );
    drop(host);
}

/// S-P3-1: a write queued before another session detached a worker is
/// refused when it would start, not run beside the worker.
#[test]
fn a_queued_write_is_refused_when_a_worker_was_detached_before_it_starts() {
    let detached = DetachedWorkers::new();
    let mut queued = Host::shared(Limits::default(), detached.clone());
    queued.call("fetch", false, "hold-before");
    queued.wait_ran("hold-before");
    let write = queued.call("commit", false, "w-queued");
    let limits = Limits {
        close_wait: Duration::from_millis(50),
        ..Limits::default()
    };
    let mut closing = Host::shared(limits, detached.clone());
    closing.call("commit", false, "hold-stubborn-other");
    closing.wait_ran("hold-stubborn-other");
    let close = closing.call("session.close", false, "");
    closing.sink.wait(close).unwrap();
    assert!(detached.running_on("/ws"));
    queued.script.release("hold-before");
    assert!(
        queued
            .sink
            .wait(write)
            .unwrap_err()
            .starts_with("WorkspaceBusy")
    );
    assert!(!queued.ran("w-queued"));
    closing.script.release("hold-stubborn-other");
}
