//! Rows H1a-1 to H1a-10 of the HTTPS model draft r5 (section 5): the operation's credential record, at unit level.
//! Credentials are compared by the digest of their header; no header or secret is printed.
use super::{
    Config, OperationAuth, Secret,
    needs::{
        CHARGE, CREDENTIAL_BYTES, CREDENTIALS, Evidence, FINGERPRINTS, KEYS, NeedKey, Needs,
        RETAINED_BYTES, Recorded,
    },
    secret::parse_secret,
};
use crate::git::endpoint::https_destination::Destination;
use gwz_transport::{
    pool::Key,
    protocol::{ErrorCode, GitService, Scheme},
};
use sha2::{Digest, Sha256};

const FETCH: GitService = GitService::UploadPackAdvertisement;

fn secret(username: &str, password: &str) -> Secret {
    parse_secret(format!("username={username}\npassword={password}\n").as_bytes()).unwrap()
}
fn need(username: Option<&str>, host: &str, port: u16, path: &str, service: GitService) -> NeedKey {
    let mut key = Key::https(host, port);
    key.username = username.map(str::to_owned);
    let destination = Destination::parse(&format!("https://{host}:{port}{path}")).unwrap();
    NeedKey::new(&key, &destination, service)
}
fn at(host: &str, path: &str) -> NeedKey {
    need(None, host, 443, path, FETCH)
}
fn record() -> Needs {
    Needs::keyed(Some([7; 32]))
}
/// The digest of the header `username:password` gives, as the record's `Basic` answer is compared.
fn header(username: &str, password: &str) -> [u8; 32] {
    Sha256::digest(secret(username, password).header().as_bytes()).into()
}
fn answered(needs: &Needs, key: &NeedKey) -> Option<[u8; 32]> {
    match needs.answer(key) {
        Ok(Recorded::Basic(header)) => Some(Sha256::digest(header.as_bytes()).into()),
        _ => None,
    }
}
fn is(needs: &Needs, key: &NeedKey, expected: fn(&Recorded) -> bool) -> bool {
    needs.answer(key).as_ref().is_ok_and(expected)
}
fn rejected(recorded: &Recorded) -> bool {
    matches!(recorded, Recorded::Rejected)
}

#[test]
fn h1a_1_a_key_separates_scheme_username_host_port_path_and_class() {
    let base = at("h", "/repo");
    assert_eq!(
        base,
        need(None, "h", 443, "/repo", GitService::UploadPackExchange)
    );
    assert_eq!(base, at("h", "/repo/"));
    assert_ne!(base, at("h", "/repo.git"));
    assert_ne!(base, at("h", "/other"));
    assert_ne!(base, at("g", "/repo"));
    assert_ne!(base, need(None, "h", 8443, "/repo", FETCH));
    let alice = need(Some("alice"), "h", 443, "/repo", FETCH);
    assert_ne!(base, alice);
    assert_ne!(alice, need(Some("bob"), "h", 443, "/repo", FETCH));
    let push = need(
        None,
        "h",
        443,
        "/repo",
        GitService::ReceivePackAdvertisement,
    );
    assert_ne!(base, push);
    assert_eq!(
        push,
        need(None, "h", 443, "/repo", GitService::ReceivePackExchange)
    );
    let mut ssh = Key::https("h", 443);
    ssh.scheme = Scheme::Ssh;
    let destination = Destination::parse("https://h:443/repo").unwrap();
    assert_ne!(base, NeedKey::new(&ssh, &destination, FETCH));
}

#[test]
fn h1a_2_equal_bytes_share_one_owner_per_origin_and_never_across_origins() {
    let needs = record();
    let (one, two, other) = (at("a", "/one"), at("a", "/two"), at("b", "/one"));
    for key in [&one, &two, &other] {
        needs.record(key, secret("user", "pass")).unwrap();
    }
    assert!(needs.shares(&one, &two));
    assert!(!needs.shares(&one, &other));
    assert_eq!(needs.counts().credentials, 2);
    let alice = need(Some("alice"), "a", 443, "/one", FETCH);
    needs.record(&alice, secret("user", "pass")).unwrap();
    assert!(needs.shares(&one, &alice), "the origin has no username");
    for key in [&one, &two, &other, &alice] {
        assert_eq!(answered(&needs, key), Some(header("user", "pass")));
    }
}

#[test]
fn h1a_3_each_bound_ends_with_the_budget_code_and_earlier_entries_work() {
    assert_eq!(CREDENTIAL_BYTES, 16 * 1024);
    // The key table: the 512th key is stored, the 513th is not.
    let needs = record();
    for index in 0..KEYS - 1 {
        needs
            .bound(&at("h", &format!("/k{index}")), 1, Evidence::OnBound)
            .unwrap();
    }
    needs
        .record(&at("h", "/last"), secret("user", "pass"))
        .unwrap();
    let over = at("h", "/over");
    assert_eq!(
        needs.record(&over, secret("user", "pass")),
        Err(ErrorCode::Capacity)
    );
    assert_eq!(
        needs.bound(&over, 1, Evidence::Challenged),
        Err(ErrorCode::Capacity)
    );
    assert!(is(&needs, &over, |r| matches!(r, Recorded::Unrecorded)));
    assert_eq!(needs.counts().keys, KEYS);
    assert_eq!(
        answered(&needs, &at("h", "/last")),
        Some(header("user", "pass"))
    );
    // Distinct credentials: the 64th is retained, the 65th leaves its key `Unretained`; equal bytes still share.
    let needs = record();
    for index in 0..CREDENTIALS {
        needs
            .record(
                &at("h", &format!("/c{index}")),
                secret("user", &format!("p{index}")),
            )
            .unwrap();
    }
    let over = at("h", "/over");
    assert_eq!(
        needs.record(&over, secret("user", "p64")),
        Err(ErrorCode::Capacity)
    );
    assert!(is(&needs, &over, |r| matches!(r, Recorded::Unretained)));
    needs
        .record(&at("h", "/again"), secret("user", "p0"))
        .unwrap();
    assert_eq!(
        answered(&needs, &at("h", "/c0")),
        Some(header("user", "p0"))
    );
    assert_eq!(
        answered(&needs, &at("h", "/c63")),
        Some(header("user", "p63"))
    );
    // One credential: 16 KiB of username and password is retained, one byte more is not.
    let needs = record();
    let fits = "a".repeat(CREDENTIAL_BYTES - 1);
    needs.record(&at("h", "/fits"), secret("u", &fits)).unwrap();
    let over = at("h", "/over");
    let large = "b".repeat(CREDENTIAL_BYTES);
    assert_eq!(
        needs.record(&over, secret("u", &large)),
        Err(ErrorCode::Capacity)
    );
    assert!(is(&needs, &over, |r| matches!(r, Recorded::Unretained)));
    assert_eq!(
        answered(&needs, &at("h", "/fits")),
        Some(header("u", &fits))
    );
    // All retained bytes: sixteen 16 KiB credentials fill 256 KiB; one byte more is not retained.
    let needs = record();
    let filler = "c".repeat(CREDENTIAL_BYTES - 3);
    for index in 0..RETAINED_BYTES / CREDENTIAL_BYTES {
        let password = format!("{index:02}{filler}");
        needs
            .record(&at("h", &format!("/r{index}")), secret("u", &password))
            .unwrap();
    }
    assert_eq!(needs.counts().retained, RETAINED_BYTES);
    assert_eq!(
        needs.record(&over, secret("v", "")),
        Err(ErrorCode::Capacity)
    );
    assert!(is(&needs, &over, |r| matches!(r, Recorded::Unretained)));
    assert_eq!(
        answered(&needs, &at("h", "/r0")),
        Some(header("u", &format!("00{filler}")))
    );
}

/// The wipes are what `Secret`'s own `Drop` left in each buffer, capacity included (the username's terminator slot is
/// its fifth byte), reported by that `Drop` after its overwrite: the probe only listens, so a `Drop` that stops wiping
/// turns this row red.
#[test]
fn h1a_4_a_rejection_wipes_the_credential_of_every_key_holding_it_at_once() {
    let needs = record();
    let wipes = needs.wipes();
    let keys: Vec<_> = (1..=16)
        .map(|index| at("h", &format!("/k{index}")))
        .collect();
    for key in &keys {
        needs.record(key, secret("user", "pass")).unwrap();
    }
    let unrelated = at("h", "/unrelated");
    needs.record(&unrelated, secret("user", "other")).unwrap();
    assert_eq!(needs.counts().credentials, 2);
    assert_eq!(needs.reject(&keys[0], None), ErrorCode::Authentication);
    assert_eq!(*wipes.lock().unwrap(), vec![(4, 5, true), (4, 4, true)]);
    for key in &keys {
        assert!(is(&needs, key, rejected));
    }
    assert_eq!(answered(&needs, &unrelated), Some(header("user", "other")));
    let counts = needs.counts();
    assert_eq!(
        (counts.credentials, counts.retained, counts.fingerprints),
        (1, 9, 1)
    );
}

#[test]
fn h1a_5_a_rejection_at_one_origin_leaves_another_origins_equal_credential_usable() {
    let needs = record();
    let (a, b) = (at("a", "/repo"), at("b", "/repo"));
    needs.record(&a, secret("user", "pass")).unwrap();
    needs.record(&b, secret("user", "pass")).unwrap();
    assert_eq!(needs.reject(&a, None), ErrorCode::Authentication);
    assert!(is(&needs, &a, rejected));
    assert_eq!(answered(&needs, &b), Some(header("user", "pass")));
    assert_eq!(needs.vet(&at("b", "/new"), &secret("user", "pass")), Ok(()));
}

#[test]
fn h1a_6_a_rejected_fingerprint_refuses_equal_bytes_at_its_origin_only_and_holds_no_secret() {
    let needs = record();
    let password = "correct-horse-battery-staple";
    let first = at("a", "/one");
    assert_eq!(
        needs.reject(&first, Some(&secret("user", password))),
        ErrorCode::Authentication
    );
    let second = at("a", "/two");
    assert_eq!(
        needs.vet(&second, &secret("user", password)),
        Err(ErrorCode::Authentication)
    );
    assert!(is(&needs, &second, rejected));
    let third = at("a", "/three");
    assert_eq!(
        needs.record(&third, secret("user", password)),
        Err(ErrorCode::Authentication)
    );
    assert!(is(&needs, &third, rejected));
    assert_eq!(
        needs.vet(&at("a", "/four"), &secret("user", "another")),
        Ok(())
    );
    assert_eq!(
        needs.vet(&at("b", "/one"), &secret("user", password)),
        Ok(())
    );
    let counts = needs.counts();
    assert_eq!(
        (counts.credentials, counts.retained, counts.fingerprints),
        (0, 0, 1)
    );
    assert!(!needs.contains(password.as_bytes()));
    assert!(!needs.contains(b"user"));
}

#[test]
fn h1a_7_one_fingerprint_past_the_bound_or_a_poisoned_lock_exhausts_the_record() {
    let needs = record();
    let wipes = needs.wipes();
    let (basic, bound) = (at("h", "/basic"), at("h", "/bound"));
    needs.record(&basic, secret("user", "kept-secret")).unwrap();
    needs.bound(&bound, 10, Evidence::Challenged).unwrap();
    for index in 0..FINGERPRINTS {
        let key = at("r", &format!("/r{index}"));
        let fresh = secret("user", &format!("r{index}"));
        assert_eq!(needs.reject(&key, Some(&fresh)), ErrorCode::Authentication);
    }
    assert_eq!(needs.counts().fingerprints, FINGERPRINTS);
    assert!(!needs.exhausted());
    let last = at("r", "/last");
    assert_eq!(
        needs.reject(&last, Some(&secret("user", "last"))),
        ErrorCode::Authentication
    );
    assert!(needs.exhausted());
    for key in [&basic, &bound] {
        assert!(is(&needs, key, |r| matches!(r, Recorded::Tombstone)));
    }
    assert!(is(&needs, &at("r", "/r0"), rejected));
    assert!(is(&needs, &last, rejected));
    assert_eq!(
        (needs.counts().credentials, needs.counts().retained),
        (0, 0)
    );
    assert!(!needs.contains(b"kept-secret"));
    assert!(wipes.lock().unwrap().iter().all(|(_, _, zero)| *zero));
    assert_eq!(wipes.lock().unwrap().len(), 2);
    let fresh = at("h", "/fresh");
    assert_eq!(
        needs.record(&fresh, secret("user", "new")),
        Err(ErrorCode::Capacity)
    );
    assert_eq!(
        needs.bound(&fresh, 1, Evidence::Challenged),
        Err(ErrorCode::Capacity)
    );
    assert_eq!(
        needs.vet(&fresh, &secret("user", "new")),
        Err(ErrorCode::Capacity)
    );
    assert!(is(&needs, &fresh, |r| matches!(r, Recorded::Unrecorded)));
    // A poisoned lock: the call that finds it gets the budget code, after wiping and tombstoning.
    let needs = record();
    let key = at("h", "/repo");
    needs
        .record(&key, secret("user", "poisoned-secret"))
        .unwrap();
    needs.poison();
    assert!(matches!(needs.answer(&key), Err(ErrorCode::Capacity)));
    assert!(needs.exhausted());
    assert!(is(&needs, &key, |r| matches!(r, Recorded::Tombstone)));
    assert!(!needs.contains(b"poisoned-secret"));
    // No randomness at creation: `Exhausted` from the start.
    assert!(Needs::keyed(None).exhausted());
}

#[test]
fn h1a_8_the_declared_charge_covers_the_record_with_every_bound_full() {
    let needs = record();
    for index in 0..FINGERPRINTS {
        let key = at("r", &format!("/r{index}"));
        needs.reject(&key, Some(&secret("user", &format!("r{index}"))));
    }
    let length = RETAINED_BYTES / CREDENTIALS;
    for index in 0..CREDENTIALS {
        let password = format!("{index:02}{}", "x".repeat(length - 3));
        needs
            .record(&at("h", &format!("/c{index}")), secret("u", &password))
            .unwrap();
    }
    for index in 0..KEYS - FINGERPRINTS - CREDENTIALS {
        needs
            .bound(&at("b", &format!("/b{index}")), 1, Evidence::OnBound)
            .unwrap();
    }
    let counts = needs.counts();
    assert_eq!(
        (
            counts.keys,
            counts.credentials,
            counts.retained,
            counts.fingerprints,
            counts.exhausted
        ),
        (KEYS, CREDENTIALS, RETAINED_BYTES, FINGERPRINTS, false)
    );
    assert!(
        needs.byte_sum() <= CHARGE,
        "{} > {CHARGE}",
        needs.byte_sum()
    );
    let context = OperationAuth::context(
        Config {
            executable: "git".into(),
            environment: Vec::new(),
        },
        None,
    );
    assert!(context.charge() >= CHARGE);
    assert!(!OperationAuth::of(&context).unwrap().needs().exhausted());
}

#[test]
fn h1a_9_no_format_path_shows_credential_bytes() {
    let needs = record();
    let (username, password) = ("alice-account", "hunter2-password");
    let key = at("h", "/repo");
    needs.record(&key, secret(username, password)).unwrap();
    needs
        .bound(&at("h", "/bound"), 3, Evidence::Challenged)
        .unwrap();
    needs.reject(
        &at("h", "/gone"),
        Some(&secret(username, "rejected-password")),
    );
    let unretained = needs.record(
        &at("h", "/large"),
        secret("u", &"z".repeat(CREDENTIAL_BYTES)),
    );
    let refused = needs.vet(&at("h", "/other"), &secret(username, "rejected-password"));
    let encoded = secret(username, password).header();
    let encoded = String::from_utf8(encoded.as_bytes()[6..].to_vec()).unwrap();
    let texts = [
        format!("{needs:?}"),
        format!("{:?}", needs.answer(&key)),
        format!("{:?}", needs.answer(&at("h", "/bound"))),
        format!("{key:?}"),
        format!("{:?}", needs.counts()),
        format!("{unretained:?}"),
        format!("{refused:?}"),
        format!("{:?}", secret(username, password)),
    ];
    for text in &texts {
        for leaked in [username, password, "rejected-password", encoded.as_str()] {
            assert!(
                !text.contains(leaked),
                "a format path shows credential bytes"
            );
        }
    }
    assert_eq!(
        texts[0],
        "Needs { keys: 5, credentials: 1, fingerprints: 1, exhausted: false }"
    );
    assert_eq!(texts[1], "Ok(Basic(SecretHeader(REDACTED)))");
}

#[test]
fn h1a_10_kinds_flip_and_a_rejection_is_terminal() {
    let needs = record();
    let wipes = needs.wipes();
    let key = at("h", "/flip");
    needs.record(&key, secret("user", "pass")).unwrap();
    needs.bound(&key, 5, Evidence::OnBound).unwrap();
    assert_eq!(
        answered(&needs, &key),
        Some(header("user", "pass")),
        "OnBound replaces nothing"
    );
    needs.bound(&key, 5, Evidence::Challenged).unwrap();
    assert!(is(&needs, &key, |r| matches!(
        r,
        Recorded::ConnectionBound {
            observed: 5,
            evidence: Evidence::Challenged
        }
    )));
    assert_eq!(needs.counts().credentials, 0);
    assert_eq!(*wipes.lock().unwrap(), vec![(4, 5, true), (4, 4, true)]);
    needs.record(&key, secret("user", "pass")).unwrap();
    assert_eq!(
        answered(&needs, &key),
        Some(header("user", "pass")),
        "a kind change replaces"
    );
    // Evidence upgrades; an equal later insert keeps the first.
    let bound = at("h", "/bound");
    needs.bound(&bound, 3, Evidence::OnBound).unwrap();
    needs.bound(&bound, 4, Evidence::Challenged).unwrap();
    needs.bound(&bound, 9, Evidence::Challenged).unwrap();
    assert!(is(&needs, &bound, |r| matches!(
        r,
        Recorded::ConnectionBound {
            observed: 4,
            evidence: Evidence::Challenged
        }
    )));
    let first = at("h", "/first");
    needs.record(&first, secret("user", "one")).unwrap();
    needs.record(&first, secret("user", "two")).unwrap();
    assert_eq!(answered(&needs, &first), Some(header("user", "one")));
    // `Rejected` is terminal: a late insert of either kind changes nothing.
    assert_eq!(needs.reject(&bound, None), ErrorCode::Authentication);
    needs.record(&bound, secret("user", "late")).unwrap();
    needs.bound(&bound, 1, Evidence::Challenged).unwrap();
    needs.bound(&bound, 1, Evidence::OnBound).unwrap();
    assert!(is(&needs, &bound, rejected));
    assert_eq!(
        needs.vet(&bound, &secret("user", "late")),
        Err(ErrorCode::Authentication)
    );
}
