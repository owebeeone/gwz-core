//! The Windows twin of [`super::key_fixture`] (GwzTransportWindowsParityPlan.md, step 1.7): an agent that lists the
//! keys a test names, in order, and signs as each key's entry says, served on a named pipe.
//!
//! The Unix `KeyAgent` is `key_agent.py` in front of a private `ssh-agent`, and starts two processes, which its
//! startup-failure row terminates and reaps. This one is [`PipeAgent`] with a thread and a pipe handle in the test
//! process, so its startup-failure row is that a start that fails part way leaves no pipe behind. What it cannot
//! represent stays Unix: security keys and certificates (`Sign::Sk`), the DSA key (`Sign::Dsa`), and the SHA-1
//! answer to an `rsa-sha2` request (libssh2 on Windows offers only `rsa-sha2-512`, and the agent signs with
//! `ring`, which has no SHA-1 RSA).
use super::agent_fixture_pipe::{
    Behavior, Identity, PipeAgent, PipeChannel, Request, Stage, pipe_name,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{fs, path::Path};

/// How the agent answers a sign request for one listed key.
pub(crate) enum Sign<'a> {
    /// The agent signs, for this base64 public key blob.
    Upstream(&'a str),
    /// Every request for this key is refused, as for an absent security key.
    Absent(&'a str),
    /// The first reply is a malformed SHA-1 signature; later replies sign.
    MalformedRsa { key: &'a str, length: usize },
}

pub(crate) struct KeyAgent {
    agent: PipeAgent,
    /// Each listed key's base64 public blob, in list order.
    pub(crate) blobs: Vec<String>,
}

impl KeyAgent {
    /// An agent in `dir` that lists `keys`, in order, signing with the `private` key files (PEM RSA keys).
    pub(crate) fn start(dir: &Path, private: &[&Path], keys: &[Sign<'_>]) -> Self {
        Self::start_observing(dir, private, keys, |_| {})
    }

    fn start_observing(
        dir: &Path,
        private: &[&Path],
        keys: &[Sign<'_>],
        observe: impl FnMut(Stage),
    ) -> Self {
        let identities: Vec<Identity> = private
            .iter()
            .map(|path| Identity::from_key_file(path))
            .collect();
        let find = |blob: &str| {
            identities
                .iter()
                .find(|identity| STANDARD.encode(identity.blob()) == blob)
                .unwrap_or_else(|| panic!("no private key for the listed key {blob}"))
                .clone()
        };
        let entries: Vec<_> = keys
            .iter()
            .map(|key| match key {
                Sign::Upstream(blob) => (find(blob), Behavior::Signs),
                Sign::Absent(blob) => (find(blob), Behavior::Refuses),
                Sign::MalformedRsa { key, length } => {
                    (find(key), Behavior::MalformedFirst(*length))
                }
            })
            .collect();
        let blobs = entries
            .iter()
            .map(|(identity, _)| STANDARD.encode(identity.blob()))
            .collect();
        let agent = PipeAgent::start_observing(&pipe_name(dir), entries, observe).unwrap();
        Self { agent, blobs }
    }

    pub(crate) fn connect(&self) -> PipeChannel {
        self.agent.connect()
    }

    /// The requests the agent has seen, in order: `list`, or `sign:<listed key's index>:<flags>`.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.agent
            .requests()
            .into_iter()
            .map(|request| match request {
                Request::List => "list".to_owned(),
                Request::Sign { key, flags } => format!("sign:{key}:{flags}"),
            })
            .collect()
    }
}

/// The type and base64 blob of the public key `dir/name.pub`.
pub(crate) fn public(dir: &Path, name: &str) -> (String, String) {
    let text = fs::read_to_string(dir.join(format!("{name}.pub"))).unwrap();
    let mut fields = text.split_whitespace();
    (
        fields.next().unwrap().to_owned(),
        fields.next().unwrap().to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::endpoint::{agent_client::Agent, agent_job::Job, ssh_fixture};
    use std::{
        io,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Arc,
        task::{Context, Poll, Waker},
        time::{Duration, Instant},
    };

    fn key(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, String) {
        let path = dir.join(name);
        ssh_fixture::run_fixture_key(&path);
        let (_, blob) = public(dir, name);
        (path, blob)
    }

    fn run<T: Send + 'static>(
        body: impl FnOnce(Arc<crate::git::endpoint::agent_job::Control>) -> io::Result<T>
        + Send
        + 'static,
    ) -> io::Result<T> {
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(10)),
            Duration::from_secs(1),
            body,
        )
        .unwrap();
        let until = Instant::now() + Duration::from_secs(12);
        loop {
            if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                return result;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_agent_lists_the_keys_it_is_told_to_in_that_order() {
        let dir = tempfile::tempdir().unwrap();
        let (first, first_blob) = key(dir.path(), "first");
        let (second, second_blob) = key(dir.path(), "second");
        let agent = KeyAgent::start(
            dir.path(),
            &[&first, &second],
            &[Sign::Upstream(&second_blob), Sign::Upstream(&first_blob)],
        );
        assert_eq!(agent.blobs, vec![second_blob.clone(), first_blob.clone()]);
        let channel = agent.connect();
        let listed = run(move |control| Agent::new(channel, control).identities()).unwrap();
        assert_eq!(
            listed,
            vec![
                STANDARD.decode(&second_blob).unwrap(),
                STANDARD.decode(&first_blob).unwrap()
            ]
        );
        assert_eq!(agent.requests(), vec!["list".to_owned()]);
    }

    #[test]
    fn an_absent_key_is_refused_every_time_and_a_malformed_first_reply_is_followed_by_signing() {
        let dir = tempfile::tempdir().unwrap();
        let (absent, absent_blob) = key(dir.path(), "absent");
        let (rsa, rsa_blob) = key(dir.path(), "rsa");
        let agent = KeyAgent::start(
            dir.path(),
            &[&absent, &rsa],
            &[
                Sign::Absent(&absent_blob),
                Sign::MalformedRsa {
                    key: &rsa_blob,
                    length: 3,
                },
            ],
        );
        let (absent_key, rsa_key) = (
            STANDARD.decode(&absent_blob).unwrap(),
            STANDARD.decode(&rsa_blob).unwrap(),
        );
        let channel = agent.connect();
        let (first, second, refused) = run(move |control| {
            let mut agent = Agent::new(channel, control);
            let refused = (
                agent.sign(&absent_key, b"d", 4)?,
                agent.sign(&absent_key, b"d", 4)?,
            );
            Ok((
                agent.sign(&rsa_key, b"d", 4)?,
                agent.sign(&rsa_key, b"d", 4)?,
                refused,
            ))
        })
        .unwrap();
        assert_eq!(refused, (None, None));
        assert!(first.is_some() && second.is_some());
        assert_ne!(first, second, "the first reply was not malformed");
        assert_eq!(
            agent.requests(),
            vec!["sign:0:4", "sign:0:4", "sign:1:4", "sign:1:4"]
        );
    }

    #[test]
    fn a_start_that_fails_part_way_leaves_no_pipe_behind() {
        for stage in [Stage::PipeCreated, Stage::Serving] {
            let dir = tempfile::tempdir().unwrap();
            let (private, blob) = key(dir.path(), "k");
            let name = pipe_name(dir.path());
            let failed = catch_unwind(AssertUnwindSafe(|| {
                KeyAgent::start_observing(
                    dir.path(),
                    &[&private],
                    &[Sign::Upstream(&blob)],
                    |reached| {
                        assert!(reached != stage, "injected startup failure");
                    },
                )
            }));
            assert!(failed.is_err());
            assert_eq!(
                PipeChannel::open(&name)
                    .err()
                    .expect("the pipe is gone")
                    .kind(),
                io::ErrorKind::NotFound
            );
        }
    }
}
