//! The ten UNBOUND M4 progress shapes, driven through `adapt_open` per shape.
//!
//! `GwzM5-8R4bG-Evidence.md` §12.9(d) leaves **18 UNBOUND rows** — "10 progress
//! plus 8 archive". This file is the progress half: §12.7's cheap closures (i)
//! and (ii), landed together because `GwzM5-8A1ActivationRecord.md` §6 rules that
//! they must be — "the registry rows and the parametric test land as one
//! package", and that the precheck walk test discharges neither, "the precheck
//! reads state+mode and never drives `adapt_open`". Everything below drives
//! `adapt_open` itself, through the real `decode -> adapt -> atomic upgrade`
//! path, exactly once per shape.
//!
//! The dispositions are read off the frozen contract, never invented. Cited
//! content-anchored per the R2-E citing rule
//! (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` Appendix A), because
//! `GwzM5-8I2CompatibilityContract.md` is a live annotatable frozen document
//! whose line numbers move under it; line numbers are a convenience, dated:
//!
//! - **§4 "Open v0 compatibility result"**, "A1 deliberately whitelists only
//!   seven one-member-workspace, `Finalizing`, normal-mode shapes … Every
//!   selected result, baseline byte/hash, candidate/composition relation,
//!   participant HEAD/ref/index/worktree, and root observation is exact.
//!   Marker/lock-only prefixes, multi-member workspaces, selected root, born
//!   root, drift, pending actions, recovery, preservation, rollback, and
//!   terminal rows are not A1 migration rules." (`:136-144` as of 2026-08-28)
//! - **§5 "Migration eligibility and atomic boundary"**, "No v0
//!   `RecoveryRequired`, `Preserving`, or `RollingBack` row is A1
//!   migration-eligible. The adapter never invents a recovery origin or reverse
//!   owner from an unjournaled v0 window." (`:170-172` as of 2026-08-28)
//! - **§5**, "Zero whitelist matches is not an error. Open read-only status
//!   leaves bytes unchanged … Completed and aborted v0 records remain v0 and
//!   use byte-preserving archival." (`:178-184` as of 2026-08-28)
//!
//! **Nine of the ten take a `valid_unlisted_corpus` row.** Their operation
//! states are all non-`Finalizing`, so `classify_open_v0`'s first gate answers
//! `ValidUnlisted` structurally and the corpus's own closure assertion — every
//! whitelist rule is open+finalizing, and the fixture's state is not
//! `Finalizing` — keeps its full force. Three state tokens the corpus had never
//! needed are added for them (`executing`, `awaiting_resolution`, `halted`);
//! that is an extension of the corpus vocabulary, not a weakening of it,
//! because the load-bearing `assert_ne!(state, Finalizing)` is untouched.
//!
//! **`G-VERIFYING` is the tenth and takes no registry row**, on
//! `GwzM5-8R4bG-Evidence.md` §12.9(c)'s own ground: it is a `Finalizing` shape,
//! and the `valid_unlisted_corpus` "cannot express a `Finalizing` shape …
//! widening it to admit these rows would weaken the registry, not extend it".
//! §12.9's own Finalizing ground reaches three of its four rows (`F-BASELINE`,
//! `F-MARKER`, `F-LOCK`); `G-VERIFYING` is a fourth `Finalizing` row it never
//! considered (E5 review [P2-4], 2026-08-28 — the first landing of this doc
//! miscounted it as a fifth);
//! see the disposition recorded at `g_verifying_is_dispositioned_by_clause`
//! below. Its record is therefore made here, by clause, with an executed test —
//! which is §12.7's own second branch, "an O8 acceptance note citing the clause
//! per row".
//!
//! **Shape overlap, stated rather than hidden.** R0 §4's row-A pre-acceptance
//! shapes `A-PRE-PRESERVE`/`A-PRE-ROLLBACK` and rows H/I's already-bound
//! `preserving/stash`/`rollback/participant` are close neighbours on this tree:
//! all four are open, publication-free, reverse-owner states. They are
//! separated here by the evidence each carries — the row-A shapes are asserted
//! to carry *no* preservation evidence and *no* reversed participant, which is
//! precisely what `preserving/stash` and `rollback/participant` retain; the
//! new `I-EVIDENCE-ROLLBACK` arm is separated by its composition commit,
//! asserted present — not by a reversed participant, which its construction
//! does not create (E5 review [P3-5], 2026-08-28). R0 §4 row I's own gap sentence ("named
//! record shapes for every reverse prefix are missing") is why the separation
//! is this thin; recording it is E5.1's job, closing it is not.

use super::*;

/// One unbound progress shape, with the registry binding it takes.
#[derive(Clone, Copy, Debug)]
enum Unbound {
    AExecuting,
    AAwaiting,
    AHalted,
    APrePreserve,
    APreRollback,
    GVerifying,
    HPreservingCandidate,
    HPreservingPrefix,
    IEvidenceRollback,
    KCompletedNopubOpen,
}

/// The ten rows of `GwzM5-8R4bG-Evidence.md` §12.9(d)'s progress half, in
/// `GwzM5-8R4bG-Evidence.md` §12.3 Table A order.
const UNBOUND_PROGRESS_SHAPES: [Unbound; 10] = [
    Unbound::AExecuting,
    Unbound::AAwaiting,
    Unbound::AHalted,
    Unbound::APrePreserve,
    Unbound::APreRollback,
    Unbound::GVerifying,
    Unbound::HPreservingCandidate,
    Unbound::HPreservingPrefix,
    Unbound::IEvidenceRollback,
    Unbound::KCompletedNopubOpen,
];

impl Unbound {
    /// The `GwzM5-8R0Inventory.md` §4 shape id, as `GwzM5-8R4bG-Evidence.md`
    /// §12.3 Table A names it.
    fn shape(self) -> &'static str {
        match self {
            Self::AExecuting => "A-EXECUTING",
            Self::AAwaiting => "A-AWAITING",
            Self::AHalted => "A-HALTED",
            Self::APrePreserve => "A-PRE-PRESERVE",
            Self::APreRollback => "A-PRE-ROLLBACK",
            Self::GVerifying => "G-VERIFYING",
            Self::HPreservingCandidate => "H-PRESERVING-CANDIDATE",
            Self::HPreservingPrefix => "H-PRESERVING-PREFIX",
            Self::IEvidenceRollback => "I-EVIDENCE-ROLLBACK",
            Self::KCompletedNopubOpen => "K-COMPLETED-NOPUB-OPEN",
        }
    }

    /// `(case_id, subcase)` in `valid_unlisted_corpus`, or `None` for the one
    /// `Finalizing` shape the corpus cannot express (§12.9(c)).
    fn corpus_binding(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::AExecuting => Some(("open/executing", "a_executing")),
            Self::AAwaiting => Some(("open/awaiting-resolution", "a_awaiting")),
            Self::AHalted => Some(("open/halted", "a_halted")),
            Self::APrePreserve => Some(("preserving/pre-acceptance", "a_pre_preserve")),
            Self::APreRollback => Some(("rollback/pre-acceptance", "a_pre_rollback")),
            Self::GVerifying => None,
            Self::HPreservingCandidate => Some(("preserving/candidate", "h_preserving_candidate")),
            Self::HPreservingPrefix => Some(("preserving/root-prefix", "h_preserving_prefix")),
            Self::IEvidenceRollback => Some(("rollback/evidence", "i_evidence_rollback")),
            Self::KCompletedNopubOpen => Some((
                "terminal/completed-no-publication",
                "k_completed_nopub_open",
            )),
        }
    }
}

/// Fails the write that records step `VerifyingPublication`, but only after it
/// lands, leaving the durable `G-VERIFYING` shape exactly as a crash between
/// the step write and the terminal write would. Same shape as
/// `characterization_publication_v0`'s store of the same name, which is private
/// to that module.
#[derive(Default)]
struct VerifyingPostWriteStore {
    fired: Cell<bool>,
}

impl MergeStore for VerifyingPostWriteStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        FileMergeStore.write_open(root, record)?;
        if !self.fired.get()
            && record.publication.as_ref().map(|progress| progress.step)
                == Some(PublicationStep::VerifyingPublication)
        {
            self.fired.set(true);
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "injected post-write verifying-publication failure",
            ));
        }
        Ok(())
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        FileMergeStore.archive(root, merge_id)
    }
}

/// One shape's durable fixture: the workspace it lives in, the backend that
/// observes it, and the exact v0 record on disk.
struct Durable {
    temp: TempDir,
    backend: crate::git::Git2Backend,
    record: MergeOperationRecord,
    _guard: Box<dyn std::any::Any>,
}

/// A three-member workspace whose merge stops at `AwaitingResolution`: R0 §4
/// row A's pre-acceptance window, with no publication of any kind.
fn pre_acceptance(label: &str) -> Durable {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_unbound_pre").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution,
        "{label}"
    );
    let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert!(record.publication.is_none(), "{label}");
    Durable {
        temp,
        backend,
        record,
        _guard: Box::new(fixture),
    }
}

/// The same window, re-stated into one of row A's other named pre-acceptance
/// states. The state is the only field that moves.
fn pre_acceptance_in_state(label: &str, state: OperationState) -> Durable {
    let mut durable = pre_acceptance(label);
    durable.record.state = state;
    FileMergeStore
        .write_open(durable.temp.path(), &durable.record)
        .unwrap();
    durable
}

/// `A-HALTED`, which needs one field beyond the state: the v0 record contract
/// requires a halted operation to hold a `Failed` participant, or a
/// `Conflicted` one carrying its error (`model/v1/validate/lifecycle.rs`, the
/// `OperationState::Halted` arm). The conflicted member gets its recorded
/// error, which is exactly the durable form a halt leaves behind. Without it
/// the row is a contradictory record, not the M4 shape, and `adapt_open` would
/// refuse it as unreadable rather than answer the disposition under test.
fn pre_acceptance_halted(label: &str) -> Durable {
    let mut durable = pre_acceptance(label);
    let conflicted = durable
        .record
        .participants
        .values_mut()
        .find(|participant| participant.state == ParticipantState::Conflicted)
        .expect("the mixed pre-acceptance window leaves one conflicted member");
    conflicted.error = Some(crate::workspace_ops::merge::MergeRecordError {
        code: ErrorCode::MergeValidationFailed,
        message: "merge halted on an unresolved conflict".to_owned(),
        detail: None,
    });
    durable.record.state = OperationState::Halted;
    FileMergeStore
        .write_open(durable.temp.path(), &durable.record)
        .unwrap();
    durable
}

/// A one-member workspace whose member has a source commit, driven to the
/// named finalization fault point. The publication-bearing progress rows are
/// all cut from this.
fn one_member_at_fault(label: &str, fault: FinalizationFault) -> Durable {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let fixture = init_one_member_workspace(temp.path(), &backend, label);
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(fault);
    invoke_with_store(&backend, &store, temp.path(), request(false), "op_unbound").unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    Durable {
        temp,
        backend,
        record,
        _guard: Box::new(fixture),
    }
}

fn build(shape: Unbound) -> Durable {
    let label = format!("v0-unbound-{}", shape.shape().to_lowercase());
    match shape {
        // Row A: "operation in the named pre-acceptance state;
        // publication/candidate absent". Only `state` separates the five.
        Unbound::AExecuting => pre_acceptance_in_state(&label, OperationState::Executing),
        Unbound::AAwaiting => pre_acceptance(&label),
        Unbound::AHalted => pre_acceptance_halted(&label),
        Unbound::APrePreserve => pre_acceptance_in_state(&label, OperationState::Preserving),
        Unbound::APreRollback => pre_acceptance_in_state(&label, OperationState::RollingBack),

        // Row G: "step `VerifyingPublication` … operation `Finalizing`", the
        // full candidate published, recorded but not yet terminal.
        Unbound::GVerifying => {
            let temp = TempDir::new(&label);
            let backend = crate::git::Git2Backend::new();
            let fixture = init_one_member_workspace(temp.path(), &backend, &label);
            feature_commit(
                &backend,
                &temp.path().join("remote"),
                "README.md",
                "source\n",
            );
            let store = VerifyingPostWriteStore::default();
            let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
            let mut ids = SequentialIdProvider::new();
            let error = handle_merge_with_dependencies(
                MergeDependencies {
                    backend: &backend,
                    store: &store,
                    clock: &clock,
                    ids: &mut ids,
                    events: &crate::operation::NullSink,
                },
                temp.path(),
                request(false),
                "op_unbound_verifying",
            )
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
            assert!(store.fired.get());
            let record = store.discover_open(temp.path()).unwrap().unwrap();
            Durable {
                temp,
                backend,
                record,
                _guard: Box::new(fixture),
            }
        }

        // Row H, source row C: the candidate is durable and the operation is
        // re-stated into `Preserving` with no preservation evidence recorded
        // yet — the window `preserving/stash` (row H's pre-acceptance source)
        // has already left behind.
        Unbound::HPreservingCandidate => {
            let mut durable =
                one_member_at_fault(&label, FinalizationFault::AfterCandidatePersistence);
            durable.record.state = OperationState::Preserving;
            FileMergeStore
                .write_open(durable.temp.path(), &durable.record)
                .unwrap();
            durable
        }

        // Row H, source row F: the same, with one of the four recorded root
        // publication prefixes retained on the record.
        Unbound::HPreservingPrefix => {
            let mut durable = one_member_at_fault(&label, FinalizationFault::AfterLockPublication);
            durable.record.state = OperationState::Preserving;
            durable
                .record
                .publication
                .as_mut()
                .unwrap()
                .preservation_prefix = Some("lock".to_owned());
            FileMergeStore
                .write_open(durable.temp.path(), &durable.record)
                .unwrap();
            durable
        }

        // Row I: "evidence rollback may be interrupted before
        // `evidence_rolled_back = true`" — the reverse owner exists on the
        // record and the flag has not flipped.
        Unbound::IEvidenceRollback => {
            let mut durable = one_member_at_fault(&label, FinalizationFault::AfterLockPublication);
            durable.record.state = OperationState::RollingBack;
            FileMergeStore
                .write_open(durable.temp.path(), &durable.record)
                .unwrap();
            durable
        }

        // Row K: terminal `Completed` still under the open directory, evidence
        // is source row J — the no-publication close, before archive.
        Unbound::KCompletedNopubOpen => {
            let temp = TempDir::new(&label);
            let backend = crate::git::Git2Backend::new();
            let fixture = init_one_member_workspace(temp.path(), &backend, &label);
            backend
                .branch_create(&temp.path().join("remote"), "feature/source", "HEAD")
                .unwrap();
            let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
            invoke_with_store(
                &backend,
                &store,
                temp.path(),
                request(false),
                "op_unbound_nopub_open",
            )
            .unwrap_err();
            let record = store.discover_open(temp.path()).unwrap().unwrap();
            Durable {
                temp,
                backend,
                record,
                _guard: Box::new(fixture),
            }
        }
    }
}

/// The exact R0 §4 combination each row names, asserted before the shape is
/// driven, so a fixture that silently drifts into a different durable shape
/// fails here rather than passing a refusal it never earned.
fn assert_r0_combination(shape: Unbound, record: &MergeOperationRecord) {
    let name = shape.shape();
    let publication = record.publication.as_ref();
    match shape {
        Unbound::AExecuting
        | Unbound::AAwaiting
        | Unbound::AHalted
        | Unbound::APrePreserve
        | Unbound::APreRollback => {
            assert!(publication.is_none(), "{name} publication must be absent");
            assert!(
                record
                    .participants
                    .values()
                    .all(|participant| participant.preservation.is_empty()),
                "{name} carries preservation evidence, which is row H's"
            );
            assert!(
                record.participants.values().all(|participant| !matches!(
                    participant.state,
                    crate::workspace_ops::merge::ParticipantState::Aborted
                        | crate::workspace_ops::merge::ParticipantState::RolledBack
                )),
                "{name} carries a reversed participant, which is row I's"
            );
            let expected = match shape {
                Unbound::AExecuting => OperationState::Executing,
                Unbound::AAwaiting => OperationState::AwaitingResolution,
                Unbound::AHalted => OperationState::Halted,
                Unbound::APrePreserve => OperationState::Preserving,
                _ => OperationState::RollingBack,
            };
            assert_eq!(record.state, expected, "{name}");
        }
        Unbound::GVerifying => {
            assert_eq!(record.state, OperationState::Finalizing, "{name}");
            let publication = publication.unwrap();
            assert_eq!(
                publication.step,
                PublicationStep::VerifyingPublication,
                "{name}"
            );
            assert!(publication.candidate.is_some(), "{name}");
            assert!(publication.composition_commit.is_some(), "{name}");
            assert!(!publication.candidate_hashes.is_empty(), "{name}");
        }
        Unbound::HPreservingCandidate => {
            assert_eq!(record.state, OperationState::Preserving, "{name}");
            let publication = publication.unwrap();
            assert!(publication.candidate.is_some(), "{name}");
            assert!(publication.composition_commit.is_none(), "{name}");
            assert!(publication.preservation_prefix.is_none(), "{name}");
        }
        Unbound::HPreservingPrefix => {
            assert_eq!(record.state, OperationState::Preserving, "{name}");
            let publication = publication.unwrap();
            assert!(publication.candidate.is_some(), "{name}");
            assert_eq!(
                publication.preservation_prefix.as_deref(),
                Some("lock"),
                "{name}"
            );
        }
        Unbound::IEvidenceRollback => {
            assert_eq!(record.state, OperationState::RollingBack, "{name}");
            let publication = publication.unwrap();
            assert!(publication.composition_commit.is_some(), "{name}");
            assert!(!publication.evidence_rolled_back, "{name}");
        }
        Unbound::KCompletedNopubOpen => {
            assert_eq!(record.state, OperationState::Completed, "{name}");
            let publication = publication.unwrap();
            assert_eq!(publication.step, PublicationStep::Complete, "{name}");
            assert!(publication.candidate.is_none(), "{name}");
            assert!(publication.composition_commit.is_none(), "{name}");
            assert!(publication.candidate_hashes.is_empty(), "{name}");
        }
    }
}

/// **Cheap closures (i) and (ii) of `GwzM5-8R4bG-Evidence.md` §12.7, in one
/// package**, as `GwzM5-8A1ActivationRecord.md` §6 requires.
///
/// Every one of the ten unbound progress shapes is built in its own workspace,
/// asserted against R0 §4's stated combination, and then driven through
/// `adapt_open` — not through the precheck walk, which "reads state+mode and
/// never drives `adapt_open`" and which L6 ruled discharges neither closure.
/// Nine assert their `valid_unlisted_corpus` row and their byte preservation
/// through `assert_i2_valid_unlisted_fixture`; the tenth, `G-VERIFYING`, is
/// dispositioned by clause below because it is `Finalizing`.
#[test]
fn v0_unbound_progress_shapes_are_refused_by_adapt_open() {
    for shape in UNBOUND_PROGRESS_SHAPES {
        let durable = build(shape);
        assert_r0_combination(shape, &durable.record);

        let Some((case_id, subcase)) = shape.corpus_binding() else {
            g_verifying_is_dispositioned_by_clause(&durable);
            continue;
        };

        // Drives `decode -> adapt_open -> atomic upgrade` and asserts
        // `ValidUnlisted` plus byte-exact v0 preservation, against this shape's
        // own registry row.
        super::compatibility_v0::assert_i2_valid_unlisted_fixture(
            &durable.backend,
            durable.temp.path(),
            &durable.record,
            case_id,
            subcase,
        );
    }
}

/// `G-VERIFYING`'s adaptation disposition: **DISPOSITIONED-UNLISTED**, recorded
/// by clause with an executed test, which is `GwzM5-8R4bG-Evidence.md` §12.7's
/// second branch for a row no corpus can hold.
///
/// It is a `Finalizing` shape — the only operation state the whitelist adapts —
/// so `classify_open_v0`'s cheap gate does not answer it and the refusal has to
/// be *shown*. It is shown structurally: the descriptor is well formed, and its
/// `publication.step` is `verifying_publication`, a value the registry's closed
/// `publication_step` enum does not carry at all, so zero rules can equal it.
/// Contract §4, "A1 deliberately whitelists only seven one-member-workspace,
/// `Finalizing`, normal-mode shapes: before publication progress, validating,
/// candidate persisted, evidence created but unrecorded, evidence recorded,
/// exact boundary/index publication prefix, and no-publication complete before
/// the terminal state write" (`:136-144` as of 2026-08-28) — verifying
/// publication is not among the seven. Contract §5, "Zero whitelist matches is
/// not an error. Open read-only status leaves bytes unchanged" (`:178-184` as
/// of 2026-08-28) — so the answer is `ValidUnlisted` and the bytes stand.
///
/// `GwzM5-8R4bG-Evidence.md` §12.9(c) is the reason there is no registry row:
/// the `valid_unlisted_corpus` "cannot express a `Finalizing` shape … widening
/// it to admit these rows would weaken the registry, not extend it". §12.9's
/// own Finalizing ground reaches three of its four rows; this is the fourth
/// `Finalizing` row, one §12.9 never considered (E5 review [P2-4],
/// 2026-08-28 — the first landing of this doc miscounted it as a fifth).
///
/// E5 review [P3-2], executed 2026-08-28: the "zero rules can equal it" half
/// was prose beside a test that only measured *this* descriptor's zero
/// matches. It is now read off the registry — see the second block below.
fn g_verifying_is_dispositioned_by_clause(durable: &Durable) {
    let descriptor = crate::workspace_ops::merge::verified_v0_descriptor(
        &durable.backend,
        durable.temp.path(),
        &durable.record,
    )
    .unwrap();
    assert_eq!(
        descriptor.value()["publication"]["step"],
        serde_yaml::Value::String("verifying_publication".to_owned())
    );
    assert_eq!(
        super::compatibility_v0::i2_whitelist_matches(descriptor.value()),
        Vec::<String>::new()
    );

    // The structural claim, executed. The zero above is this one descriptor's;
    // the disposition rests on a stronger zero — that no rule *can* equal any
    // `verifying_publication` descriptor. Two registry facts give it: the
    // closed `publication_step` enum does not carry the token, and every rule's
    // descriptor draws its step from that enum. A rule minted with the token
    // would fail the first; a rule minted outside the enum would fail the
    // second, and the checker would reject the registry either way.
    let steps = super::compatibility_v0::i2_normalization_enum("publication_step");
    assert!(
        !steps.iter().any(|step| step == "verifying_publication"),
        "the closed publication_step enum must not carry verifying_publication: {steps:?}"
    );
    for (rule_id, step) in super::compatibility_v0::i2_whitelist_publication_steps() {
        assert!(
            steps.contains(&step),
            "{rule_id} carries a publication step outside the closed enum: {step}"
        );
    }

    let decoded = crate::workspace_ops::merge::decode_production_v0(
        serde_yaml::to_string(&durable.record).unwrap().as_bytes(),
    )
    .unwrap();
    assert_eq!(
        crate::workspace_ops::merge::adapt_open_v0(
            &durable.backend,
            durable.temp.path(),
            &decoded,
            "r3-test-writer",
        )
        .unwrap(),
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted
    );
    super::atomic_upgrade_v0::assert_valid_unlisted(
        &durable.backend,
        durable.temp.path(),
        &durable.record,
    );
}
