use super::*;

/// The failure core would report: the code's name and the text.
type Failure = String;

/// Releases or holds the scripted handlers, by request text.
#[derive(Default)]
pub(super) struct Script {
    released: Mutex<HashSet<String>>,
    changed: Condvar,
    pub(super) ran: Mutex<Vec<String>>,
}

impl Script {
    pub(super) fn release(&self, request: &str) {
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

pub(super) struct Ports {
    pub(super) script: Arc<Script>,
}

impl HostPorts for Ports {
    type Session = Carrier;
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

    fn lane(&self, method: &str) -> Lane {
        match method {
            "operation.cancel" | "session.close" | "bad-control" => Lane::Control,
            _ => Lane::Call,
        }
    }

    fn classify(&self, call: &Call) -> Result<Request, HostError> {
        let request = text(&call.request);
        Ok(match call.method.as_str() {
            "status" => Request::Direct,
            "fetch" => Request::Operation {
                class: Class::Fetch,
                response_view: false,
            },
            "push" => Request::Operation {
                class: Class::Push,
                response_view: false,
            },
            "merge" => Request::Operation {
                class: Class::Write,
                response_view: true,
            },
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
            "bad" | "bad-control" => {
                return Err(HostError::new(Code::InvalidRequest, "bad call"));
            }
            _ => Request::Operation {
                class: Class::Write,
                response_view: false,
            },
        })
    }

    fn resolve(&self, _session: &Carrier, call: &Call) -> Result<Resolved, Failure> {
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

    fn run(&self, call: &Call, resolved: &Resolved, worker: &Worker<Carrier>) -> Outcome<Failure> {
        let request = resolved.request_id.clone();
        crate::lock(&self.script.ran).push(request.clone());
        let _ = worker.event(format!("started {request}").into_bytes());
        if request.contains("panic") {
            panic!("scripted panic");
        }
        if request.starts_with("open") {
            // A handler with a running transport open: through its gate it
            // reaches the carrier, sends `open` on tag 16 and waits for the
            // client's answer, which the router hands back to its live call.
            let carrier = worker.effect(|scope| scope.session().clone()).unwrap();
            carrier.send(Frame {
                tag: 16,
                body: format!("{request}|open").into_bytes(),
            });
            let answer = carrier.wait_inbound(&request, Duration::from_secs(5));
            let answer = answer.map_or_else(|| "no answer".to_owned(), |answer| text(&answer));
            let _ = worker.event(b"opened".to_vec());
            return Outcome {
                terminal: Terminal::Completed(
                    format!("done {} {}", call.method, answer).into_bytes(),
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

    fn spool(&self, _session: &Carrier, _call: &Call) -> Result<Vec<u8>, Failure> {
        Ok(b"spool".to_vec())
    }

    fn release_spools(&self, _session: &Carrier) {}

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
