//! Lane X's Tier A suite: every case runs against the recording fake, so
//! each assertion is about what this crate *did*, not about Git.
//!
//! The scenario is one family: the receiving workspace `/D` and the source
//! workspace `/A`, each with a root repository and three members
//! (`mem_app`, `mem_lib`, `mem_doc`) at the same recorded paths.

use std::cell::Cell;

use gwz_repo_contract::{ObjectFormat, ObjectId, RepoKey};

use super::*;
use crate::test_support::{RecordingTransport, TransportCall};

const IMPORT: &str = "refs/gwz/local-imports/t1";

fn oid(byte: u8) -> ObjectId {
    ObjectId::from_bytes(ObjectFormat::Sha1, &[byte; 20]).unwrap()
}

fn member(id: &str) -> RepoKey {
    RepoKey::Member { id: id.to_owned() }
}

fn members() -> Vec<RepoKey> {
    vec![member("mem_app"), member("mem_lib"), member("mem_doc")]
}

fn receivers() -> Vec<Participant> {
    vec![
        Participant::root("/D"),
        Participant::new(member("mem_app"), "app", "/D/app"),
        Participant::new(member("mem_lib"), "lib", "/D/lib"),
        Participant::new(member("mem_doc"), "doc", "/D/doc"),
    ]
}

fn sources() -> Vec<Participant> {
    vec![
        Participant::root("/A"),
        Participant::new(member("mem_app"), "app", "/A/app"),
        Participant::new(member("mem_lib"), "lib", "/A/lib"),
        Participant::new(member("mem_doc"), "doc", "/A/doc"),
    ]
}

fn request(selected: Vec<RepoKey>) -> ImportRequest {
    ImportRequest {
        transfer: TransferId::new("t1").unwrap(),
        receivers: receivers(),
        sources: sources(),
        selected,
        selector: SourceSelector::Head,
    }
}

/// Every source resolves: app 1, lib 2, root 3, doc 4.
fn transport() -> RecordingTransport {
    let mut transport = RecordingTransport::new();
    for (path, byte) in [("/A/app", 1), ("/A/lib", 2), ("/A", 3), ("/A/doc", 4)] {
        transport.source(path, &SourceSelector::Head, oid(byte));
    }
    transport
}

fn resolve(source: &str) -> TransportCall {
    TransportCall::ResolveSource {
        source: source.into(),
        selector: SourceSelector::Head,
    }
}

fn ref_exists(repository: &str, name: &str) -> TransportCall {
    TransportCall::RefExists {
        repository: repository.into(),
        name: name.to_owned(),
    }
}

fn fetch(receiver: &str, source: &str) -> TransportCall {
    TransportCall::FetchAnonymous {
        receiver: receiver.into(),
        source: source.into(),
        refspecs: vec![format!("HEAD:{IMPORT}")],
    }
}

fn read_ref(repository: &str, name: &str) -> TransportCall {
    TransportCall::ReadRef {
        repository: repository.into(),
        name: name.to_owned(),
    }
}

/// Cancelled from the `limit`-th check onwards.
struct CancelAt {
    checks: Cell<usize>,
    limit: usize,
}

impl CancelAt {
    fn new(limit: usize) -> Self {
        Self {
            checks: Cell::new(0),
            limit,
        }
    }
}

impl Cancellation for CancelAt {
    fn is_cancelled(&self) -> bool {
        let seen = self.checks.get();
        self.checks.set(seen + 1);
        seen >= self.limit
    }
}

#[test]
fn import_ref_names_live_in_the_retained_namespace() {
    let id = TransferId::new("01J").unwrap();
    assert_eq!(id.import_ref(), "refs/gwz/local-imports/01J");
    assert!(TransferId::new("").is_err());
    assert!(TransferId::new("a/b").is_err());
    assert!(!IMPORT_REF_NAMESPACE.starts_with("refs/gwz/merge"));
}

// --- pairing: decided before the transport is touched at all -------------

/// Design §6: pairing is by member id, and a set mismatch — an id one
/// workspace has and the other does not, in either direction — refuses the
/// whole operation, aggregating both directions, with no fetch.
#[test]
fn a_member_set_mismatch_refuses_aggregating_before_any_transport_call() {
    let mut transport = transport();
    let mut request = request(vec![member("mem_app")]);
    request.sources.retain(|participant| {
        participant.key != member("mem_lib") && participant.key != member("mem_doc")
    });
    request
        .sources
        .push(Participant::new(member("mem_extra"), "extra", "/A/extra"));

    let error = prepare_import(&request, &mut transport, &NeverCancelled).unwrap_err();
    let ImportError::PairingIncomplete { missing, moved } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(
        missing,
        &vec![member("mem_doc"), member("mem_extra"), member("mem_lib")],
        "both directions are reported at once, not just the selected one"
    );
    assert!(moved.is_empty());
    assert!(error.effects().is_empty());
    assert!(
        transport.calls().is_empty(),
        "a pairing refusal happens before the first transport call"
    );
}

/// Path is not the key, but the same id at two different recorded paths is
/// a mismatch: the two workspaces no longer describe the same member.
/// Different spellings of the same path are not.
#[test]
fn the_same_id_recorded_at_a_different_path_refuses_before_any_transport_call() {
    let mut transport = transport();
    let mut request = request(members());
    request.sources[1] = Participant::new(member("mem_app"), "vendor/app", "/A/vendor/app");

    let error = prepare_import(&request, &mut transport, &NeverCancelled).unwrap_err();
    let ImportError::PairingIncomplete { missing, moved } = &error else {
        panic!("{error:?}");
    };
    assert!(missing.is_empty());
    assert_eq!(
        moved,
        &vec![MovedMember {
            key: member("mem_app"),
            receiver_path: "app".to_owned(),
            source_path: "vendor/app".to_owned(),
        }]
    );
    assert!(transport.calls().is_empty());

    let mut respelled = request.clone();
    respelled.sources[1] = Participant::new(member("mem_app"), "./app/", "/A/app");
    assert!(
        pair_participants(&respelled).is_ok(),
        "`./app/` and `app` are the same recorded path"
    );
}

/// A selected `@root` has no member-lock entry: it pairs directly with the
/// other workspace's root, and takes no part in the member-set comparison.
#[test]
fn the_root_pairs_separately_with_the_other_workspace_root() {
    let paired = pair_participants(&request(vec![RepoKey::Root, member("mem_app")])).unwrap();
    assert_eq!(
        paired,
        vec![
            Pairing {
                key: RepoKey::Root,
                receiver: "/D".into(),
                source: "/A".into(),
            },
            Pairing {
                key: member("mem_app"),
                receiver: "/D/app".into(),
                source: "/A/app".into(),
            },
        ]
    );

    // The source workspace has no root entry: unselected root is not a set
    // mismatch, selected root is.
    let mut rootless = request(vec![member("mem_app")]);
    rootless.sources.retain(|p| p.key != RepoKey::Root);
    assert!(pair_participants(&rootless).is_ok());
    rootless.selected.push(RepoKey::Root);
    assert_eq!(
        pair_participants(&rootless).unwrap_err(),
        ImportError::PairingIncomplete {
            missing: vec![RepoKey::Root],
            moved: Vec::new(),
        }
    );
}

#[test]
fn an_empty_or_impossible_selection_is_an_invalid_request() {
    let mut transport = transport();
    for (selected, receivers, sources) in [
        (Vec::new(), receivers(), sources()),
        (vec![member("mem_nope")], receivers(), sources()),
        (
            vec![member("mem_app"), member("mem_app")],
            receivers(),
            sources(),
        ),
        (
            vec![member("mem_app")],
            {
                let mut duplicated = receivers();
                duplicated.push(Participant::new(member("mem_app"), "app", "/D/app2"));
                duplicated
            },
            sources(),
        ),
        (vec![member("mem_app")], receivers(), {
            let mut duplicated = sources();
            duplicated.push(Participant::new(member("mem_app"), "app", "/A/app2"));
            duplicated
        }),
    ] {
        let request = ImportRequest {
            selected,
            receivers,
            sources,
            ..request(vec![member("mem_app")])
        };
        let error = prepare_import(&request, &mut transport, &NeverCancelled).unwrap_err();
        assert!(
            matches!(error, ImportError::InvalidRequest { .. }),
            "{error:?}"
        );
        assert!(error.effects().is_empty());
    }
    assert!(transport.calls().is_empty());
}

// --- capture, collision check, fetch, verification -----------------------

/// The whole happy path in call order: resolve every source, check the
/// import name in every receiver, fetch each with one explicit refspec,
/// then read every received id back. Each receiver holds its own captured
/// object under the one common name.
#[test]
fn an_import_captures_fetches_and_verifies_the_exact_source_oids() {
    let mut transport = transport();
    let imported = prepare_import(
        &request(vec![member("mem_app"), member("mem_lib")]),
        &mut transport,
        &NeverCancelled,
    )
    .unwrap();

    assert_eq!(imported.import_ref, IMPORT);
    assert_eq!(
        imported.vector,
        vec![
            ImportedCommit {
                key: member("mem_app"),
                oid: oid(1),
            },
            ImportedCommit {
                key: member("mem_lib"),
                oid: oid(2),
            },
        ]
    );
    assert_eq!(
        transport.calls(),
        [
            resolve("/A/app"),
            resolve("/A/lib"),
            ref_exists("/D/app", IMPORT),
            ref_exists("/D/lib", IMPORT),
            fetch("/D/app", "/A/app"),
            fetch("/D/lib", "/A/lib"),
            read_ref("/D/app", IMPORT),
            read_ref("/D/lib", IMPORT),
        ]
    );
    // One name, two different captured objects behind it.
    assert_eq!(transport.ref_at("/D/app".as_ref(), IMPORT), Some(&oid(1)));
    assert_eq!(transport.ref_at("/D/lib".as_ref(), IMPORT), Some(&oid(2)));
    assert_eq!(imported.oid_for(&member("mem_lib")), Some(&oid(2)));
}

/// `merge --remote A <ref>` resolves `<ref>` in the source and fetches that
/// same name; the import ref is still the common one. The name reaches the
/// port unchanged.
#[test]
fn a_named_source_ref_is_resolved_in_the_source_and_fetched_by_that_name() {
    let selector = SourceSelector::Ref("refs/heads/lane/agent-17".to_owned());
    let mut transport = RecordingTransport::new();
    transport.source("/A/app", &selector, oid(7));
    let imported = prepare_import(
        &ImportRequest {
            selector: selector.clone(),
            ..request(vec![member("mem_app")])
        },
        &mut transport,
        &NeverCancelled,
    )
    .unwrap();

    assert_eq!(imported.oid_for(&member("mem_app")), Some(&oid(7)));
    assert_eq!(
        transport.calls()[0],
        TransportCall::ResolveSource {
            source: "/A/app".into(),
            selector,
        }
    );
    assert_eq!(
        transport.calls()[2],
        TransportCall::FetchAnonymous {
            receiver: "/D/app".into(),
            source: "/A/app".into(),
            refspecs: vec![format!("refs/heads/lane/agent-17:{IMPORT}")],
        },
        "no `+`: the name was checked free, so a forced update would only \
         hide a collision that appeared since"
    );
}

/// Design §6.1: a branch or ref that does not resolve in the source refuses
/// before any transfer, and reports every participant that lacks it.
#[test]
fn a_source_ref_that_does_not_resolve_refuses_aggregating_before_any_transfer() {
    let mut transport = RecordingTransport::new();
    transport.source("/A/app", &SourceSelector::Head, oid(1));
    let error = prepare_import(
        &request(vec![
            member("mem_app"),
            member("mem_lib"),
            member("mem_doc"),
        ]),
        &mut transport,
        &NeverCancelled,
    )
    .unwrap_err();

    let ImportError::SourceMissing { missing } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(
        missing.iter().map(|p| p.key.clone()).collect::<Vec<_>>(),
        vec![member("mem_lib"), member("mem_doc")],
        "every unresolvable source is named, not just the first"
    );
    assert!(error.effects().is_empty());
    assert!(
        transport
            .calls()
            .iter()
            .all(|call| matches!(call, TransportCall::ResolveSource { .. })),
        "capture only: nothing was checked or fetched"
    );
}

/// The import name must be free in *every* receiver before any receiver is
/// fetched into — a collision found halfway would leave a half-imported
/// family behind a name that already means something else.
#[test]
fn a_collision_in_one_receiver_of_several_refuses_before_any_fetch() {
    let mut transport = transport();
    transport.set_ref("/D/lib", IMPORT, oid(9));

    let error = prepare_import(
        &request(vec![member("mem_app"), member("mem_lib")]),
        &mut transport,
        &NeverCancelled,
    )
    .unwrap_err();
    assert_eq!(
        error,
        ImportError::RefCollision {
            key: member("mem_lib"),
            import_ref: IMPORT.to_owned(),
        }
    );
    assert!(error.effects().is_empty());
    assert_eq!(
        transport.calls(),
        [
            resolve("/A/app"),
            resolve("/A/lib"),
            ref_exists("/D/app", IMPORT),
            ref_exists("/D/lib", IMPORT),
        ],
        "the first receiver was checked and never fetched into"
    );
    assert_eq!(
        transport.ref_at("/D/lib".as_ref(), IMPORT),
        Some(&oid(9)),
        "the colliding ref is left exactly as it was"
    );
}

/// A partial fetch keeps the refs it managed to create, names them, and
/// never reaches the participants behind it.
#[test]
fn a_partial_fetch_keeps_the_refs_it_created_and_refuses() {
    let mut transport = transport();
    // The second receiver's fetch lands its ref and still fails.
    transport.fail_call_after_write(
        "fetch_anonymous",
        2,
        TransportError::Failed {
            detail: "connection lost".to_owned(),
        },
    );
    let error = prepare_import(&request(members()), &mut transport, &NeverCancelled).unwrap_err();

    let ImportError::TransferFailed {
        key,
        detail,
        effects,
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(key, &member("mem_lib"));
    assert!(detail.contains("connection lost"));
    assert_eq!(
        effects,
        &vec![
            ImportEffect::RefCreated {
                key: member("mem_app"),
                import_ref: IMPORT.to_owned(),
                oid: oid(1),
            },
            // The failed fetch did create this one; it is read back, named,
            // and left alone.
            ImportEffect::RefCreated {
                key: member("mem_lib"),
                import_ref: IMPORT.to_owned(),
                oid: oid(2),
            },
        ]
    );
    assert_eq!(transport.ref_at("/D/app".as_ref(), IMPORT), Some(&oid(1)));
    assert_eq!(transport.ref_at("/D/lib".as_ref(), IMPORT), Some(&oid(2)));
    assert_eq!(
        transport.ref_at("/D/doc".as_ref(), IMPORT),
        None,
        "no participant behind the failure was fetched"
    );
    assert!(!transport.calls().iter().any(|call| matches!(
        call,
        TransportCall::FetchAnonymous { receiver, .. }
            if receiver == std::path::Path::new("/D/doc")
    )));
}

/// A fetch that fails without creating anything reports only the refs the
/// earlier participants left.
#[test]
fn a_fetch_that_creates_nothing_reports_only_the_earlier_refs() {
    let mut transport = transport();
    transport.fail_call(
        "fetch_anonymous",
        2,
        TransportError::Repository {
            path: "/D/lib".into(),
            detail: "receiver is locked".to_owned(),
        },
    );
    let error = prepare_import(&request(members()), &mut transport, &NeverCancelled).unwrap_err();

    let ImportError::TransferFailed { key, effects, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(key, &member("mem_lib"));
    assert_eq!(
        effects,
        &vec![ImportEffect::RefCreated {
            key: member("mem_app"),
            import_ref: IMPORT.to_owned(),
            oid: oid(1),
        }],
        "only the ref this import actually created"
    );
    assert_eq!(transport.ref_at("/D/lib".as_ref(), IMPORT), None);
    assert_eq!(transport.ref_at("/D/doc".as_ref(), IMPORT), None);
}

/// The verification exists for exactly this: the source advanced between
/// capture and fetch, so the receiver holds an object the caller never
/// captured. No engine may see that.
#[test]
fn a_source_that_advances_between_capture_and_fetch_fails_verification() {
    let mut transport = transport();
    transport.drift_after_resolve("/A/lib", &SourceSelector::Head, oid(42));

    let error = prepare_import(
        &request(vec![member("mem_app"), member("mem_lib")]),
        &mut transport,
        &NeverCancelled,
    )
    .unwrap_err();
    let ImportError::VectorMismatch {
        key,
        expected,
        received,
        effects,
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(
        (key, expected, received),
        (&member("mem_lib"), &oid(2), &Some(oid(42)))
    );
    assert_eq!(
        effects,
        &vec![
            ImportEffect::RefCreated {
                key: member("mem_app"),
                import_ref: IMPORT.to_owned(),
                oid: oid(1),
            },
            ImportEffect::RefCreated {
                key: member("mem_lib"),
                import_ref: IMPORT.to_owned(),
                // Corrected from the captured id: the ref is real and holds
                // what the receiver actually took.
                oid: oid(42),
            },
        ]
    );
}

/// A transfer that reports success and delivers nothing is caught by the
/// same read-back, and leaves no effect to report for that receiver.
#[test]
fn a_transfer_that_delivers_nothing_fails_verification() {
    let mut transport = transport();
    transport.succeed_without_effect("fetch_anonymous");

    let error = prepare_import(
        &request(vec![member("mem_app")]),
        &mut transport,
        &NeverCancelled,
    )
    .unwrap_err();
    assert_eq!(
        error,
        ImportError::VectorMismatch {
            key: member("mem_app"),
            expected: oid(1),
            received: None,
            effects: Vec::new(),
        }
    );
}

/// A retry mints a fresh transfer id, so it never collides with, reuses or
/// disturbs what the failed attempt left behind.
#[test]
fn a_retry_uses_a_fresh_transfer_id_and_leaves_the_earlier_import_alone() {
    let mut transport = transport();
    transport.fail_next_after_write(
        "fetch_anonymous",
        TransportError::Failed {
            detail: "interrupted".to_owned(),
        },
    );
    let selected = vec![member("mem_app"), member("mem_lib")];
    prepare_import(&request(selected.clone()), &mut transport, &NeverCancelled).unwrap_err();
    assert_eq!(transport.ref_at("/D/app".as_ref(), IMPORT), Some(&oid(1)));

    let retry = ImportRequest {
        transfer: TransferId::new("t2").unwrap(),
        ..request(selected)
    };
    let imported = prepare_import(&retry, &mut transport, &NeverCancelled).unwrap();
    assert_eq!(imported.import_ref, "refs/gwz/local-imports/t2");
    assert_eq!(
        transport.ref_at("/D/app".as_ref(), IMPORT),
        Some(&oid(1)),
        "the abandoned import is retained, not reused and not pruned"
    );
    assert_eq!(
        transport.ref_at("/D/app".as_ref(), "refs/gwz/local-imports/t2"),
        Some(&oid(1))
    );
    assert_eq!(
        transport.ref_at("/D/lib".as_ref(), "refs/gwz/local-imports/t2"),
        Some(&oid(2))
    );
}

#[test]
fn cancellation_reports_the_refs_it_had_created_and_stops_there() {
    let mut transport = transport();
    // Checks: one before capture, then one before each fetch.
    let error = prepare_import(
        &request(vec![member("mem_app"), member("mem_lib")]),
        &mut transport,
        &CancelAt::new(2),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ImportError::Cancelled {
            effects: vec![ImportEffect::RefCreated {
                key: member("mem_app"),
                import_ref: IMPORT.to_owned(),
                oid: oid(1),
            }],
        }
    );
    assert_eq!(transport.ref_at("/D/lib".as_ref(), IMPORT), None);

    let mut untouched = super::tests::transport();
    let error = prepare_import(
        &request(vec![member("mem_app")]),
        &mut untouched,
        &CancelAt::new(0),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ImportError::Cancelled {
            effects: Vec::new()
        }
    );
    assert!(untouched.calls().is_empty());
}

/// Design §6.2: no automatic pruning in v0, on any path. The port has no
/// removal method at all — the exhaustive match below is what makes that
/// structural, so adding one fails to compile here — and every ref that
/// existed before a success, a refusal or a cancellation still exists after.
#[test]
fn nothing_in_this_crate_prunes_a_ref() {
    fn removes_a_ref(call: &TransportCall) -> bool {
        match call {
            TransportCall::ResolveSource { .. }
            | TransportCall::RefExists { .. }
            | TransportCall::FetchAnonymous { .. }
            | TransportCall::PushAnonymous { .. }
            | TransportCall::ReadRef { .. } => false,
        }
    }

    let mut transport = transport();
    transport.set_ref("/D/app", "refs/heads/main", oid(5));
    transport.set_ref("/D/lib", "refs/heads/main", oid(6));

    let selected = vec![member("mem_app"), member("mem_lib")];
    prepare_import(&request(selected.clone()), &mut transport, &NeverCancelled).unwrap();
    // A collision, a cancellation and a verification failure, in turn.
    for (id, arrange) in [
        ("t1", None::<fn(&mut RecordingTransport)>),
        (
            "t3",
            Some(
                (|transport: &mut RecordingTransport| {
                    transport.succeed_without_effect("fetch_anonymous");
                }) as fn(&mut RecordingTransport),
            ),
        ),
    ] {
        if let Some(arrange) = arrange {
            arrange(&mut transport);
        }
        let attempt = ImportRequest {
            transfer: TransferId::new(id).unwrap(),
            ..request(selected.clone())
        };
        prepare_import(&attempt, &mut transport, &NeverCancelled).unwrap_err();
    }
    prepare_import(&request(selected), &mut transport, &CancelAt::new(1)).unwrap_err();

    for (repository, name, expected) in [
        ("/D/app", "refs/heads/main", oid(5)),
        ("/D/lib", "refs/heads/main", oid(6)),
        ("/D/app", IMPORT, oid(1)),
        ("/D/lib", IMPORT, oid(2)),
    ] {
        assert_eq!(
            transport.ref_at(repository.as_ref(), name),
            Some(&expected),
            "{repository} {name} survives every path"
        );
    }
    assert!(!transport.calls().iter().any(removes_a_ref));
}

// --- push ----------------------------------------------------------------

fn push_transport() -> RecordingTransport {
    let mut transport = RecordingTransport::new();
    transport.set_ref("/D/app", "refs/heads/main", oid(1));
    transport.set_ref("/D/lib", "refs/heads/main", oid(2));
    transport.set_ref("/D", "refs/heads/main", oid(3));
    transport
}

fn push_item(key: RepoKey, source: &str, destination: &str, receiver: ReceiverState) -> PushItem {
    PushItem {
        key,
        source: source.into(),
        destination: destination.into(),
        head: PushHead::Attached {
            branch: "main".to_owned(),
        },
        receiver,
        refspec: None,
    }
}

/// Design §6.1: each selected repository's current attached branch is
/// published to the same branch in the receiver, and no named remote is
/// involved.
#[test]
fn push_publishes_the_attached_branch_into_the_same_branch() {
    let mut transport = push_transport();
    let report = push_local(
        &PushPlan {
            items: vec![push_item(
                member("mem_app"),
                "/D/app",
                "/hub/app",
                ReceiverState::Bare,
            )],
        },
        &mut transport,
    );

    assert!(report.all_pushed());
    assert_eq!(
        report.outcome_for(&member("mem_app")),
        Some(&PushOutcome::Pushed {
            refspec: "refs/heads/main:refs/heads/main".to_owned(),
        })
    );
    assert_eq!(
        transport.calls(),
        [
            ref_exists("/D/app", "refs/heads/main"),
            TransportCall::PushAnonymous {
                source: "/D/app".into(),
                destination: "/hub/app".into(),
                refspec: "refs/heads/main:refs/heads/main".to_owned(),
            },
        ]
    );
    assert_eq!(
        transport.ref_at("/hub/app".as_ref(), "refs/heads/main"),
        Some(&oid(1))
    );
}

/// The checked-out-branch refusal, and the recorded platform limit behind
/// it: libgit2's local transport refuses every push into a non-bare
/// repository, so any checkout receiver is refused (design §11 item 16 is
/// open — this states the limit, it does not settle it). Both are decided
/// from the observed state, with no transport call for that repository.
#[test]
fn push_refuses_a_checked_out_branch_and_every_other_non_bare_receiver() {
    let mut transport = push_transport();
    let report = push_local(
        &PushPlan {
            items: vec![
                push_item(
                    RepoKey::Root,
                    "/D",
                    "/root",
                    ReceiverState::Checkout {
                        checked_out: Some("main".to_owned()),
                    },
                ),
                push_item(
                    member("mem_app"),
                    "/D/app",
                    "/other/app",
                    ReceiverState::Checkout {
                        checked_out: Some("lane/agent-17".to_owned()),
                    },
                ),
                push_item(
                    member("mem_lib"),
                    "/D/lib",
                    "/other/lib",
                    ReceiverState::Checkout { checked_out: None },
                ),
            ],
        },
        &mut transport,
    );

    assert_eq!(
        report.outcome_for(&RepoKey::Root),
        Some(&PushOutcome::Refused {
            refusal: PushRefusal::CheckedOutBranch {
                branch: "main".to_owned(),
            },
        })
    );
    for key in [member("mem_app"), member("mem_lib")] {
        assert_eq!(
            report.outcome_for(&key),
            Some(&PushOutcome::Refused {
                refusal: PushRefusal::NonBareReceiver {
                    target: "refs/heads/main".to_owned(),
                },
            }),
            "{key}"
        );
    }
    assert!(!report.all_pushed());
    assert!(
        transport.calls().is_empty(),
        "every refusal is decided before any transfer"
    );
}

/// A detached source has no branch for the same-branch rule, so it refuses
/// before transfer; an explicit refspec says what to publish and keeps its
/// own mapping.
#[test]
fn push_refuses_a_detached_head_unless_an_explicit_refspec_says_what_to_publish() {
    let mut transport = push_transport();
    let detached = PushItem {
        head: PushHead::Detached,
        ..push_item(member("mem_app"), "/D/app", "/hub/app", ReceiverState::Bare)
    };
    let report = push_local(
        &PushPlan {
            items: vec![detached.clone()],
        },
        &mut transport,
    );
    assert_eq!(
        report.outcome_for(&member("mem_app")),
        Some(&PushOutcome::Refused {
            refusal: PushRefusal::DetachedHead,
        })
    );
    assert!(transport.calls().is_empty());

    let explicit = PushItem {
        refspec: Some("refs/heads/main:refs/heads/lane/from-A".to_owned()),
        ..detached
    };
    let report = push_local(
        &PushPlan {
            items: vec![explicit],
        },
        &mut transport,
    );
    assert_eq!(
        report.outcome_for(&member("mem_app")),
        Some(&PushOutcome::Pushed {
            refspec: "refs/heads/main:refs/heads/lane/from-A".to_owned(),
        }),
        "the explicit mapping is kept, not rewritten to the same-branch rule"
    );
    assert_eq!(
        transport.ref_at("/hub/app".as_ref(), "refs/heads/lane/from-A"),
        Some(&oid(1))
    );
}

#[test]
fn push_refuses_a_missing_source_branch_and_a_malformed_refspec_before_transfer() {
    let mut transport = push_transport();
    let missing = PushItem {
        head: PushHead::Attached {
            branch: "lane/agent-17".to_owned(),
        },
        ..push_item(member("mem_app"), "/D/app", "/hub/app", ReceiverState::Bare)
    };
    let malformed = PushItem {
        refspec: Some("refs/heads/main".to_owned()),
        ..push_item(member("mem_lib"), "/D/lib", "/hub/lib", ReceiverState::Bare)
    };
    let report = push_local(
        &PushPlan {
            items: vec![missing, malformed],
        },
        &mut transport,
    );

    assert_eq!(
        report.outcome_for(&member("mem_app")),
        Some(&PushOutcome::Refused {
            refusal: PushRefusal::MissingSourceRef {
                name: "refs/heads/lane/agent-17".to_owned(),
            },
        })
    );
    assert!(matches!(
        report.outcome_for(&member("mem_lib")),
        Some(PushOutcome::Refused {
            refusal: PushRefusal::InvalidRefspec { .. }
        })
    ));
    assert!(
        !transport
            .calls()
            .iter()
            .any(|call| matches!(call, TransportCall::PushAnonymous { .. }))
    );
}

/// Ordinary per-repository partial results: one published, one refused
/// before transfer, one attempted and rejected by the receiver. The report
/// carries all three and the caller decides.
#[test]
fn push_keeps_per_repository_partial_results() {
    let mut transport = push_transport();
    transport.fail_next(
        "push_anonymous",
        TransportError::Rejected {
            refspec: "refs/heads/main:refs/heads/main".to_owned(),
            detail: "non-fast-forward".to_owned(),
        },
    );
    let report = push_local(
        &PushPlan {
            items: vec![
                push_item(member("mem_lib"), "/D/lib", "/hub/lib", ReceiverState::Bare),
                push_item(
                    RepoKey::Root,
                    "/D",
                    "/root",
                    ReceiverState::Checkout {
                        checked_out: Some("main".to_owned()),
                    },
                ),
                push_item(member("mem_app"), "/D/app", "/hub/app", ReceiverState::Bare),
            ],
        },
        &mut transport,
    );

    assert!(!report.all_pushed());
    assert!(matches!(
        report.outcome_for(&member("mem_lib")),
        Some(PushOutcome::Failed { .. })
    ));
    assert!(matches!(
        report.outcome_for(&RepoKey::Root),
        Some(PushOutcome::Refused { .. })
    ));
    assert!(matches!(
        report.outcome_for(&member("mem_app")),
        Some(PushOutcome::Pushed { .. })
    ));
    assert_eq!(
        transport.ref_at("/hub/app".as_ref(), "refs/heads/main"),
        Some(&oid(1)),
        "a later repository still publishes after an earlier failure"
    );
    assert_eq!(
        transport.ref_at("/hub/lib".as_ref(), "refs/heads/main"),
        None
    );
}
