use super::*;

#[test]
fn call_ids_must_increase_and_nothing_is_read_before_the_session_opens() {
    let mut host = Host::new(Limits::default());
    host.call("status", false, "a");
    let again = host.service.receive(&body(1, "status", false, "b"));
    assert!(again.is_err(), "a repeated call_id is a protocol error");
    let undecodable = host.service.receive(b"no fields");
    assert!(undecodable.is_err());

    let sink = Arc::new(Sink::default()) as Arc<dyn ReplySink>;
    let closed = Service::new(
        Ports {
            script: Arc::default(),
        },
        Arc::new(Carrier::default()),
        sink,
        Limits::default(),
        DetachedWorkers::new(),
        false,
    );
    let early = closed.receive(&body(1, "status", false, "a"));
    assert!(
        early.is_err(),
        "a call before the handshake is a protocol error"
    );
    closed.open();
    assert!(closed.is_open());
}

/// Red row 1 (step 2.2b): a call whose handler has a running `Open`
/// round-trips. The handler's `open` leaves on tag 16 while the call runs;
/// the client's `opened` comes back on tag 17, and the test router hands it to
/// the call because the service's live-call registry names it; only then does
/// the call's reply arrive. The router is the brief's fake of step 2.1's,
/// which this row is re-pointed at when 2.1 merges.
#[test]
fn a_call_with_a_running_open_round_trips_over_the_transport_lane() {
    let mut host = Host::new(Limits::default());
    let fetch = host.call("fetch", false, "open-1");
    let open = host.carrier.wait_sent();
    assert_eq!((open.tag, text(&open.body)), (16, "open-1|open".to_owned()));
    assert!(
        !host.sink.has(fetch),
        "the envelope travels before the reply"
    );
    assert!(host.service.is_live("open-1"));
    let opened = Frame {
        tag: 17,
        body: b"open-1|opened".to_vec(),
    };
    assert_eq!(
        test_route(&host.service, &host.carrier, &opened),
        Ok("delivered")
    );
    assert_eq!(
        host.sink.wait(fetch),
        Ok("done fetch open-1|opened".to_owned())
    );
    // The call has ended, so it is no longer live: a late frame is discarded.
    assert!(!host.service.is_live("open-1"));
    assert_eq!(
        test_route(&host.service, &host.carrier, &opened),
        Ok("discarded")
    );
}

/// Red row 4 (step 2.2b): a transport-lane frame before the handshake is a
/// protocol error, as is any session frame before it, or a frame of a tag the
/// host never takes once open.
#[test]
fn a_transport_lane_frame_before_the_handshake_is_a_protocol_error() {
    let sink = Arc::new(Sink::default()) as Arc<dyn ReplySink>;
    let carrier = Carrier::default();
    let service = Service::new(
        Ports {
            script: Arc::default(),
        },
        Arc::new(carrier.clone()),
        sink,
        Limits::default(),
        DetachedWorkers::new(),
        false,
    );
    service.start().unwrap();
    let lane_frame = Frame {
        tag: 17,
        body: b"r|opened".to_vec(),
    };
    let early = test_route(&service, &carrier, &lane_frame);
    assert!(early.unwrap_err().0.contains("before the session opened"));
    let call = Frame::new(Tag::SessionCall, body(1, "status", false, "s"));
    assert!(service.route(&call).is_err(), "a call before the handshake");
    service.open();
    assert_eq!(test_route(&service, &carrier, &lane_frame), Ok("discarded"));
    let reply_tag = Frame {
        tag: 2,
        body: Vec::new(),
    };
    assert!(test_route(&service, &carrier, &reply_tag).is_err());
    service.channel_closed(Ending::Protocol);
    assert!(service.wait_ended(Duration::from_secs(5)));
}

/// S-P3-2: a malformed control call is refused on the control lane, decided
/// from its method's name before anything is decoded.
#[test]
fn a_malformed_control_calls_refusal_takes_the_control_lane() {
    let mut host = Host::new(Limits::default());
    let bad = host.call("bad-control", false, "x");
    assert!(
        host.sink
            .wait(bad)
            .unwrap_err()
            .starts_with("InvalidRequest")
    );
    assert_eq!(host.sink.lane(bad), (Lane::Control, true));
}
