use super::*;

#[test]
fn a_unary_call_replies_with_its_final_response_and_a_direct_call_on_a_direct_worker() {
    let mut host = Host::new(Limits::default());
    let fetch = host.call("fetch", false, "f1");
    assert_eq!(host.sink.wait(fetch), Ok("done fetch f1".to_owned()));
    let status = host.call("status", true, "s1");
    // A direct method replies with its final response, whatever its mode.
    assert_eq!(host.sink.wait(status), Ok("done status s1".to_owned()));
}

#[test]
fn a_cancel_before_admission_settles_the_call_before_resolution_runs_it() {
    let mut host = Host::new(Limits::default());
    // The first call's resolution hangs; the second waits behind it.
    let slow = host.call("commit", false, "slow-1");
    let second = host.call("commit", false, "c2");
    let cancel = host.call("operation.cancel", false, &second.to_string());
    // The cancel was read while admission was stuck in resolution.
    assert!(!host.sink.has(second));
    host.script.release("resolve slow-1");
    assert!(host.sink.wait(second).unwrap_err().starts_with("Cancelled"));
    assert_eq!(host.sink.wait(cancel), Ok("cleanup 0 false".to_owned()));
    assert_eq!(host.sink.lane(cancel), (Lane::Control, true));
    assert!(
        !host.ran("c2"),
        "a call cancelled before admission never runs"
    );
    assert_eq!(host.sink.wait(slow), Ok("done commit slow-1".to_owned()));
}

#[test]
fn cancel_reaches_a_running_handler_and_answers_once_it_is_terminal() {
    let mut host = Host::new(Limits::default());
    let fetch = host.call("fetch", false, "hold-f");
    host.wait_ran("hold-f");
    let cancel = host.call("operation.cancel", false, &fetch.to_string());
    assert_eq!(
        host.sink.wait(fetch),
        Err("Cancelled: cancelled".to_owned())
    );
    assert_eq!(host.sink.wait(cancel), Ok("cleanup 0 true".to_owned()));
    // A spent unary call's record is gone.
    let late = host.call("operation.cancel", false, &fetch.to_string());
    assert!(
        host.sink
            .wait(late)
            .unwrap_err()
            .starts_with("OperationExpired")
    );
    let unseen = host.call("operation.cancel", false, "99");
    assert!(
        host.sink
            .wait(unseen)
            .unwrap_err()
            .starts_with("OperationNotFound")
    );
}

#[test]
fn a_duplicate_live_request_id_a_failed_resolution_and_a_panic_are_settled() {
    let mut host = Host::new(Limits::default());
    host.call("fetch", false, "hold-dup");
    host.wait_ran("hold-dup");
    let duplicate = host.call("fetch", false, "hold-dup");
    assert!(
        host.sink
            .wait(duplicate)
            .unwrap_err()
            .starts_with("InvalidRequest")
    );
    let unresolved = host.call("commit", false, "unresolvable");
    assert_eq!(
        host.sink.wait(unresolved),
        Err("WorkspaceNotFound: no workspace".to_owned())
    );
    // A write waits for the fetch, so release it first.
    host.script.release("hold-dup");
    let panicked = host.call("commit", false, "panic");
    assert!(
        host.sink
            .wait(panicked)
            .unwrap_err()
            .starts_with("Internal")
    );
    let bad = host.call("bad", false, "x");
    assert!(
        host.sink
            .wait(bad)
            .unwrap_err()
            .starts_with("InvalidRequest")
    );
}
