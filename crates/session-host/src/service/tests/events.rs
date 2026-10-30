use super::*;

#[test]
fn a_submit_is_accepted_its_events_read_before_its_result_and_then_released() {
    let mut host = Host::new(Limits::default());
    let submit = host.call("fetch", true, "hold-s");
    assert_eq!(host.sink.wait(submit), Ok("accepted op-1".to_owned()));
    host.wait_ran("hold-s");
    let read = host.call("events.subscribe", false, "op-1 0 5000");
    assert_eq!(
        host.sink.wait(read),
        Ok("events next=1 complete=false [1=started hold-s]".to_owned())
    );
    let result = host.call("operation.result", false, "op-1");
    let early_release = host.call("operation.release", false, "op-1");
    assert!(
        host.sink
            .wait(early_release)
            .unwrap_err()
            .starts_with("OpenOperation")
    );
    assert!(!host.sink.has(result), "the result waits for the terminal");
    // A held read is answered by the next append.
    let held = host.call("events.subscribe", false, "op-1 1 5000");
    host.script.release("hold-s");
    assert_eq!(host.sink.wait(result), Ok("result hold-s".to_owned()));
    let held = host.sink.wait(held).unwrap();
    assert!(held.contains("2=finished hold-s"), "{held}");
    let response = host.call("operation.response", false, "op-1");
    assert_eq!(host.sink.wait(response), Ok("done fetch hold-s".to_owned()));
    let release = host.call("operation.release", false, "op-1");
    assert_eq!(host.sink.wait(release), Ok("released".to_owned()));
    let again = host.call("operation.release", false, "op-1");
    assert!(
        host.sink
            .wait(again)
            .unwrap_err()
            .starts_with("OperationExpired")
    );
    let never = host.call("operation.result", false, "op-9");
    assert!(
        host.sink
            .wait(never)
            .unwrap_err()
            .starts_with("OperationNotFound")
    );
}

#[test]
fn a_held_read_returns_empty_when_its_wait_elapses() {
    let mut host = Host::new(Limits::default());
    let submit = host.call("fetch", true, "hold-w");
    host.sink.wait(submit).unwrap();
    host.wait_ran("hold-w");
    let read = host.call("events.subscribe", false, "op-1 1 50");
    assert_eq!(
        host.sink.wait(read),
        Ok("events next=1 complete=false []".to_owned())
    );
    host.script.release("hold-w");
}

#[test]
fn an_event_log_keeps_a_reset_for_the_history_it_dropped() {
    let signal = Arc::new(ReadSignal::default());
    let log = EventLog::new(3, b"reset".to_vec(), signal);
    for index in 1..=5 {
        log.push(format!("e{index}").into_bytes());
    }
    let read = EventsRead {
        operation_id: "op-1".to_owned(),
        stream_id: "s".to_owned(),
        cursor: None,
        max_records: None,
        max_bytes: None,
        wait: Duration::ZERO,
    };
    let batch = log.read(&read, 1024);
    let payloads: Vec<String> = batch.records.iter().map(|(_, p)| text(p)).collect();
    assert_eq!(payloads, ["reset", "e4", "e5"]);
    assert_eq!(batch.next_cursor, 5);
    assert!(!batch.complete);
    log.complete();
    let after = EventsRead {
        cursor: Some(5),
        ..read
    };
    assert!(log.read(&after, 1024).complete);
}
