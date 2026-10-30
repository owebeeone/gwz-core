use super::*;

/// A reply: its call, its answer, its lane and whether it was control class.
type Sent = (u64, Result<String, String>, Lane, bool);

/// The replies the host sent, by call, and the order of everything it did.
#[derive(Default)]
pub(super) struct Sink {
    replies: Mutex<Vec<Sent>>,
    changed: Condvar,
    pub(super) closed: Mutex<bool>,
    /// `reply N` and `closed`, in the order they happened.
    pub(super) order: Mutex<Vec<String>>,
    /// A call whose reply the sink holds back, and for how long, or with how
    /// many yields when the duration is zero.
    pub(super) slow: Mutex<Option<(u64, Duration, u32)>>,
    /// A call whose send parks once its reply is visible to the client, until
    /// `resume`: the client then runs ahead of the sender's bookkeeping.
    pub(super) parked: Mutex<Option<u64>>,
    pub(super) resume: Mutex<bool>,
    resumed: Condvar,
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
        let slow = *crate::lock(&self.slow);
        if let Some((call, delay, yields)) = slow
            && call == id
        {
            std::thread::sleep(delay);
            for _ in 0..yields {
                std::thread::yield_now();
            }
        }
        if *crate::lock(&self.closed) {
            return Err(SendError::Closed(
                frame,
                gwz_session_contract::Closed::Local,
            ));
        }
        crate::lock(&self.order).push(format!("reply {id}"));
        let payload = text(&frame.body[8..]);
        let answer = if frame.tag == Tag::SessionReply.byte() {
            Ok(payload)
        } else {
            Err(payload)
        };
        crate::lock(&self.replies).push((id, answer, lane, control));
        self.changed.notify_all();
        if *crate::lock(&self.parked) == Some(id) {
            let resume = crate::lock(&self.resume);
            let _ = self
                .resumed
                .wait_timeout_while(resume, Duration::from_secs(20), |resume| !*resume)
                .unwrap();
        }
        Ok(())
    }

    fn close_channel(&self) {
        *crate::lock(&self.closed) = true;
        crate::lock(&self.order).push("closed".to_owned());
    }
}

impl Sink {
    pub(super) fn wait(&self, call_id: u64) -> Result<String, String> {
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

    pub(super) fn resume(&self) {
        *crate::lock(&self.resume) = true;
        self.resumed.notify_all();
    }

    pub(super) fn has(&self, call_id: u64) -> bool {
        crate::lock(&self.replies)
            .iter()
            .any(|(id, ..)| *id == call_id)
    }

    pub(super) fn lane(&self, call_id: u64) -> (Lane, bool) {
        let replies = crate::lock(&self.replies);
        let (_, _, lane, control) = replies.iter().find(|(id, ..)| *id == call_id).unwrap();
        (*lane, *control)
    }
}

pub(super) struct Host {
    pub(super) service: Service<Ports>,
    pub(super) carrier: Carrier,
    pub(super) sink: Arc<Sink>,
    pub(super) script: Arc<Script>,
    next: u64,
}

impl Host {
    pub(super) fn new(limits: Limits) -> Self {
        Self::shared(limits, DetachedWorkers::new())
    }

    pub(super) fn shared(limits: Limits, detached: DetachedWorkers) -> Self {
        let script = Arc::new(Script::default());
        let sink = Arc::new(Sink::default());
        let carrier = Carrier::default();
        let service = Service::new(
            Ports {
                script: Arc::clone(&script),
            },
            Arc::new(carrier.clone()),
            Arc::clone(&sink) as Arc<dyn ReplySink>,
            limits,
            detached,
            true,
        );
        service.start().unwrap();
        Self {
            service,
            carrier,
            sink,
            script,
            next: 0,
        }
    }

    pub(super) fn call(&mut self, method: &str, submit: bool, request: &str) -> u64 {
        self.next += 1;
        self.service
            .receive(&body(self.next, method, submit, request))
            .unwrap();
        self.next
    }

    pub(super) fn ran(&self, request: &str) -> bool {
        crate::lock(&self.script.ran)
            .iter()
            .any(|ran| ran == request)
    }

    pub(super) fn wait_ran(&self, request: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.ran(request) {
            assert!(Instant::now() < deadline, "{request} never ran");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
