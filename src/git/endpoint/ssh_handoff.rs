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
use std::{
    collections::BTreeMap,
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
pub(crate) struct Handoff(Arc<Mutex<BTreeMap<(String, i64), UrlExtras>>>);

impl Handoff {
    /// Queues an open with `open`, which returns its stream, and deposits
    /// `extras` under `session` and that stream, holding the handoff
    /// throughout: an `Open` can reach the endpoint only once `open` has
    /// queued it, and the endpoint's [`Handoff::take`] waits for the deposit.
    /// When `open` fails, the extras stay with the caller. The driver keeps
    /// the [`Deposit`] until its open has its answer, whichever it is.
    pub(crate) fn open<E>(
        &self,
        session: &str,
        extras: &mut Option<UrlExtras>,
        open: impl FnOnce() -> Result<i64, E>,
    ) -> Result<(i64, Option<Deposit>), E> {
        let mut deposits = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let stream = open()?;
        let deposit = extras.take().map(|extras| {
            deposits.insert((session.to_owned(), stream), extras);
            Deposit {
                handoff: self.clone(),
                session: session.to_owned(),
                stream,
            }
        });
        Ok((stream, deposit))
    }

    /// The extras deposited for `session`'s `stream`, once: the endpoint's,
    /// as it starts that open.
    pub(crate) fn take(&self, session: &str, stream: i64) -> Option<UrlExtras> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(session.to_owned(), stream))
    }
}

/// An open's deposit. Dropping it drops what the endpoint never took.
pub(crate) struct Deposit {
    handoff: Handoff,
    session: String,
    stream: i64,
}

impl Drop for Deposit {
    fn drop(&mut self) {
        drop(self.handoff.take(&self.session, self.stream));
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn extras_reach_the_stream_their_open_queued_and_only_once() {
            let handoff = Handoff::default();
            let mut extras = Some(UrlExtras::new("GitHost.Example".into(), None));
            let (stream, deposit) = handoff.open("s", &mut extras, || Ok::<_, ()>(7)).unwrap();
            assert_eq!(stream, 7);
            assert!(extras.is_none());
            assert!(handoff.take("s", 8).is_none());
            assert!(handoff.take("other", 7).is_none());
            assert_eq!(handoff.take("s", 7).unwrap().host(), "GitHost.Example");
            assert!(handoff.take("s", 7).is_none());
            drop(deposit);
            assert!(handoff.0.lock().unwrap().is_empty());
        }

        #[test]
        fn a_failed_open_deposits_nothing_and_keeps_its_extras() {
            let handoff = Handoff::default();
            let mut extras = Some(UrlExtras::new("GitHost.Example".into(), None));
            assert!(matches!(handoff.open("s", &mut extras, || Err::<i64, _>("full")), Err("full")));
            assert!(extras.is_some());
            assert!(handoff.0.lock().unwrap().is_empty());
            let (stream, deposit) = handoff.open("s", &mut extras, || Ok::<_, ()>(9)).unwrap();
            assert_eq!(stream, 9);
            assert_eq!(handoff.0.lock().unwrap().len(), 1);
            // The endpoint never took them: the driver's deposit drops them.
            drop(deposit);
            assert!(handoff.0.lock().unwrap().is_empty());
            let mut none = None;
            let (_, deposit) = handoff.open("s", &mut none, || Ok::<_, ()>(10)).unwrap();
            assert!(deposit.is_none() && handoff.0.lock().unwrap().is_empty());
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
                driver.open("s", &mut extras, || {
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
