//! The SSH adapter's cross-binding reuse proofs (plan step 1.6; contract section 7.1; dev-docs/GwzConnectionReuseDesign.md
//! section 4). The host leases an idle connection another binding last used only after `authorize_reuse` reports
//! `Authorized` for the asking binding. The worker answers from the connection's reuse evidence (what its setup
//! authenticated with, `Resource::reuse_evidence`): an agent key it must hold now (a fresh signature), a trusted host
//! key (`known_hosts` read again), each proved once per binding for the adapter's life. A connection that keeps no
//! evidence (a password's, a helper's) is `Unprovable`, as is a proof that could not finish: the connection stays idle
//! for the binding that used it. A refused key or an untrusted host key is `Revoked`, and the idle connections that
//! share that key or that host and host key are retired and reported `IdleClosed` after it.
use super::{Entry, Phase, Report, Worker, disposing};
use crate::git::endpoint::{
    agent_auth::{Authenticator, Check, Possession, Verdict},
    agent_job::Job,
    ssh_pool::{Connector, Proofs, Resource},
    ssh_worker::ChannelResource,
};
use gwz_connection::{Completion, ProofId, ReuseCommand, ReuseVerdict};
use std::task::{Context, Poll};

/// One proof running for one lease.
struct Running {
    command: ReuseCommand,
    /// The binding, as the proof records name it.
    binding: String,
    /// The agent key whose possession is being proved, and the host, port and host key whose trust is.
    key: Option<Vec<u8>>,
    host: Option<(String, u16, Vec<u8>)>,
    job: Job<Verdict>,
    /// Whether the host still waits for the verdict. A proof it cancelled runs on, and a pass is still recorded.
    waited: bool,
}

#[derive(Default)]
pub(super) struct Proving {
    proofs: Proofs,
    running: Vec<Running>,
}
impl Proving {
    pub(super) fn cancel(&mut self, proof: ProofId) {
        for running in &mut self.running {
            if running.command.proof == proof {
                running.waited = false;
            }
        }
    }
    /// The adapter is disposing: every running proof ends.
    pub(super) fn clear(&mut self) {
        self.running.clear();
    }
    fn record(&mut self, running: &Running) {
        if let Some(key) = &running.key {
            self.proofs.add_key(&running.binding, key);
        }
        if let Some((name, port, host_key)) = &running.host {
            self.proofs
                .add_host(&running.binding, name, *port, host_key);
        }
    }
}

impl<C> Worker<C>
where
    C: Connector,
    C::Resource: ChannelResource,
{
    fn report(&self, command: ReuseCommand, verdict: ReuseVerdict) {
        self.shared.push(Completion::Reuse {
            connection: command.connection,
            proof: command.proof,
            verdict,
        });
    }
    /// Starts the proof `command` asks for, or answers it at once.
    pub(super) fn authorize(&mut self, command: ReuseCommand, now: u64) {
        let Some(entry) = self
            .entries
            .get(&command.connection)
            .filter(|entry| matches!(entry.phase, Phase::Idle))
        else {
            return self.report(command, ReuseVerdict::Unprovable);
        };
        let Some(evidence) = entry.resource.reuse_evidence() else {
            return self.report(command, ReuseVerdict::Unprovable);
        };
        let binding = format!("{:?}", command.binding);
        let key = entry.key.clone();
        let possession = match &evidence.authenticator {
            Authenticator::Agent(accepted) => {
                (!self.reuse.proofs.has_key(&binding, accepted)).then(|| accepted.clone())
            }
            // The pool leased the connection for an equal selected-key identity: its token names the same bytes.
            Authenticator::KeyFile(_) => None,
        };
        let host = (!self
            .reuse
            .proofs
            .has_host(&binding, &key.host, key.port, &evidence.host_key))
        .then(|| (key.host.clone(), key.port, evidence.host_key.clone()));
        if possession.is_none() && host.is_none() {
            return self.report(command, ReuseVerdict::Authorized);
        }
        let check = Check {
            possession: possession.clone(),
            trust: host.as_ref().map(|(name, port, host_key)| {
                (vec![name.to_ascii_lowercase()], *port, host_key.clone())
            }),
        };
        let deadline = (self.settings.connect_timeout_ms != 0)
            .then(|| now.saturating_add(self.settings.connect_timeout_ms));
        let started = evidence
            .authority
            .start(check, deadline, self.settings.io_timeout_ms);
        match started {
            Ok(job) => self.reuse.running.push(Running {
                command,
                binding,
                key: possession,
                host,
                job,
                waited: true,
            }),
            // No proof job can start (a full job budget): nothing is proved, the connection stays idle.
            Err(_) => self.report(command, ReuseVerdict::Unprovable),
        }
    }
    /// One pass over the running proofs (reuse design section 4's table).
    pub(super) fn settle_proofs(&mut self, cx: &mut Context<'_>, now: u64) {
        let mut index = 0;
        while index < self.reuse.running.len() {
            let Poll::Ready(result) = self.reuse.running[index].job.poll_result(cx) else {
                index += 1;
                continue;
            };
            let running = self.reuse.running.swap_remove(index);
            let command = running.command;
            // The job ended on its own clocks or was cancelled: nothing is proved.
            let Ok(verdict) = result else {
                if running.waited {
                    self.report(command, ReuseVerdict::Unprovable);
                }
                continue;
            };
            let refused = verdict.possession == Some(Possession::Refused);
            let untrusted = verdict.trusted == Some(false);
            if refused || untrusted {
                if running.waited {
                    self.retire_matching(&running, refused, untrusted, now);
                    self.report(command, ReuseVerdict::Revoked);
                }
            } else if verdict.possession == Some(Possession::Unprovable) {
                if running.waited {
                    self.report(command, ReuseVerdict::Unprovable);
                }
            } else {
                // A passed proof is the binding's, whether or not the lease that asked for it still waits.
                self.reuse.record(&running);
                if running.waited {
                    self.report(command, ReuseVerdict::Authorized);
                }
            }
        }
    }
    /// A failed proof retires every other idle connection with the key (possession) or the host and host key (trust)
    /// that failed; each is disposed of and reported `IdleClosed`.
    fn retire_matching(&mut self, running: &Running, refused: bool, untrusted: bool, now: u64) {
        let settings = self.settings;
        for (id, entry) in &mut self.entries {
            if *id == running.command.connection {
                continue;
            }
            let Entry {
                resource,
                phase,
                key,
            } = entry;
            if !matches!(phase, Phase::Idle) {
                continue;
            }
            let Some(evidence) = resource.reuse_evidence() else {
                continue;
            };
            let same_key = refused
                && running.key.as_ref().is_some_and(|accepted| {
                    evidence.authenticator == Authenticator::Agent(accepted.clone())
                });
            let same_host = untrusted
                && running.host.as_ref().is_some_and(|(name, port, host_key)| {
                    key.host == *name && key.port == *port && evidence.host_key == *host_key
                });
            if same_key || same_host {
                *phase = disposing(Report::IdleClosed, false, now, &settings);
            }
        }
    }
}
