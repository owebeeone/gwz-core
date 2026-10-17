//! What the real handshakes answered, for the test rows that connect to a live `sshd` and must not hide a failure.
//!
//! libssh2's WinCNG backend ends about one key exchange in a hundred by itself (`LIBSSH2_ERROR_KEX_FAILURE`, -8), and
//! the setup connects afresh twice after it (operator decisions D14 and D32, [`KexRetry`]). A row that makes many
//! connections and may meet a failure that survived those retries retries on exactly that one class: three key
//! exchange failures in a row, observed in the libssh2 codes themselves. Anything else fails the row with the codes printed, so a residual
//! class is seen and counted and not absorbed.
use super::{
    Network, establish_inner,
    kex_retry::{KEX_FAILURE, KexRetry, Native, Steps},
    read_regular, resolve,
};
use crate::git::endpoint::{agent_job::Control, ssh_connection::SshConnection};
use gwz_transport::pool::Key;
use ssh2::ErrorCode;
use std::{
    io,
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
};

/// The most tries a row makes for one expected connection (the first and four more).
pub(crate) const TRIES: usize = 5;

/// What one `establish` did on the network: its TCP connects, and the answer of each handshake that was not
/// "would block" (`None` for a handshake that completed, `Some(code)` for libssh2's error).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) connects: usize,
    pub(crate) handshakes: Vec<Option<ErrorCode>>,
}

impl Observed {
    /// Whether the setup ended on the class that survives D32's two retries: three key exchange failures and nothing
    /// else. Two (or one) is not it: a setup that stopped there was refused a retry or had none, which the row must
    /// be told.
    pub(crate) fn is_triple_kex_failure(&self) -> bool {
        self.handshakes == [Some(KEX_FAILURE); 3]
    }
}

impl Observed {
    /// What the handshakes of this try came to, for the count a loop keeps of them: `clean` (one handshake, completed),
    /// `recovered` (the first retry cured a first -8), `recovered-twice` (the second cured it), `triple` (-8 three
    /// times, what the row retries), `none` (the setup ended before any handshake: a trust file it refused to read,
    /// say) and `other` (anything else, which the row never retries).
    pub(crate) fn class(&self) -> &'static str {
        match self.handshakes.as_slice() {
            [None] => "clean",
            [Some(KEX_FAILURE), None] => "recovered",
            [Some(KEX_FAILURE), Some(KEX_FAILURE), None] => "recovered-twice",
            [Some(KEX_FAILURE), Some(KEX_FAILURE), Some(KEX_FAILURE)] => "triple",
            [] => "none",
            _ => "other",
        }
    }
}

/// One line per try on the test's output, `KEX-TRY <class> <handshakes>`, so that a loop of runs can count the
/// classes (run with `--nocapture`); the output of a passing run is otherwise not shown.
pub(crate) fn report(tries: &[Observed]) {
    for tried in tries {
        eprintln!("KEX-TRY {} {:?}", tried.class(), tried.handshakes);
    }
}

/// Steps that pass everything to `inner` and write down what the handshakes answered.
struct Recording<S> {
    inner: S,
    seen: Arc<Mutex<Observed>>,
}

impl<S: Steps> Steps for Recording<S> {
    fn connect(&mut self, address: SocketAddr, control: &Control) -> io::Result<TcpStream> {
        self.seen.lock().unwrap().connects += 1;
        self.inner.connect(address, control)
    }

    fn handshake(&mut self, session: &mut ssh2::Session) -> Result<(), ssh2::Error> {
        let result = self.inner.handshake(session);
        let answer = match &result {
            Ok(()) => Some(None),
            // A nonblocking session answers "would block" until it is done: not an answer.
            Err(error) if error.code() == ErrorCode::Session(-37) => None,
            Err(error) => Some(Some(error.code())),
        };
        if let Some(answer) = answer {
            self.seen.lock().unwrap().handshakes.push(answer);
        }
        result
    }
}

/// One setup for `key` against `steps` under the policy `kex`, and what it did on the network.
pub(crate) fn establish_observed_once<S: Steps>(
    steps: S,
    kex: KexRetry,
    key: &Key,
    known_path: &Path,
    control: &Control,
) -> (io::Result<(SshConnection, Vec<u8>)>, Observed) {
    let seen = Arc::new(Mutex::new(Observed::default()));
    let mut network = Network {
        steps: Recording {
            inner: steps,
            seen: seen.clone(),
        },
        kex,
        credential_offered: &|| false,
    };
    let result = establish_inner(
        key,
        &key.host,
        known_path,
        control,
        read_regular,
        resolve,
        &mut network,
    );
    let observed = seen.lock().unwrap().clone();
    (result, observed)
}

/// The setup, made again (at most [`TRIES`] times) only when it ended on a triple key exchange failure. Returns the
/// last result and what every try did, so that a row can print the codes of a failure it does not expect.
pub(crate) fn establish_retrying_triple_kex<S: Steps>(
    mut make_steps: impl FnMut() -> S,
    kex: KexRetry,
    key: &Key,
    known_path: &Path,
    control: &Control,
) -> (io::Result<(SshConnection, Vec<u8>)>, Vec<Observed>) {
    let mut tries = Vec::new();
    loop {
        let (result, observed) =
            establish_observed_once(make_steps(), kex, key, known_path, control);
        let again = result.is_err() && observed.is_triple_kex_failure() && tries.len() + 1 < TRIES;
        tries.push(observed);
        if !again {
            return (result, tries);
        }
    }
}

/// [`establish_retrying_triple_kex`] with the real steps and the platform's policy: what production does.
pub(crate) fn establish_production_retrying_triple_kex(
    key: &Key,
    known_path: &Path,
    control: &Control,
) -> (io::Result<(SshConnection, Vec<u8>)>, Vec<Observed>) {
    establish_retrying_triple_kex(|| Native, KexRetry::PLATFORM, key, known_path, control)
}
