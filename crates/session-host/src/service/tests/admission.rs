use super::*;

#[test]
fn a_write_waits_for_the_running_write_and_fetches_run_side_by_side() {
    let mut host = Host::new(Limits::default());
    host.call("commit", false, "hold-w1");
    host.wait_ran("hold-w1");
    let second = host.call("stage", false, "w2");
    let fetch = host.call("fetch", false, "f-beside");
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !host.ran("w2"),
        "a write runs beside nothing but direct calls"
    );
    assert!(!host.ran("f-beside"), "a fetch does not run beside a write");
    let status = host.call("status", false, "s");
    assert_eq!(host.sink.wait(status), Ok("done status s".to_owned()));
    host.script.release("hold-w1");
    host.sink.wait(second).unwrap();
    host.sink.wait(fetch).unwrap();

    host.call("fetch", false, "hold-a");
    host.call("fetch", false, "hold-b");
    host.wait_ran("hold-a");
    host.wait_ran("hold-b");
    host.call("push", false, "hold-p1");
    host.wait_ran("hold-p1");
    let push2 = host.call("push", false, "p2");
    std::thread::sleep(Duration::from_millis(50));
    assert!(!host.ran("p2"), "at most one push runs per workspace");
    for request in ["hold-a", "hold-b", "hold-p1"] {
        host.script.release(request);
    }
    host.sink.wait(push2).unwrap();
}

#[test]
fn the_queue_the_table_and_the_outstanding_calls_are_bounded() {
    let limits = Limits {
        running_operations: 1,
        queued_operations: 1,
        operation_table: 2,
        outstanding_calls: 4,
        control_reserve: 1,
        ..Limits::default()
    };
    let mut host = Host::new(limits);
    host.call("fetch", false, "hold-1");
    host.wait_ran("hold-1");
    host.call("fetch", false, "q");
    let full = host.call("fetch", false, "over");
    assert!(host.sink.wait(full).unwrap_err().starts_with("SessionFull"));
    // Calls one and two are outstanding; two more fit, a fifth does not.
    host.call("operation.result", false, "op-1");
    host.call("operation.result", false, "op-2");
    let fifth = host.call("status", false, "s");
    assert!(
        host.sink
            .wait(fifth)
            .unwrap_err()
            .starts_with("SessionFull")
    );
    // The control reserve is separate from the outstanding calls.
    let control = host.call("operation.cancel", false, "1");
    let control2 = host.call("operation.cancel", false, "2");
    assert!(
        host.sink
            .wait(control2)
            .unwrap_err()
            .starts_with("SessionFull")
    );
    host.sink.wait(control).unwrap();
    host.script.release("hold-1");
}

#[test]
fn a_full_table_evicts_the_oldest_delivered_terminal_record() {
    let limits = Limits {
        running_operations: 1,
        queued_operations: 1,
        operation_table: 2,
        ..Limits::default()
    };
    let mut host = Host::new(limits);
    for request in ["a", "b"] {
        let submit = host.call("fetch", true, request);
        host.sink.wait(submit).unwrap();
    }
    for operation in ["op-1", "op-2"] {
        let result = host.call("operation.result", false, operation);
        host.sink.wait(result).unwrap();
    }
    // A fetch declares one view, its result: op-1 and op-2 are delivered,
    // and the third evicts op-1, the oldest.
    let third = host.call("fetch", true, "c");
    assert_eq!(host.sink.wait(third), Ok("accepted op-3".to_owned()));
    let gone = host.call("operation.result", false, "op-1");
    assert!(
        host.sink
            .wait(gone)
            .unwrap_err()
            .starts_with("OperationExpired")
    );
}

/// S-P2-3 (+ C-P3-2): a record is delivered only once every view its method
/// declares has been handed over, so it is never evicted before.
#[test]
fn a_record_is_evicted_only_once_every_declared_view_is_delivered() {
    let limits = Limits {
        running_operations: 1,
        queued_operations: 1,
        operation_table: 2,
        ..Limits::default()
    };
    // A merge declares its result and its response; only its result is read.
    let mut host = Host::new(limits.clone());
    let merge = host.call("merge", true, "m");
    host.sink.wait(merge).unwrap();
    let other = host.call("fetch", true, "f");
    host.sink.wait(other).unwrap();
    for operation in ["op-1", "op-2"] {
        let result = host.call("operation.result", false, operation);
        host.sink.wait(result).unwrap();
    }
    let third = host.call("fetch", true, "g");
    // op-2 (a fetch whose result was read) is evictable; op-1 is not.
    assert_eq!(host.sink.wait(third), Ok("accepted op-3".to_owned()));
    let fourth = host.call("fetch", true, "h");
    assert!(
        host.sink
            .wait(fourth)
            .unwrap_err()
            .starts_with("SessionFull")
    );
    let response = host.call("operation.response", false, "op-1");
    assert_eq!(host.sink.wait(response), Ok("done merge m".to_owned()));

    // A fetch whose response alone was read is not delivered: its result is
    // the view it declares.
    let mut host = Host::new(limits);
    for request in ["a", "b"] {
        let submit = host.call("fetch", true, request);
        host.sink.wait(submit).unwrap();
        let operation = format!("op-{}", if request == "a" { 1 } else { 2 });
        let response = host.call("operation.response", false, &operation);
        host.sink.wait(response).unwrap();
    }
    let third = host.call("fetch", true, "c");
    assert!(
        host.sink
            .wait(third)
            .unwrap_err()
            .starts_with("SessionFull")
    );
    let result = host.call("operation.result", false, "op-1");
    assert_eq!(host.sink.wait(result), Ok("result a".to_owned()));
}

/// A result delivered from a worker's thread is delivered as soon as the
/// client can read it: the client's next submit, sent on reading that reply,
/// may evict the record even while the sender is still inside its send.
#[test]
fn a_view_is_delivered_before_the_client_can_read_it() {
    let limits = Limits {
        running_operations: 1,
        queued_operations: 1,
        operation_table: 2,
        ..Limits::default()
    };
    let mut host = Host::new(limits);
    let first = host.call("fetch", true, "hold-a");
    host.sink.wait(first).unwrap();
    host.wait_ran("hold-a");
    let second = host.call("fetch", true, "b");
    host.sink.wait(second).unwrap();
    // The result is asked while op-1 runs, so the worker's thread answers it.
    let result = host.call("operation.result", false, "op-1");
    *crate::lock(&host.sink.parked) = Some(result);
    host.script.release("hold-a");
    assert_eq!(host.sink.wait(result), Ok("result hold-a".to_owned()));
    // The table holds op-1, whose result the client has, and op-2, which has
    // not been read: op-1 is the one to evict for op-3.
    let third = host.call("fetch", true, "c");
    let admitted = host.sink.wait(third);
    host.sink.resume();
    assert_eq!(admitted, Ok("accepted op-3".to_owned()));
}
