//! What an SSH open's URL holds beyond the open's destination, handed from
//! the driver that parsed the URL to the endpoint that sets the connection
//! up, in process only (TR2.18).
//!
//! The protocol's destination carries the pool key's host, lowercased, and has
//! no password slot (transport design §3.3). 1.0.17's libgit2 also uses the
//! host as the URL wrote it, which libssh2 hashes for a hashed `known_hosts`
//! name, and a password beside the URL's user, which it offers when the
//! server offers password authentication. The password is a secret: it
//! never enters a protocol message, a log or an error, and it is wiped when
//! the last open or setup that may use it drops it. The local placement's
//! driver and endpoint sessions run in one process
//! and share one [`Handoff`]. The driver deposits an open's extras under its
//! carrier session and stream while it queues the `Open`, and the endpoint
//! takes them as it starts that open. Nothing here crosses a carrier, so an
//! endpoint in another process gets none.
use super::agent_source::AgentSource;
use super::setup_retry::EndNote;
use super::ssh_host_key::HostKeyRefused;
use super::stream_io::ProducerGate;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, Mutex},
};

/// A password from an SSH URL. It has no `Display` or `Clone`, its `Debug`
/// shows none of it, and it overwrites its whole allocation as it drops.
pub(crate) struct UrlPassword(Vec<u8>);

impl UrlPassword {
    /// The password libssh2 is given for the URL's decoded `password`.
    /// libgit2 hands libssh2 a C string, so it ends at the first NUL.
    pub(crate) fn new(mut password: Vec<u8>) -> Self {
        if let Some(end) = password.iter().position(|byte| *byte == 0) {
            // The bytes past it stay in the allocation until the drop wipes them.
            password.truncate(end);
        }
        Self(password)
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for UrlPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UrlPassword(<redacted>)")
    }
}

impl Drop for UrlPassword {
    fn drop(&mut self) {
        crate::session_host::environment::overwrite(&mut self.0);
    }
}

/// What one open's URL holds beyond its destination.
pub(crate) struct UrlExtras {
    host: String,
    password: Option<UrlPassword>,
    helpers_allowed: bool,
}

impl UrlExtras {
    /// `host` is the destination's host as the URL wrote it, and `password`
    /// the password beside its user, which libgit2 uses.
    pub(crate) fn new(host: String, password: Option<UrlPassword>) -> Self {
        Self {
            host,
            password,
            helpers_allowed: true,
        }
    }
    pub(crate) fn with_helpers(mut self, allowed: bool) -> Self {
        self.helpers_allowed = allowed;
        self
    }
    pub(crate) fn helpers_allowed(&self) -> bool {
        self.helpers_allowed
    }

    /// The host as the URL wrote it, which a hashed `known_hosts` name may hash.
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    /// The URL's password, which authenticates its open's connection when
    /// the server takes it.
    pub(crate) fn password(&self) -> Option<&UrlPassword> {
        self.password.as_ref()
    }
}

/// One runtime's handoff, shared by its driver and endpoint sessions. It
/// holds the extras of opens queued and not yet started, under their carrier
/// session and stream.
#[derive(Clone, Default)]
pub(crate) struct Handoff {
    extras: Arc<Mutex<BTreeMap<(String, i64), UrlExtras>>>,
    agent: AgentSource,
    /// What the trust check last found for each destination it refused (decision D12). The wire carries a refused host
    /// key as `Trust` and nothing more, so the endpoint leaves the finding here and the driver words the failure from
    /// it. The driver never takes it: several opens to one host fail on one finding. Bounded by `TRUST_LIMIT`.
    trust: Arc<Mutex<BTreeMap<(String, u16), HostKeyRefused>>>,
    /// How each failed open ended beyond its failure, which the wire has no field for (a Down key's members, a
    /// throttled member's refusals, the local wait that expired): the endpoint leaves it under the open's carrier
    /// session and stream, and the driver takes it once to word the failure. Bounded by `END_LIMIT`; an open the
    /// driver never takes it for (cancelled) is pushed out by later ones.
    ends: Arc<Mutex<BTreeMap<(String, i64), EndNote>>>,
    /// The opens the endpoint holds now, under their carrier session and stream: waiting for room, behind a hold of
    /// their site or in their key's retry wait, with their allocation clocks stopped. The wire has no field for it, so
    /// the endpoint's session keeps this current and the driver's open backstop reads it, to charge an open for its
    /// attempts and not for its holds (`transport_host/session/driver/backstop.rs`).
    holds: Arc<Mutex<BTreeSet<(String, i64)>>>,
    /// The producer gates of the push opens queued and not yet started, under their carrier session and stream: the
    /// driver's `BlockingStream` and the endpoint's pump share each one (GwzPushProducerBusyDesign §2).
    producers: Arc<Mutex<BTreeMap<(String, i64), ProducerGate>>>,
}

/// The destinations whose trust findings a handoff keeps.
const TRUST_LIMIT: usize = 256;
/// The failed opens whose end notes a handoff keeps.
const END_LIMIT: usize = 256;

impl Handoff {
    /// A handoff for a runtime whose endpoint uses `agent`: the driver words the agent step's failures from it, since
    /// a failure crosses to the driver as a code with no text (plan step 3.2b).
    pub(crate) fn with_agent(agent: AgentSource) -> Self {
        Self {
            extras: Arc::default(),
            agent,
            trust: Arc::default(),
            ends: Arc::default(),
            holds: Arc::default(),
            producers: Arc::default(),
        }
    }

    /// Records that the endpoint no longer holds the opens of `released` and holds those of `held`.
    pub(crate) fn update_holds(&self, released: &[(String, i64)], held: &[(String, i64)]) {
        let mut holds = self.holds.lock().unwrap_or_else(|e| e.into_inner());
        for open in released {
            holds.remove(open);
        }
        holds.extend(held.iter().cloned());
    }

    /// The streams of `session` whose opens the endpoint holds now.
    pub(crate) fn holds(&self, session: &str) -> BTreeSet<i64> {
        let holds = self.holds.lock().unwrap_or_else(|e| e.into_inner());
        holds
            .range((session.to_owned(), i64::MIN)..=(session.to_owned(), i64::MAX))
            .map(|(_, stream)| *stream)
            .collect()
    }

    /// Leaves `note` for the driver of `session`'s `stream`, which words that open's failure with it.
    pub(crate) fn note_end(&self, session: &str, stream: i64, note: EndNote) {
        let mut ends = self.ends.lock().unwrap_or_else(|e| e.into_inner());
        ends.insert((session.to_owned(), stream), note);
        while ends.len() > END_LIMIT {
            ends.pop_first();
        }
    }

    /// The note left for `session`'s `stream`, once: the driver's, as its open fails.
    pub(crate) fn take_end(&self, session: &str, stream: i64) -> Option<EndNote> {
        self.ends
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(session.to_owned(), stream))
    }

    /// Records what `error` says the trust check found for `host` and `port`, if it was a refused host key.
    pub(crate) fn note_trust(&self, host: &str, port: u16, error: &std::io::Error) {
        let Some(found) = HostKeyRefused::found_in(error) else {
            return;
        };
        let mut trust = self.trust.lock().unwrap_or_else(|e| e.into_inner());
        trust.insert((host.to_ascii_lowercase(), port), found);
        while trust.len() > TRUST_LIMIT {
            trust.pop_first();
        }
    }

    /// What the trust check last found for `host` and `port`, if it refused the key in this runtime.
    pub(crate) fn trust_found(&self, host: &str, port: u16) -> Option<HostKeyRefused> {
        self.trust
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(host.to_ascii_lowercase(), port))
            .copied()
    }

    pub(crate) fn agent(&self) -> &AgentSource {
        &self.agent
    }

    /// Queues an open with `open`, which returns its stream, and deposits
    /// `extras` and `producer` under `session` and that stream, holding the
    /// handoff throughout: an `Open` can reach the endpoint only once `open`
    /// has queued it, and the endpoint's [`Handoff::take`] waits for the
    /// deposit. When `open` fails, the extras stay with the caller and nothing
    /// is deposited. The driver keeps the [`Deposit`] until its open has its
    /// answer, whichever it is.
    pub(crate) fn open<E>(
        &self,
        session: &str,
        extras: &mut Option<UrlExtras>,
        producer: Option<&ProducerGate>,
        open: impl FnOnce() -> Result<i64, E>,
    ) -> Result<(i64, Option<Deposit>), E> {
        let mut deposits = self.extras.lock().unwrap_or_else(|e| e.into_inner());
        let stream = open()?;
        let deposited = extras.take().map(|extras| {
            deposits.insert((session.to_owned(), stream), extras);
        });
        let gated = producer.map(|gate| {
            self.producers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((session.to_owned(), stream), gate.clone());
        });
        let deposit = (deposited.is_some() || gated.is_some()).then(|| Deposit {
            handoff: self.clone(),
            session: session.to_owned(),
            stream,
        });
        Ok((stream, deposit))
    }

    /// The producer gate deposited for `session`'s `stream`, once: the endpoint's, as it starts that open.
    pub(crate) fn take_producer(&self, session: &str, stream: i64) -> Option<ProducerGate> {
        self.producers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(session.to_owned(), stream))
    }

    /// The extras deposited for `session`'s `stream`, once: the endpoint's,
    /// as it starts that open.
    pub(crate) fn take(&self, session: &str, stream: i64) -> Option<UrlExtras> {
        self.extras
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(session.to_owned(), stream))
    }
}

/// An open's deposit. Dropping it drops what the endpoint never took: its
/// extras and its producer gate.
pub(crate) struct Deposit {
    handoff: Handoff,
    session: String,
    stream: i64,
}

impl Drop for Deposit {
    fn drop(&mut self) {
        drop(self.handoff.take(&self.session, self.stream));
        drop(self.handoff.take_producer(&self.session, self.stream));
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn extras_reach_the_stream_their_open_queued_and_only_once() {
            let handoff = Handoff::default();
            let mut extras = Some(UrlExtras::new("GitHost.Example".into(), None));
            let (stream, deposit) = handoff.open("s", &mut extras, None, || Ok::<_, ()>(7)).unwrap();
            assert_eq!(stream, 7);
            assert!(extras.is_none());
            assert!(handoff.take("s", 8).is_none());
            assert!(handoff.take("other", 7).is_none());
            assert_eq!(handoff.take("s", 7).unwrap().host(), "GitHost.Example");
            assert!(handoff.take("s", 7).is_none());
            drop(deposit);
            assert!(handoff.extras.lock().unwrap().is_empty());
        }

        #[test]
        fn a_failed_open_deposits_nothing_and_keeps_its_extras() {
            let handoff = Handoff::default();
            let mut extras = Some(UrlExtras::new("GitHost.Example".into(), None));
            assert!(matches!(handoff.open("s", &mut extras, None, || Err::<i64, _>("full")), Err("full")));
            assert!(extras.is_some());
            assert!(handoff.extras.lock().unwrap().is_empty());
            let (stream, deposit) = handoff.open("s", &mut extras, None, || Ok::<_, ()>(9)).unwrap();
            assert_eq!(stream, 9);
            assert_eq!(handoff.extras.lock().unwrap().len(), 1);
            // The endpoint never took them: the driver's deposit drops them.
            drop(deposit);
            assert!(handoff.extras.lock().unwrap().is_empty());
            let mut none = None;
            let (_, deposit) = handoff.open("s", &mut none, None, || Ok::<_, ()>(10)).unwrap();
            assert!(deposit.is_none() && handoff.extras.lock().unwrap().is_empty());
        }

        #[test]
        fn a_producer_gate_reaches_the_stream_its_open_queued_and_only_once() {
            let handoff = Handoff::default();
            let gate = ProducerGate::new(7);
            let mut none = None;
            let (stream, deposit) = handoff
                .open("s", &mut none, Some(&gate), || Ok::<_, ()>(7))
                .unwrap();
            assert_eq!(stream, 7);
            assert!(deposit.is_some(), "a gate is a deposit");
            assert!(handoff.take_producer("s", 8).is_none());
            assert!(handoff.take_producer("other", 7).is_none());
            assert!(handoff.take("s", 7).is_none(), "a gate is not URL extras: nothing else rides with it");
            let taken = handoff.take_producer("s", 7).unwrap();
            assert_eq!(taken.ceiling_ms(), 7);
            drop(taken.enter());
            assert!(gate.busy(), "the endpoint holds the driver's own gate");
            assert!(handoff.take_producer("s", 7).is_none());
            drop(deposit);
        }

        #[test]
        fn a_dropped_deposit_takes_back_a_gate_the_endpoint_never_took() {
            let handoff = Handoff::default();
            let gate = ProducerGate::new(7);
            let mut extras = Some(UrlExtras::new("GitHost.Example".into(), None));
            let (_, deposit) = handoff
                .open("s", &mut extras, Some(&gate), || Ok::<_, ()>(5))
                .unwrap();
            assert_eq!(handoff.producers.lock().unwrap().len(), 1);
            drop(deposit);
            assert!(handoff.producers.lock().unwrap().is_empty());
            assert!(handoff.extras.lock().unwrap().is_empty());
        }

        #[test]
        fn a_failed_open_deposits_no_gate() {
            let handoff = Handoff::default();
            let gate = ProducerGate::new(7);
            let mut none = None;
            assert!(matches!(
                handoff.open("s", &mut none, Some(&gate), || Err::<i64, _>("full")),
                Err("full")
            ));
            assert!(handoff.producers.lock().unwrap().is_empty());
        }

        #[test]
        fn a_password_ends_at_its_first_nul_and_shows_nothing() {
            let password = UrlPassword::new(b"pass\0word".to_vec());
            assert_eq!(password.bytes(), b"pass");
            assert_eq!(format!("{password:?}"), "UrlPassword(<redacted>)");
            let extras = UrlExtras::new("host".into(), Some(UrlPassword::new(vec![0xff, b'x'])));
            assert_eq!(extras.password().unwrap().bytes(), [0xff, b'x']);
        }

        #[test]
        fn the_endpoint_waits_for_a_deposit_in_progress() {
            use std::sync::mpsc;
            let handoff = Handoff::default();
            let (queued, observe) = mpsc::channel();
            let (release, wait) = mpsc::channel::<()>();
            let driver = handoff.clone();
            let opener = std::thread::spawn(move || {
                let mut extras = Some(UrlExtras::new("GitHost.Example".into(), None));
                driver.open("s", &mut extras, None, || {
                    // The Open is queued: an endpoint could see it now.
                    queued.send(()).unwrap();
                    wait.recv().unwrap();
                    Ok::<_, ()>(3)
                })
            });
            observe.recv().unwrap();
            let endpoint = handoff.clone();
            let taker = std::thread::spawn(move || endpoint.take("s", 3).map(|e| e.host().to_owned()));
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!taker.is_finished(), "take returned before the deposit");
            release.send(()).unwrap();
            let (stream, deposit) = opener.join().unwrap().unwrap();
            assert_eq!(stream, 3);
            assert_eq!(taker.join().unwrap().as_deref(), Some("GitHost.Example"));
            drop(deposit);
        }
    }
}
