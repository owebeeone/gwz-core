//! Phase 1 remediation R3c (State P3-2(g)): the reuse proof of the two key types `reuse_auth` does not hold. A second
//! binding's lease of an idle connection that an OpenSSH user certificate, or a security key
//! (`sk-ssh-ed25519@openssh.com`), authenticated is authorized by the agent's fresh possession signature with that
//! key (contract §7.1), and refused when the agent lacks the key. The server is the loopback `sshd` fixture, which
//! trusts a fixture CA for the certificate and authorizes the security key. The agent is the TR2.8 key agent
//! (`key_fixture` on Unix, `key_fixture_pipe` on Windows): it lists the held key first and a plain fallback key
//! second, signs a certificate with the private agent behind it and a security key with its software authenticator
//! (SHA-256(application) || flags || counter || SHA-256(data), signed by the plain key that backs it), and logs every
//! request (`reuse_agent::ProofAgent`). Every key is made under the row's temporary directory; nothing reads the
//! user's agent or keys.
use super::{
    key_material::{certify, keygen, public, security_key},
    local_endpoint,
    reuse_agent::{ProofAgent, Sign},
    reuse_auth::{exchange_on, wait_counts},
};
use crate::git::endpoint::{fixture_host, ssh_fixture::SshdFixture, ssh_worker::Endpoint};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gwz_transport::pool::Key;
use std::{fs, io};

/// The key type under test.
#[derive(Clone, Copy)]
enum Held {
    /// An Ed25519 key's OpenSSH user certificate, signed by the fixture CA for the server's user.
    Certificate,
    /// A software `sk-ssh-ed25519@openssh.com` key.
    SecurityKey,
}

/// A server that accepts the held key (through its CA, or as an authorized key) and a plain fallback, and the agent
/// listing the held key at index 0 and the fallback at index 1.
struct Rig {
    // The agent outlives the endpoint's connections and the server outlives both.
    endpoint: Endpoint,
    agent: ProofAgent,
    ssh: SshdFixture,
    held: Vec<u8>,
    _material: tempfile::TempDir,
}
impl Rig {
    fn new(kind: Held) -> Self {
        let material = tempfile::tempdir().unwrap();
        let dir = material.path();
        let ca = keygen(dir, "ca", "ed25519", None);
        let trusted = fixture_host::config_value(&format!("{}.pub", ca.display()));
        let ssh = SshdFixture::with_config(&format!("TrustedUserCAKeys {trusted}\n"));
        let key = keygen(dir, "key", "ed25519", None);
        let plain = public(dir, "key");
        let fallback = keygen(dir, "fallback", "ed25519", None);
        let spare = public(dir, "fallback");
        let (held, backing) = match kind {
            Held::Certificate => (certify(dir, &ca, "key", &plain, &ssh.user), None),
            Held::SecurityKey => (security_key(&plain.1), Some(plain.1.clone())),
        };
        let mut authorized = format!("{} {}\n", spare.0, spare.1);
        if backing.is_some() {
            authorized.push_str(&format!("{} {}\n", held.0, held.1));
        }
        fs::write(ssh.temp.path().join("authorized_keys"), authorized).unwrap();
        let signer = match &backing {
            None => Sign::Upstream(&held.1),
            Some(backing) => Sign::Sk {
                key: &held.1,
                backing,
            },
        };
        let agent = ProofAgent::start(dir, &[&key, &fallback], &[signer, Sign::Upstream(&spare.1)]);
        let endpoint = local_endpoint::connect(
            super::reuse_auth::config(),
            ssh.known_hosts.clone(),
            agent.source(),
            3000,
        );
        Self {
            endpoint,
            agent,
            ssh,
            held: STANDARD.decode(&held.1).unwrap(),
            _material: material,
        }
    }
    fn exchange(&self, binding: &str) -> io::Result<gwz_transport::protocol::Opened> {
        let key = Key::ssh(&self.ssh.user, "127.0.0.1", self.ssh.port);
        exchange_on(
            &self.endpoint,
            key,
            self.ssh.repository.to_str().unwrap(),
            binding,
            None,
        )
    }
    /// The sign requests the agent has seen for the held key (listed first) and for the fallback (listed second).
    fn signs(&self) -> (usize, usize) {
        let requests = self.agent.requests();
        let count = |wanted: &str| {
            requests
                .iter()
                .filter(|request| request.starts_with(wanted))
                .count()
        };
        (count("sign:0:"), count("sign:1:"))
    }
}

/// Binding B leases the idle connection of binding A, which `kind` authenticated: B proves possession of that key with
/// one fresh signature the agent makes with it, and A's connection serves B and keeps serving it without asking again.
fn another_binding_proves_possession_of(kind: Held) {
    let rig = Rig::new(kind);
    let first = rig
        .exchange("binding-a")
        .unwrap_or_else(|e| panic!("{e:?}\n{}", rig.ssh.log()));
    assert_eq!(
        rig.signs(),
        (1, 0),
        "A's connection authenticated with the held key: {:?}",
        rig.agent.requests()
    );
    let other = rig.exchange("binding-b").unwrap();
    assert!(other.reused, "a proven connection serves the other binding");
    assert_eq!(first.connection_id, other.connection_id);
    assert_eq!(
        rig.signs(),
        (2, 0),
        "exactly one fresh possession signature, by the held key"
    );
    let again = rig.exchange("binding-b").unwrap();
    assert!(again.reused);
    assert_eq!(
        rig.signs(),
        (2, 0),
        "the binding's proof is kept for its life"
    );
}

/// The agent no longer holds `kind`'s key when binding B leases A's idle connection: the proof is refused, A's
/// connection is retired, and B opens a connection of its own (on Unix the agent keeps refusing the held key, so B
/// authenticates it with the fallback; the pipe agent refuses only the one request, so B's authenticates with the
/// held key again).
fn an_agent_without_the_key_gives_the_other_binding_a_fresh_connection(kind: Held) {
    let rig = Rig::new(kind);
    let first = rig
        .exchange("binding-a")
        .unwrap_or_else(|e| panic!("{e:?}\n{}", rig.ssh.log()));
    rig.agent.withdraw(&rig.held);
    let other = rig.exchange("binding-b").unwrap();
    assert!(
        !other.reused,
        "authority the agent no longer holds is not reused"
    );
    assert_ne!(first.connection_id, other.connection_id);
    wait_counts(
        &rig.endpoint,
        "only binding B's connection is left",
        |counts| counts.idle == 1 && counts.total() == 1,
    );
}

#[test]
fn p1r_r3c_a_certificates_proof_of_possession_authorizes_the_other_binding() {
    another_binding_proves_possession_of(Held::Certificate);
}

#[test]
fn p1r_r3c_an_agent_without_the_certificate_gives_the_other_binding_a_fresh_connection() {
    an_agent_without_the_key_gives_the_other_binding_a_fresh_connection(Held::Certificate);
}

#[test]
fn p1r_r3c_a_security_keys_proof_of_possession_authorizes_the_other_binding() {
    another_binding_proves_possession_of(Held::SecurityKey);
}

#[test]
fn p1r_r3c_an_agent_without_the_security_key_gives_the_other_binding_a_fresh_connection() {
    an_agent_without_the_key_gives_the_other_binding_a_fresh_connection(Held::SecurityKey);
}
