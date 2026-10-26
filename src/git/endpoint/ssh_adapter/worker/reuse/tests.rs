//! The pure parts of the adapter's reuse proofs. The proof jobs themselves run against the private agent in
//! `transport_host/connection_host/tests.rs`.
use super::{proof_deadline, trust_names};

#[test]
fn a_proof_never_outlasts_its_opening() {
    // Code P2-1 = State P2-1, adapter row: the job's deadline is at most the opening's, whatever the connect bound.
    for (connect, remaining) in [
        (0, 5_000),
        (20_000, 5_000),
        (3_000, 5_000),
        (20_000, 0),
        (1, 1),
    ] {
        let at = proof_deadline(1_000, connect, remaining);
        assert!(at <= 1_000 + remaining, "{connect} {remaining}: {at}");
    }
    // The adapter's own bound still applies when it is the earlier.
    assert_eq!(proof_deadline(1_000, 3_000, 5_000), 4_000);
    assert_eq!(proof_deadline(1_000, 20_000, 5_000), 6_000);
    // No connect bound: only the opening's.
    assert_eq!(proof_deadline(1_000, 0, 5_000), 6_000);
    // An opening with nothing left gives a proof that has already expired.
    assert_eq!(proof_deadline(1_000, 20_000, 0), 1_000);
    assert_eq!(proof_deadline(u64::MAX - 1, 20_000, 5_000), u64::MAX);
}

#[test]
fn trust_is_checked_under_the_host_as_written_when_it_spells_it_differently() {
    assert_eq!(
        trust_names("githost.example", "githost.example"),
        ["githost.example"]
    );
    assert_eq!(
        trust_names("githost.example", "GitHost.Example"),
        ["githost.example", "GitHost.Example"]
    );
    // A pool key that was not lowercased is lowercased; the written spelling is added only for the same host.
    assert_eq!(
        trust_names("GitHost.Example", "GitHost.Example"),
        ["githost.example", "GitHost.Example"]
    );
    assert_eq!(
        trust_names("githost.example", "other.example"),
        ["githost.example"]
    );
}
