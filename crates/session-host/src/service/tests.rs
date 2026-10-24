#![cfg(test)]
//! The operation service on fake ports: core's half (decoding, classes,
//! handlers, encoding) is a script here, so these rows check the host's own
//! rules (§4.2, §5.1–§5.4, §6, §8).

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gwz_session_contract::{Frame, Lane, SendError, Tag};

use super::*;
use crate::lane::{LaneFrame, LaneKind, Lanes, Routed, StreamTarget, route_frame};

/// The failure core would report: the code's name and the text.
type Failure = String;

/// Releases or holds the scripted handlers, by request text.
#[derive(Default)]
struct Script {
    released: Mutex<HashSet<String>>,
    changed: Condvar,
    ran: Mutex<Vec<String>>,
}

impl Script {
    fn release(&self, request: &str) {
        crate::lock(&self.released).insert(request.to_owned());
        self.changed.notify_all();
    }

    fn released(&self, request: &str) -> bool {
        crate::lock(&self.released).contains(request)
    }

    /// Waits until `request` is released or `token` cancelled (true when
    /// cancelled); `ignore` ignores the token.
    fn hold(&self, request: &str, token: &CancellationToken, ignore: bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut released = crate::lock(&self.released);
        loop {
            if released.contains(request) {
                return false;
            }
            if !ignore && token.is_cancelled() {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            released = self
                .changed
                .wait_timeout(released, Duration::from_millis(5))
                .unwrap()
                .0;
        }
    }
}

struct Ports {
    script: Arc<Script>,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A call's body: `id|method|mode|request`.
fn body(call_id: u64, method: &str, submit: bool, request: &str) -> Vec<u8> {
    format!("{call_id}|{method}|{}|{request}", u8::from(submit)).into_bytes()
}

impl HostPorts for Ports {
    type Session = Lanes;
    type Failure = Failure;

    fn decode(&self, body: &[u8]) -> Result<Call, ProtocolError> {
        let body = text(body);
        let parts: Vec<&str> = body.splitn(4, '|').collect();
        let [id, method, mode, request] = parts.as_slice() else {
            return Err(ProtocolError("the body does not decode".to_owned()));
        };
        Ok(Call {
            call_id: id.parse().map_err(|_| ProtocolError("bad id".to_owned()))?,
            method: (*method).to_owned(),
            mode: if *mode == "1" {
                Mode::Submit
            } else {
                Mode::Unary
            },
            request: request.as_bytes().to_vec(),
            verb: None,
        })
    }

    fn classify(&self, call: &Call) -> Result<Request, HostError> {
        let request = text(&call.request);
        Ok(match call.method.as_str() {
            "status" => Request::Direct,
            "fetch" => Request::Operation {
                class: Class::Fetch,
            },
            "push" => Request::Operation { class: Class::Push },
            "events.subscribe" => {
                let (operation_id, rest) = request.split_once(' ').unwrap_or((&request, "0 0"));
                let (cursor, wait) = rest.split_once(' ').unwrap();
                Request::Events(EventsCall::Read(EventsRead {
                    operation_id: operation_id.to_owned(),
                    stream_id: "s".to_owned(),
                    cursor: Some(cursor.parse().unwrap()),
                    max_records: None,
                    max_bytes: None,
                    wait: Duration::from_millis(wait.parse().unwrap()),
                }))
            }
            "operation.result" => Request::Result {
                operation_id: request,
            },
            "operation.response" => Request::Response {
                operation_id: request,
            },
            "operation.release" => Request::Release {
                operation_id: request,
            },
            "operation.cancel" => Request::Cancel(match request.parse::<u64>() {
                Ok(call_id) => CancelTarget::Call(call_id),
                Err(_) => CancelTarget::Operation(request),
            }),
            "session.close" => Request::Close,
            "bad" => {
                return Err(HostError::new(Code::InvalidRequest, "bad call"));
            }
            _ => Request::Operation {
                class: Class::Write,
            },
        })
    }

    fn resolve(&self, _session: &Lanes, call: &Call) -> Result<Resolved, Failure> {
        let request = text(&call.request);
        if request.starts_with("slow") {
            let start = Instant::now();
            while !self.script.released(&format!("resolve {request}"))
                && start.elapsed() < Duration::from_secs(20)
            {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if request.starts_with("unresolvable") {
            return Err("WorkspaceNotFound: no workspace".to_owned());
        }
        Ok(Resolved {
            workspace: "/ws".to_owned(),
            request_id: request,
        })
    }

    fn run(&self, call: &Call, resolved: &Resolved, worker: &Worker<Lanes>) -> Outcome<Failure> {
        let request = resolved.request_id.clone();
        crate::lock(&self.script.ran).push(request.clone());
        let _ = worker.event(format!("started {request}").into_bytes());
        if request.contains("panic") {
            panic!("scripted panic");
        }
        if request.starts_with("open") {
            // A handler with a running transport open: it binds its call's
            // lane, sends `open` on tag 16 and waits for the client's answer.
            let lane = worker
                .effect(|scope| scope.session().register(&request, 1024))
                .unwrap()
                .unwrap();
            let target = StreamTarget {
                request_id: request.clone(),
                session_id: "s".to_owned(),
                stream_id: 1,
            };
            let open = Frame {
                tag: 16,
                body: format!("{request}|s|1|open").into_bytes(),
            };
            lane.send(open, &target).unwrap();
            let answer = lane.recv(Duration::from_secs(20)).unwrap();
            let _ = worker.event(b"opened".to_vec());
            return Outcome {
                terminal: Terminal::Completed(
                    format!("done {} {}", call.method, text(&answer)).into_bytes(),
                ),
                result: Vec::new(),
                cleanup: Cleanup::default(),
            };
        }
        let cancelled = if request.contains("hold") {
            self.script
                .hold(&request, worker.token(), request.contains("stubborn"))
        } else {
            false
        };
        let terminal = if cancelled {
            Terminal::Cancelled("Cancelled: cancelled".to_owned())
        } else {
            let _ = worker.event(format!("finished {request}").into_bytes());
            Terminal::Completed(format!("done {} {}", call.method, request).into_bytes())
        };
        Outcome {
            terminal,
            result: format!("result {request}").into_bytes(),
            cleanup: Cleanup {
                pending_local_work: 0,
                peer_cleanup_confirmed: call.method == "fetch",
            },
        }
    }

    fn spool(&self, _session: &Lanes, _call: &Call) -> Result<Vec<u8>, Failure> {
        Ok(b"spool".to_vec())
    }

    fn release_spools(&self, _session: &Lanes) {}

    fn failure(&self, error: HostError) -> Failure {
        format!("{:?}: {}", error.code, error.message)
    }

    fn settled_result(&self, _call: &Call, operation_id: &str, failure: &Failure) -> Vec<u8> {
        format!("result {operation_id} {failure}").into_bytes()
    }

    fn detached(&self, _call: &Call, operation_id: Option<&str>) -> (Failure, Vec<u8>) {
        (
            "Cancelled: detached at the close bound; outcome unknown".to_owned(),
            format!("detached {operation_id:?}").into_bytes(),
        )
    }

    fn accepted(&self, _call: &Call, _resolved: &Resolved, operation_id: &str) -> Vec<u8> {
        format!("accepted {operation_id}").into_bytes()
    }

    fn reset_event(&self, _operation_id: &str) -> Vec<u8> {
        b"reset".to_vec()
    }

    fn events(&self, _read: &EventsRead, batch: &EventsBatch) -> Vec<u8> {
        let records: Vec<String> = batch
            .records
            .iter()
            .map(|(seq, payload)| format!("{seq}={}", text(payload)))
            .collect();
        format!(
            "events next={} complete={} [{}]",
            batch.next_cursor,
            batch.complete,
            records.join(",")
        )
        .into_bytes()
    }

    fn events_ended(&self, _operation_id: &str, _stream_id: &str) -> Vec<u8> {
        b"ended".to_vec()
    }

    fn cleanup(&self, report: Cleanup) -> Vec<u8> {
        format!(
            "cleanup {} {}",
            report.pending_local_work, report.peer_cleanup_confirmed
        )
        .into_bytes()
    }

    fn released(&self) -> Vec<u8> {
        b"released".to_vec()
    }

    fn reply(&self, call_id: u64, body: Vec<u8>) -> Frame {
        let mut bytes = call_id.to_be_bytes().to_vec();
        bytes.extend(body);
        Frame::new(Tag::SessionReply, bytes)
    }

    fn error(&self, call_id: u64, failure: &Failure) -> Frame {
        let mut bytes = call_id.to_be_bytes().to_vec();
        bytes.extend(failure.as_bytes());
        Frame::new(Tag::SessionError, bytes)
    }
}

/// A reply: its call, its answer, its lane and whether it was control class.
type Sent = (u64, Result<String, String>, Lane, bool);
/// A lane frame, its lane, its target and its class.
type SentLane = (Frame, Lane, StreamTarget, bool);

/// The replies the host sent, by call.
#[derive(Default)]
struct Sink {
    replies: Mutex<Vec<Sent>>,
    lanes: Mutex<Vec<SentLane>>,
    lane_changed: Condvar,
    changed: Condvar,
    closed: Mutex<bool>,
}

impl ReplySink for Sink {
    fn send_reply(
        &self,
        frame: Frame,
        lane: Lane,
        target: u64,
        control: bool,
    ) -> Result<(), SendError> {
        let id = u64::from_be_bytes(frame.body[..8].try_into().unwrap());
        assert_eq!(id, target);
        let payload = text(&frame.body[8..]);
        let answer = if frame.tag == Tag::SessionReply.byte() {
            Ok(payload)
        } else {
            Err(payload)
        };
        crate::lock(&self.replies).push((id, answer, lane, control));
        self.changed.notify_all();
        Ok(())
    }

    fn send_lane(
        &self,
        frame: Frame,
        lane: Lane,
        target: &StreamTarget,
        control: bool,
    ) -> Result<(), SendError> {
        crate::lock(&self.lanes).push((frame, lane, target.clone(), control));
        self.lane_changed.notify_all();
        Ok(())
    }

    fn close_channel(&self) {
        *crate::lock(&self.closed) = true;
    }
}

impl Sink {
    fn wait(&self, call_id: u64) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut replies = crate::lock(&self.replies);
        loop {
            if let Some((_, answer, _, _)) = replies.iter().find(|(id, ..)| *id == call_id) {
                return answer.clone();
            }
            assert!(Instant::now() < deadline, "no reply to call {call_id}");
            replies = self
                .changed
                .wait_timeout(replies, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }

    fn wait_lane(&self) -> (Frame, Lane, StreamTarget, bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut lanes = crate::lock(&self.lanes);
        loop {
            if !lanes.is_empty() {
                return lanes.remove(0);
            }
            assert!(Instant::now() < deadline, "no lane frame");
            lanes = self
                .lane_changed
                .wait_timeout(lanes, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }

    fn has(&self, call_id: u64) -> bool {
        crate::lock(&self.replies)
            .iter()
            .any(|(id, ..)| *id == call_id)
    }

    fn lane(&self, call_id: u64) -> (Lane, bool) {
        let replies = crate::lock(&self.replies);
        let (_, _, lane, control) = replies.iter().find(|(id, ..)| *id == call_id).unwrap();
        (*lane, *control)
    }
}

struct Host {
    service: Service<Ports>,
    lanes: Lanes,
    sink: Arc<Sink>,
    script: Arc<Script>,
    next: u64,
}

impl Host {
    fn new(limits: Limits) -> Self {
        Self::shared(limits, DetachedWorkers::new())
    }

    fn shared(limits: Limits, detached: DetachedWorkers) -> Self {
        let script = Arc::new(Script::default());
        let sink = Arc::new(Sink::default());
        let lanes = Lanes::new(Arc::clone(&sink) as Arc<dyn ReplySink>);
        let service = Service::new(
            Ports {
                script: Arc::clone(&script),
            },
            Arc::new(lanes.clone()),
            Arc::clone(&sink) as Arc<dyn ReplySink>,
            limits,
            detached,
            true,
        );
        service.start().unwrap();
        Self {
            service,
            lanes,
            sink,
            script,
            next: 0,
        }
    }

    fn call(&mut self, method: &str, submit: bool, request: &str) -> u64 {
        self.next += 1;
        self.service
            .receive(&body(self.next, method, submit, request))
            .unwrap();
        self.next
    }

    fn ran(&self, request: &str) -> bool {
        crate::lock(&self.script.ran)
            .iter()
            .any(|ran| ran == request)
    }

    fn wait_ran(&self, request: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.ran(request) {
            assert!(Instant::now() < deadline, "{request} never ran");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

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
        Arc::new(Lanes::new(Arc::clone(&sink))),
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
    let result = host.call("operation.result", false, "op-1");
    host.sink.wait(result).unwrap();
    let response = host.call("operation.response", false, "op-2");
    host.sink.wait(response).unwrap();
    // op-1 and op-2 are delivered; the third evicts op-1.
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

/// The test client's decoding of a lane frame: `request|session|stream|kind`.
fn decode_lane(frame: &Frame) -> Result<LaneFrame, ProtocolError> {
    let body = text(&frame.body);
    let parts: Vec<&str> = body.split('|').collect();
    let [request_id, session_id, stream_id, kind] = parts.as_slice() else {
        return Err(ProtocolError("the lane body does not decode".to_owned()));
    };
    let kind = match *kind {
        "open" => LaneKind::Other {
            kind: 4,
            control: false,
        },
        "opened" => LaneKind::Other {
            kind: 5,
            control: true,
        },
        other => {
            return Err(ProtocolError(format!("unknown kind {other}")));
        }
    };
    Ok(LaneFrame {
        tag: frame.tag,
        target: StreamTarget {
            request_id: (*request_id).to_owned(),
            session_id: (*session_id).to_owned(),
            stream_id: stream_id.parse().unwrap(),
        },
        kind,
        body: frame.body.clone(),
    })
}

/// Red row (step 2.2b): a call whose handler has a running `Open` round-trips.
/// The handler's `open` leaves on tag 16 while the call runs, the client's
/// `opened` comes back on tag 17 and reaches the handler, and only then does
/// the call's reply arrive. The carrier is this test's fake, since the
/// in-process adapter takes tags 16 and 17 only once step 2.1 lands.
#[test]
fn a_call_with_a_running_open_round_trips_over_the_transport_lane() {
    let mut host = Host::new(Limits::default());
    let fetch = host.call("fetch", false, "open-1");
    let (open, lane, target, control) = host.sink.wait_lane();
    assert_eq!((open.tag, lane, control), (16, Lane::Call, false));
    assert_eq!(target.request_id, "open-1");
    assert!(
        !host.sink.has(fetch),
        "the envelope travels before the reply"
    );
    let opened = Frame {
        tag: 17,
        body: b"open-1|s|1|opened".to_vec(),
    };
    let routed = route_frame(&host.service, &host.lanes, &opened, decode_lane).unwrap();
    assert_eq!(routed, Routed::Delivered);
    assert_eq!(
        host.sink.wait(fetch),
        Ok("done fetch open-1|s|1|opened".to_owned())
    );
    // The call has ended, so its lane is gone: a late frame is discarded.
    let late = route_frame(&host.service, &host.lanes, &opened, decode_lane).unwrap();
    assert_eq!(late, Routed::Discarded);
}

/// Red row (step 2.2b): a transport-lane frame before the handshake is a
/// protocol error, as is a frame of a tag the host never takes.
#[test]
fn a_transport_lane_frame_before_the_handshake_is_a_protocol_error() {
    let sink = Arc::new(Sink::default()) as Arc<dyn ReplySink>;
    let lanes = Lanes::new(Arc::clone(&sink));
    let service = Service::new(
        Ports {
            script: Arc::default(),
        },
        Arc::new(lanes.clone()),
        sink,
        Limits::default(),
        DetachedWorkers::new(),
        false,
    );
    service.start().unwrap();
    let lane_frame = Frame {
        tag: 17,
        body: b"r|s|1|opened".to_vec(),
    };
    let early = route_frame(&service, &lanes, &lane_frame, decode_lane);
    assert!(early.unwrap_err().0.contains("before the session opened"));
    service.open();
    assert_eq!(
        route_frame(&service, &lanes, &lane_frame, decode_lane).unwrap(),
        Routed::Discarded
    );
    let reply_tag = Frame {
        tag: 2,
        body: Vec::new(),
    };
    assert!(route_frame(&service, &lanes, &reply_tag, decode_lane).is_err());
    service.channel_closed(Ending::Protocol);
    assert!(service.wait_ended(Duration::from_secs(5)));
}
