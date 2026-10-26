//! The SSH adapter's cross-binding reuse proofs (plan step 1.6; contract section 7.1; dev-docs/GwzConnectionReuseDesign.md
//! section 4). The host leases an idle connection another binding last used only after `authorize_reuse` reports
//! `Authorized` for the asking binding. The worker answers from the connection's reuse evidence (what its setup
//! authenticated with, `Resource::reuse_evidence`): an agent key it must hold now (a fresh signature), a trusted host
//! key (`known_hosts` read again under the host as the pool keys it and as the URL wrote it), each proved once per binding
//! for the adapter's life. A connection that keeps no evidence (a password's, a helper's) is `Unprovable`, as is a proof
//! that could not finish: the connection stays idle for the binding that used it. A refused key or an untrusted host key
//! is `Revoked`, and the idle connections that share that key or that host and host key are retired and reported
//! `IdleClosed` after it.
//!
//! Leases of one binding that need the same proof share one job (contract section 7.1: concurrent waiters in a binding
//! share the proof job), keyed by the binding, the agent key, the host, port and host key and the host's spellings. The
//! job runs under the earlier of the first waiter's opening deadline (`ReuseCommand::remaining_ms`) and the adapter's
//! connect bound, and never longer: a waiter that joins later is not given more time, so it can only be answered
//! `Unprovable` early, which fails closed. The host ends a waiter whose own deadline passes (`cancel_reuse`), which
//! removes that waiter only; the job goes on while any waiter remains, and a pass is recorded for the binding even when
//! none does.
use super::{Entry, Phase, Report, Worker, disposing};
use crate::git::endpoint::{
    agent_auth::{Authenticator, Check, Possession, Verdict},
    agent_job::Job,
    ssh_pool::{Connector, Proofs, Resource},
    ssh_worker::ChannelResource,
};
use gwz_connection::{BindingId, Completion, ProofId, ReuseCommand, ReuseVerdict};
use gwz_transport::pool::ConnectionId;
use std::{
    collections::BTreeSet,
    task::{Context, Poll},
};

/// One proof job, and the leases that wait for it.
struct Running {
    /// The binding, as the proof records name it.
    binding: String,
    /// The agent key whose possession is being proved, and the host, port and host key whose trust is.
    key: Option<Vec<u8>>,
    host: Option<(String, u16, Vec<u8>)>,
    /// The host names the trust check looks the host key up under.
    names: Vec<String>,
    job: Job<Verdict>,
    /// The leases that wait for the verdict, each by its own command. A lease the host cancelled is gone from here;
    /// the job runs on and a pass is still recorded.
    commands: Vec<ReuseCommand>,
}

#[derive(Default)]
pub(super) struct Proving {
    proofs: Proofs,
    running: Vec<Running>,
}
impl Proving {
    /// The host no longer waits for `proof`: its lease leaves the job, which goes on for the others.
    pub(super) fn cancel(&mut self, proof: ProofId) {
        for running in &mut self.running {
            running.commands.retain(|command| command.proof != proof);
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
    /// The connections a running proof waits on, which a failed proof of another binding does not retire.
    fn busy(&self) -> BTreeSet<ConnectionId> {
        self.running
            .iter()
            .flat_map(|running| running.commands.iter().map(|command| command.connection))
            .collect()
    }
}

/// How the proof records name a binding: by the fields of its identifier, not by its `Debug` text.
fn binding_name(binding: BindingId) -> String {
    format!("{:032x}.{}", binding.host().get(), binding.serial())
}

/// The names a trust check looks `key_host` up under: the host lowercased, as the pool keys it, and the host as the URL
/// wrote it when that spells the same host differently (TR2.18).
fn trust_names(key_host: &str, written: &str) -> Vec<String> {
    let lowered = key_host.to_ascii_lowercase();
    let mut names = vec![lowered.clone()];
    if written != lowered && written.eq_ignore_ascii_case(&lowered) {
        names.push(written.to_owned());
    }
    names
}

/// The deadline of a proof started at `now` (the worker's clock, milliseconds): the earlier of the adapter's connect
/// bound (none when 0) and what is left of the opening's own deadline. A proof never outlasts its opening.
fn proof_deadline(now: u64, connect_timeout_ms: u64, remaining_ms: u64) -> u64 {
    let opening = now.saturating_add(remaining_ms);
    if connect_timeout_ms == 0 {
        opening
    } else {
        opening.min(now.saturating_add(connect_timeout_ms))
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
    /// Starts the proof `command` asks for, joins the job that already runs it for the binding, or answers at once.
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
        let binding = binding_name(command.binding);
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
        let names = trust_names(&key.host, &evidence.written_host);
        if let Some(running) = self.reuse.running.iter_mut().find(|running| {
            running.binding == binding
                && running.key == possession
                && running.host == host
                && running.names == names
        }) {
            running.commands.push(command);
            return;
        }
        let check = Check {
            possession: possession.clone(),
            trust: host
                .as_ref()
                .map(|(_, port, host_key)| (names.clone(), *port, host_key.clone())),
        };
        let deadline = proof_deadline(now, self.settings.connect_timeout_ms, command.remaining_ms);
        let started = evidence
            .authority
            .start(check, Some(deadline), self.settings.io_timeout_ms);
        match started {
            Ok(job) => self.reuse.running.push(Running {
                binding,
                key: possession,
                host,
                names,
                job,
                commands: vec![command],
            }),
            // No proof job can start (a full job budget): nothing is proved, the connection stays idle.
            Err(_) => self.report(command, ReuseVerdict::Unprovable),
        }
    }
    /// Whether `connection` is still an idle connection the worker holds, which only a lease of it can start an exchange on.
    fn still_idle(&self, connection: ConnectionId) -> bool {
        self.entries
            .get(&connection)
            .is_some_and(|entry| matches!(entry.phase, Phase::Idle))
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
            // The job ended on its own clocks or was cancelled: nothing is proved.
            let Ok(verdict) = result else {
                for command in &running.commands {
                    self.report(*command, ReuseVerdict::Unprovable);
                }
                continue;
            };
            let refused = verdict.possession == Some(Possession::Refused);
            let untrusted = verdict.trusted == Some(false);
            if refused || untrusted {
                // A proof the host no longer waits for retires nothing.
                if !running.commands.is_empty() {
                    self.retire_matching(&running, refused, untrusted, now);
                    for command in &running.commands {
                        self.report(*command, ReuseVerdict::Revoked);
                    }
                }
            } else if verdict.possession == Some(Possession::Unprovable) {
                for command in &running.commands {
                    self.report(*command, ReuseVerdict::Unprovable);
                }
            } else {
                // A passed proof is the binding's, whether or not a lease that asked for it still waits.
                self.reuse.record(&running);
                for command in &running.commands {
                    // A connection that went idle-lost or was retired while the proof ran serves nobody.
                    let verdict = if self.still_idle(command.connection) {
                        ReuseVerdict::Authorized
                    } else {
                        ReuseVerdict::Unprovable
                    };
                    self.report(*command, verdict);
                }
            }
        }
    }
    /// A failed proof retires every other idle connection with the key (possession) or the host and host key (trust)
    /// that failed; each is disposed of and reported `IdleClosed`. A connection a lease waits on (this job's, or another
    /// proof's) is the host's, and one the host has leased for an exchange is not idle for retirement's purpose
    /// (`Claims`): neither is retired.
    fn retire_matching(&mut self, running: &Running, refused: bool, untrusted: bool, now: u64) {
        let settings = self.settings;
        let mut busy = self.reuse.busy();
        busy.extend(running.commands.iter().map(|command| command.connection));
        for (id, entry) in &mut self.entries {
            if busy.contains(id) {
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
            if (same_key || same_host) && self.shared.claims.retire(*id) {
                *phase = disposing(Report::IdleClosed, false, now, &settings);
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
