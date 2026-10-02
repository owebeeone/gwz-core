//! TR2.8's key-type list without a server: which agent keys the transport
//! offers, the algorithm and flags each method asks for, the shape each
//! signature must have, the one `ssh-rsa` downgrade, and an agent's refusal.
use crate::git::endpoint::{
    agent_client::{Agent, Channel},
    agent_job::{Control, Job},
    agent_keys::{self, KeyType, Signed},
};
use std::{
    io::{self, Cursor, Read, Write},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

fn blob(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field);
    }
    out
}

fn kind(name: &str) -> KeyType {
    KeyType::of(&blob(&[name.as_bytes()])).unwrap()
}

/// The fields an OpenSSH certificate carries after its key's.
fn certificate_tail() -> Vec<u8> {
    let mut tail = [0; 12].to_vec();
    tail.extend(blob(&[b"id", b""]));
    tail.extend_from_slice(&[0; 16]);
    tail.extend(blob(&[b"", b"", b"", b"ca", b"signature"]));
    tail
}

#[test]
fn every_listed_type_and_its_certificate_is_offered_and_any_other_is_skipped() {
    for name in [
        "ssh-ed25519",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "ssh-rsa",
        "ssh-dss",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
        "ssh-ed25519-cert-v01@openssh.com",
        "ecdsa-sha2-nistp521-cert-v01@openssh.com",
        "ssh-rsa-cert-v01@openssh.com",
        "sk-ssh-ed25519-cert-v01@openssh.com",
        "sk-ecdsa-sha2-nistp256-cert-v01@openssh.com",
    ] {
        assert!(KeyType::of(&blob(&[name.as_bytes()])).is_some(), "{name}");
    }
    for name in [
        "ssh-dss-cert-v01@openssh.com",
        "ssh-xmss@openssh.com",
        "rsa-sha2-512",
        "webauthn-sk-ecdsa-sha2-nistp256@openssh.com",
        "ssh-ed25519-cert-v01@openssh.co",
        "",
    ] {
        assert!(KeyType::of(&blob(&[name.as_bytes()])).is_none(), "{name}");
    }
    assert!(KeyType::of(&[0, 0, 0, 9, b's']).is_none());
}

#[test]
fn each_method_asks_for_its_algorithm_and_rsa_for_sha2_by_flag() {
    for (key, method, algorithm) in [
        ("ssh-rsa", "rsa-sha2-512", "rsa-sha2-512"),
        ("ssh-rsa", "ssh-rsa", "ssh-rsa"),
        (
            "ssh-rsa-cert-v01@openssh.com",
            "rsa-sha2-256-cert-v01@openssh.com",
            "rsa-sha2-256",
        ),
        (
            "ssh-rsa-cert-v01@openssh.com",
            "ssh-rsa-cert-v01@openssh.com",
            "ssh-rsa",
        ),
        (
            "ecdsa-sha2-nistp384-cert-v01@openssh.com",
            "ecdsa-sha2-nistp384-cert-v01@openssh.com",
            "ecdsa-sha2-nistp384",
        ),
        (
            "sk-ssh-ed25519-cert-v01@openssh.com",
            "sk-ssh-ed25519-cert-v01@openssh.com",
            "sk-ssh-ed25519@openssh.com",
        ),
    ] {
        assert_eq!(
            kind(key).algorithm(method).unwrap(),
            algorithm,
            "{key} {method}"
        );
    }
    for (key, method) in [
        ("ssh-rsa", "ssh-ed25519"),
        ("ssh-rsa", "rsa-sha2-512-cert-v01@openssh.com"),
        ("ssh-rsa-cert-v01@openssh.com", "rsa-sha2-512"),
        ("ssh-ed25519", "ssh-rsa"),
        ("ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384"),
    ] {
        assert!(kind(key).algorithm(method).is_err(), "{key} {method}");
    }
    let flags: Vec<_> = ["rsa-sha2-256", "rsa-sha2-512", "ssh-rsa", "ssh-ed25519"]
        .map(agent_keys::flags)
        .into();
    assert_eq!(flags, [2, 4, 0, 0]);
}

#[test]
fn each_signature_is_checked_against_its_key_and_a_security_key_keeps_its_counter() {
    let ed25519 = blob(&[b"ssh-ed25519", &[7; 32]]);
    let check = |key: &[u8], name: &str, reply: &[u8]| {
        let (kind, method) = (KeyType::of(key).unwrap(), name.to_owned());
        let algorithm = kind.algorithm(&method).unwrap();
        kind.signature(key, &method, algorithm, reply)
    };
    let raw = |result: io::Result<Signed>| match result {
        Ok(Signed::Signature(bytes)) => Some(bytes),
        _ => None,
    };
    assert_eq!(
        raw(check(
            &ed25519,
            "ssh-ed25519",
            &blob(&[b"ssh-ed25519", &[1; 64]])
        )),
        Some(vec![1; 64])
    );
    for reply in [
        blob(&[b"ssh-ed25519", &[1; 63]]),
        blob(&[b"ssh-ed25519", &[1; 64], b""]),
        blob(&[b"ssh-ed2551X", &[1; 64]]),
    ] {
        assert!(check(&ed25519, "ssh-ed25519", &reply).is_err());
    }
    // A certificate: the base type's signature, its key read past the nonce.
    let mut certificate = blob(&[b"ssh-ed25519-cert-v01@openssh.com", b"nonce", &[7; 32]]);
    certificate.extend(certificate_tail());
    let reply = blob(&[b"ssh-ed25519", &[1; 64]]);
    assert!(
        raw(check(
            &certificate,
            "ssh-ed25519-cert-v01@openssh.com",
            &reply
        ))
        .is_some()
    );
    assert!(
        check(
            &certificate[..certificate.len() - 1],
            "ssh-ed25519-cert-v01@openssh.com",
            &reply
        )
        .is_err()
    );
    // A security key: libssh2 sends the signature string, flags and counter unframed.
    let sk = blob(&[b"sk-ssh-ed25519@openssh.com", &[7; 32], b"ssh:"]);
    let mut reply = blob(&[b"sk-ssh-ed25519@openssh.com", &[1; 64]]);
    reply.extend_from_slice(&[1, 0, 0, 0, 9]);
    assert_eq!(
        raw(check(&sk, "sk-ssh-ed25519@openssh.com", &reply)),
        Some(reply[30..].to_vec())
    );
    assert!(check(&sk, "sk-ssh-ed25519@openssh.com", &reply[..reply.len() - 1]).is_err());
    // ECDSA: two positive integers, each no longer than the field.
    let p256 = blob(&[
        b"ecdsa-sha2-nistp256",
        b"nistp256",
        &[[4].as_slice(), &[9; 64]].concat(),
    ]);
    let pair = |r: &[u8], s: &[u8]| blob(&[b"ecdsa-sha2-nistp256", &blob(&[r, s])]);
    assert!(
        raw(check(
            &p256,
            "ecdsa-sha2-nistp256",
            &pair(&[0, 0x80, 1], &[5; 32])
        ))
        .is_some()
    );
    for reply in [
        pair(&[0x80], &[5]),
        pair(&[5; 33], &[5]),
        pair(&[0, 5], &[5]),
        pair(&[], &[5]),
    ] {
        assert!(check(&p256, "ecdsa-sha2-nistp256", &reply).is_err());
    }
    // RSA: as long as the modulus.
    let rsa = blob(&[
        b"ssh-rsa",
        &[1, 0, 1],
        &[[0].as_slice(), &[0xc5; 256]].concat(),
    ]);
    assert!(
        raw(check(
            &rsa,
            "rsa-sha2-512",
            &blob(&[b"rsa-sha2-512", &[3; 256]])
        ))
        .is_some()
    );
    assert!(check(&rsa, "rsa-sha2-512", &blob(&[b"rsa-sha2-512", &[3; 255]])).is_err());
}

#[test]
fn only_an_ssh_rsa_answer_to_a_sha2_request_is_a_downgrade() {
    let rsa = blob(&[b"ssh-rsa", &[1, 0, 1], &[0xc5; 256]]);
    let kind = KeyType::of(&rsa).unwrap();
    let ssh_rsa = blob(&[b"ssh-rsa", &[3; 256]]);
    for method in ["rsa-sha2-512", "rsa-sha2-256"] {
        assert!(matches!(
            kind.signature(&rsa, method, method, &ssh_rsa),
            Ok(Signed::Downgraded)
        ));
    }
    assert!(matches!(
        kind.signature(&rsa, "ssh-rsa", "ssh-rsa", &ssh_rsa),
        Ok(Signed::Signature(_))
    ));
    let other = blob(&[b"rsa-sha2-256", &[3; 256]]);
    assert!(
        kind.signature(&rsa, "rsa-sha2-512", "rsa-sha2-512", &other)
            .is_err()
    );
    let ed25519 = blob(&[b"ssh-ed25519", &[7; 32]]);
    let kind = KeyType::of(&ed25519).unwrap();
    assert!(
        kind.signature(
            &ed25519,
            "ssh-ed25519",
            "ssh-ed25519",
            &blob(&[b"ssh-rsa", &[3; 64]])
        )
        .is_err()
    );
}

#[test]
fn malformed_rsa_downgrades_fail_for_plain_and_certified_keys() {
    for certificate in [false, true] {
        let mut key = if certificate {
            blob(&[
                b"ssh-rsa-cert-v01@openssh.com",
                b"nonce",
                &[1, 0, 1],
                &[0xc5; 256],
            ])
        } else {
            blob(&[b"ssh-rsa", &[1, 0, 1], &[0xc5; 256]])
        };
        if certificate {
            key.extend(certificate_tail());
        }
        let kind = KeyType::of(&key).unwrap();
        let method = if certificate {
            "rsa-sha2-512-cert-v01@openssh.com"
        } else {
            "rsa-sha2-512"
        };
        let check = |key: &[u8], reply: &[u8]| kind.signature(key, method, "rsa-sha2-512", reply);
        for size in [0, 1, 255, 257] {
            assert!(
                check(&key, &blob(&[b"ssh-rsa", &vec![0xff; size]])).is_err(),
                "certificate={certificate}, size={size}"
            );
        }
        let valid = blob(&[b"ssh-rsa", &[3; 256]]);
        assert!(matches!(check(&key, &valid), Ok(Signed::Downgraded)));
        assert!(check(&key, &valid[..valid.len() - 1]).is_err());
        assert!(check(&key, &[valid.clone(), vec![0]].concat()).is_err());
        assert!(check(&key[..key.len() - 1], &valid).is_err());
    }
}

struct Script(Cursor<Vec<u8>>);
impl Read for Script {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
impl Write for Script {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Channel for Script {
    fn wait(&mut self, _: bool, control: &Control) -> io::Result<()> {
        control.check()
    }
}

#[test]
fn a_refusal_is_none_and_leaves_the_agent_usable_for_the_next_key() {
    let mut replies = vec![0, 0, 0, 1, 5];
    let signed = [
        [14].as_slice(),
        &blob(&[&blob(&[b"ssh-ed25519", &[1; 64]])]),
    ]
    .concat();
    replies.extend_from_slice(&(signed.len() as u32).to_be_bytes());
    replies.extend(signed);
    let mut job = Job::start(None, Duration::from_secs(1), move |control| {
        let mut agent = Agent::new(Script(Cursor::new(replies)), control);
        assert_eq!(agent.sign(b"absent", b"data", 0)?, None);
        assert!(agent.sign(b"present", b"data", 0)?.is_some());
        Ok(())
    })
    .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return result.unwrap();
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
