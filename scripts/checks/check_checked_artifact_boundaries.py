#!/usr/bin/env python3
"""Fail-closed structural inventory for the checked-artifact boundary."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

# Trust anchor for the compiler route into the protected v1 tree. Cargo's lib
# target is checked semantically below; each exact parent then selects the next
# canonical module. Hashing only the descendant tree would prove resident
# bytes without proving that rustc actually loads them.
PROTECTED_COMPILER_ROOT_DIGESTS = {
    "src/lib.rs": "e035f8a53ddb589362972c85593cc0dff4b590129de38fe0fdb72ca1880f544e",
    "src/workspace_ops/mod.rs": "663b228d1f3fddc74853d3e26f9623a0d7d2009f172f53640697de35042a8124",
    "src/workspace_ops/merge/mod.rs": "76e4830e6ca1784f3dba5326768c173bfc434f49fe56196c1f565aca8adb8200",
}

PROTECTED_COMPILER_MODULES = {
    "checked_artifact/entry.rs",
    "git/gitbackend/authority_backend.rs",
    "git/gitbackend/preservation_root/files.rs",
    "git/gitbackend/preservation_image.rs",
    "workspace_ops/merge/preserve/checked_bundle.rs",
    "workspace_ops/merge/preserve/plan.rs",
    "workspace_ops/merge/root/artifact_facts.rs",
    "workspace_ops/merge/v1_lifecycle/authority/observe.rs",
}

# Complete positive allowlist for the small production boundary. Any executable
# or non-executable source change requires deliberate review and a digest
# update; this closes aliases and new wrappers without guessing writer names.
# R2-E E4.7 (2026-09-02) re-pins THREE flat entries, for the allowance-class
# close-out -- comments, `reason` strings and expired allows only, no production
# semantics: `checked_artifact/entry.rs` (the stale "E4.2-E4.6 convert the
# consumers that will read it" at the activation door re-pointed);
# `checked_artifact/mod.rs` (six subtree allows RE-REASONED PERMANENT and the
# `pub(crate) mod entry` allow EXPIRED -- measured, it suppressed nothing);
# `operation/workspace_mutator_lock.rs` (E4.1 [P3-5]'s stale allow EXPIRED --
# `catalog_mutation_lease` has four production callers, so it suppressed
# nothing). Every other flat and tree digest was recomputed in the same pass and
# is unchanged.
PROTECTED_SOURCE_DIGESTS = {
    # R2-E E4.4-6-B (2026-09-02) pins the `write_atomic` family's own implementation:
    # the capability-free inventory counts its CALLERS, so converting THIS file would
    # convert every carved `:277`/`:278`/`:279` writer while moving no count there
    # (round 1 [P3-5]). Not a boundary module -- pinned solely as that backstop.
    "artifact/mod.rs": "22bce8182daf6865512c639957dcb16d3c91af15972bb34b25e9fdd9ae546d11",
    "checked_artifact/bootstrap.rs": "d85d894032512125ee5ad0cab770db25dd37cee32096ad84be3061eaab94b2aa",
    "checked_artifact/bootstrap/runtime/mod.rs": "1bddf4b40e4bd6454300e7b08b54119875ec19daacb819a14dbd0c483784230d",
    # R2-E E4.1 commit (b) re-pins this entry for precondition 1: the SUBSTRATE
    # that answers a durable-identity probe gains its own `PlatformCapability`
    # value, distinct from the identity VALUE contract that keeps
    # `DurableObjectIdentity`, and it is the one capability carrying an
    # actionable remedy sentence.
    "checked_artifact/capability.rs": "dcb8d2b0f74fea8033db1eeba5523788ef0742077e2eca158789369709a22df7",
    # R2-E E4.1 commit (b) re-pins this entry for O2: the boundary module gains
    # `activate_workspace_catalog`, the first production catalog activation,
    # and the four ENTRY_* inventories below move with it.
    # R2-E E4.2 re-pins it again for §10 rows `:273`/`:280`:
    # `bootstrap_merge_start_parents` and `create_merge_store_record` join the
    # door above, and [P3-2]'s renderer is extracted to a named `pub(super)` fn
    # so its three arms take an in-suite guard.
    "checked_artifact/entry.rs": "272675b6163641a2186d964aee46fa75926781644ff3e8f925f058c91b39f845",
    "checked_artifact/authority.rs": "fd300c5b8fb9dfacd41a4f0c6c39923fc8decbb07a6933af2eaa471c4ebdf1ed",
    "checked_artifact/mod.rs": "cffba4530283bb2e1a99cb3b1947e8d43a4a705d70e2c58b347929e06db98933",
    # R2-E E4.1 commit (a) re-pins this entry for the E7 dual's Code [P3 F3]:
    # `inspect_family`'s 1 MiB budget now charges `DirEntry::metadata().len()`
    # in the enumeration loop, before any leaf is read, and the post-read
    # accumulation it replaces is gone.
    "checked_artifact/residue.rs": "8894be425ddd6755aa053a4e42aca540611ba45c688b42c4757343be5142349a",
    "checked_artifact/transition.rs": "13b483bc0dc3099082727a5d499b97f627ba7d41a65b929ec557416ac59b37ca",
    "git/gitbackend/authority_backend.rs": "0abb856d03118b0d304170beab3fcd8e18e3ae4c3b7860f66771351849c14ff1",
    "git/gitbackend.rs": "b85dfd3f32671886a34d2bee5c79200dc6da74a9f99fd5cfa0fe1d801667b3fb",
    "git/gitbackend/preservation_root/files.rs": "7a6b72ac62a91a48992b04a563d85354dcef950aad420c610e7a08c3c2409b35",
    "git/gitbackend/preservation_image.rs": "b45057e105a74d50c5163886d3346e9ea859464971c4cd03fc49392c5b67bac5",
    "workspace_ops/merge/preserve/artifacts.rs": "c2f97f284c6e9ad241184d8db0bdbb7e8e5d8afe2890b3e08333d8bd212e71d9",
    "workspace_ops/merge/preserve/checked_bundle.rs": "dbc3e4de328afefbedd3ee343c0bf384b2852d499e3f007960159ff229595251",
    "workspace_ops/merge/preserve/plan.rs": "3730179e156151c4a853752ec769712d3ae81bd21e7729b892ab4cb14474ff89",
    "workspace_ops/merge/root/artifact_facts.rs": "d4bb3d895070c4bafbb6ee8fed2664768b6e4d6be43fe764f877add4f4c42f19",
    "operation/workspace_mutator_lock.rs": "c390191ea03c64d635ae80de0405cd213a6f067d9648c4735801062330019b0b",
}

CONCRETE_PRESERVATION_OBSERVER_REFERENCES = {
    "git/gitbackend.rs",
    "workspace_ops/merge/preserve/checked_bundle.rs",
    "workspace_ops/merge/preserve/plan.rs",
    "workspace_ops/merge/v1_lifecycle/authority/observe/reverse/preservation/phase.rs",
    "workspace_ops/merge/v1_lifecycle/authority/observe/reverse/preservation/phase/evidence.rs",
}

# Rust permits `#[path]` modules to name any file suffix. Freeze every approved
# edge, require its target to remain a regular in-crate `.rs` file, and reject
# `include` entirely so the source inventory matches the compiler-loaded graph.
APPROVED_RUST_PATH_EDGES = {
    (
        "checked_artifact/capability/pre_catalog/provider/platform.rs",
        "platform/linux.rs",
    ),
    (
        "checked_artifact/capability/pre_catalog/provider/platform.rs",
        "platform/macos.rs",
    ),
    (
        "checked_artifact/capability/pre_catalog/provider/platform.rs",
        "platform/unsupported.rs",
    ),
    (
        "checked_artifact/capability/pre_catalog/provider/platform.rs",
        "platform/windows.rs",
    ),
    ("checked_artifact/mod.rs", "tests.rs"),
    ("checked_artifact/tests.rs", "tests/durability.rs"),
    ("checked_artifact/tests.rs", "tests/exact_source.rs"),
    # R2-D Phase 4 Step 4.1: the four converted legacy leaf edges and the
    # sealed leaf publication they route through.
    ("checked_artifact/tests.rs", "tests/leaf_publication.rs"),
    ("checked_artifact/tests.rs", "tests/recovery_protocol.rs"),
    ("checked_artifact/tests.rs", "tests/removal_recovery.rs"),
    ("checked_artifact/tests.rs", "tests/staging_recovery.rs"),
    ("lib.rs", "../protocol/corpus/rust/vectors.rs"),
    ("lib.rs", "cbor.rs"),
    ("protocol/mod.rs", "generated.rs"),
    ("workspace_ops/merge/mod.rs", "tests/acceptance_v0/mod.rs"),
    ("workspace_ops/merge/mod.rs", "tests/transition_matrix_v0.rs"),
    (
        "workspace_ops/merge/participant_semantics/continue_eligibility.rs",
        "continue_eligibility_tests.rs",
    ),
    ("workspace_ops/merge/participant_semantics/result.rs", "result_tests.rs"),
    (
        "workspace_ops/merge/participant_semantics/rollback.rs",
        "rollback_tests/mod.rs",
    ),
    (
        "workspace_ops/merge/participant_semantics/status.rs",
        "status_tests/mod.rs",
    ),
    ("workspace_ops/merge/plan.rs", "plan/tests.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/durable_recovery.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/event_sink.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/execution.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/prepared_recovery.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/resolution_race.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/resolution_validation.rs"),
    ("workspace_ops/merge/start.rs", "start/tests/root_execution.rs"),
    ("workspace_ops/merge/v1_lifecycle/archive.rs", "tests/archive.rs"),
    ("workspace_ops/merge/v1_lifecycle/archive.rs", "tests/gc.rs"),
    (
        "workspace_ops/merge/v1_lifecycle/finalization.rs",
        "tests/finalization.rs",
    ),
    (
        "workspace_ops/merge/v1_lifecycle/finalization.rs",
        "tests/finalization_inputs.rs",
    ),
    (
        "workspace_ops/merge/v1_lifecycle/finalization.rs",
        "tests/finalization_root.rs",
    ),
    (
        "workspace_ops/merge/v1_lifecycle/reverse/preservation.rs",
        "../tests/reverse_preservation/mod.rs",
    ),
    (
        "workspace_ops/merge/v1_lifecycle/reverse/rollback.rs",
        "../tests/reverse_rollback/mod.rs",
    ),
    ("workspace_ops/merge/v1_lifecycle/service.rs", "tests/service.rs"),
    (
        "workspace_ops/merge/v1_lifecycle/service.rs",
        "tests/service_sequence.rs",
    ),
    ("workspace_ops/merge/v1_lifecycle/status.rs", "tests/status.rs"),
}

# Module-tree roots are protected as one path-and-byte manifest. This includes
# the root module, every current descendant, and the descendant file set, so a
# nested helper, a new source file, or a changed module edge fails closed.
#
# R2-E Phase E2 re-pins deliberately (GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT
# §6.2(b), §6.3's amended §3.6 duty list). Two of the three moves were
# forecast there: `.../pre_catalog.rs`, whose descendant root gains the new
# `provider/barrier_mutation.rs` (DECISION B-1), and `.../platform.rs`, which
# gains the third `DirentBarrierClass` variant (DECISION B-3). The third,
# `checked_artifact/catalog.rs`, was NOT in that inventory and is recorded here
# rather than absorbed: O6's witness reaches the barrier owner through
# `OpaqueRetainedCatalogV1`, whose forwarder lives in `catalog/bootstrap.rs`
# under this tree.
#
# R2-E Phase E6.2b re-pins ONE entry, `checked_artifact/platform.rs`, for the
# executed anchor nit (E0.2b §7.2 / `GwzM5-8R2DSettledTuple.md:659-662`,
# authorized by the lane owner 2026-08-28): `platform/anchor.rs`'s `survey`
# admits a retired ordinal only if `retired_name` would have produced that
# exact name, and `platform/anchor/tests.rs` gains the row that drives it. The
# other six digests were recomputed in the same pass and are unchanged, which
# is the evidence that the edit stayed inside the anchor protocol.
#
# The R2-E E6 landing reconcile re-pins the SAME entry once more, for the
# review's three fold-in cures (GwzM5-8R2E-E6-Review.md F-1/F-2/F-3,
# authorized by the lane owner 2026-08-28): the module doc's closed-grammar
# table row corrected to `.ca1-anchor-retired-<ordinal>` (F-1, the name the
# survey actually adopts), the refusal test strengthened to assert the whole
# directory listing unchanged across the refusal (F-2), and the survey
# comment gaining the F-3 trade sentence (slot-wastage exchanged for a
# recoverable fail-closed refusal on the foreign shape). Comments and one
# test assertion only -- no production semantics move; the other six digests
# were recomputed in the same pass and are unchanged.
#
# R2-E E4.1 commit (a) re-pins the SAME entry once more, for [R2-P3-3]'s
# one-word cost-claim fix (`GwzM5-8R2E-E7-Acceptance.md` §5 record act 2): the
# roaming survey's cost sentence said "two `symlink_metadata` calls" while
# `leaf_is_resident` is a full bounded leaf observation. Comment only -- no
# production semantics move; the other six digests were recomputed in the same
# pass and are unchanged.
# R2-E E4.1 commit (b), the activation package (O2), re-pins THREE tree
# entries: `capability/pre_catalog.rs` (the four platform providers' substrate
# refusals move to `PersistentFilesystemIdentity`, and the unsupported stub's
# Linux-profile claim is swept to a named unreachable placeholder),
# `catalog.rs` (the owner is activated: the blanket `dead_code` allow retires to
# the two admitted-action capabilities that stay dead, and the restart-arm row
# lands in its own suite), and `v1_lifecycle/mod.rs` (the checked prologue calls
# the activation door, and its ordering row lands beside it). The other four
# digests were recomputed in the same pass and are unchanged.
#
# R2-E E4.1 commit (c) re-pins ONE of them, `v1_lifecycle/mod.rs`: activation
# moves off the shared prologue onto `acquire_activated`, so the reverse (abort)
# arms stay capability-free. The other six were recomputed and are unchanged.
#
# R2-E E4.2 re-pins TWO: `capability/pre_catalog.rs`, for §11.3 item 2(a)'s
# dated disposition at `retain_managed_parent_at_for_test` (comment only); and
# `v1_lifecycle/mod.rs`, whose creation lease gains `acquire_for_merge_start`,
# bootstrapping §10 row `:273`'s two managed parents before `create_open`, with
# the creation path publishing through the checked boundary rather than its own
# raw durable writers. The other five were recomputed and are unchanged.
#
# R2-E E4.3-B (2026-09-02) re-pins ONE, `v1_lifecycle/mod.rs`: the exception's
# dated `///` at `store/rewrite.rs::commit` (doc only) and P-2's tripwire module
# with its `mod` declaration. The other six were recomputed unchanged;
# `bootstrap/managed.rs`, re-dated by this package, is under NO entry -- measured.
#
# R2-E E4.7 (2026-09-02) re-pins THREE tree entries, all comment/`reason`-only:
# `catalog.rs` (its four allows re-reasoned PERMANENT pending DR-1);
# `capability/pre_catalog.rs` (`provider.rs`'s `authority_record_binding` and
# `barrier_mutation` allows re-reasoned, `leaf_observation`'s EXPIRED --
# measured, nothing in that module is dead even with the `mod capability`
# blanket lifted); and `v1_lifecycle/mod.rs`, for the three [R2-P3-1] dated
# residual sentences at `finalization/execute.rs:45,:48,:51` with the operator's
# ruling (a) quoted in that file's header, and the `gc_archived` allowance
# re-reasoned PERMANENT PENDING DR-1 rather than deleted. NO count in any row of
# `V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES` or of
# `CAPABILITY_FREE_RAW_WRITER_INVENTORY` moves: this step edits only masked-out
# comments and string contents. The other four tree digests were recomputed in
# the same pass and are unchanged.
PROTECTED_SOURCE_TREE_DIGESTS = {
    "checked_artifact/bootstrap/runtime/catalog_lease.rs": "91ac3dfada76860dda1d41a0c3cad66f6836229680773b1b1644e4aabe20b0b2",
    "checked_artifact/capability/path.rs": "23e46dbde50a0530c331c34dd68a9d40096394c6817075d3f66ad3f0e27a91c6",
    "checked_artifact/capability/pre_catalog.rs": "e02937db60c39e2a37f2b8432ae0c8fe6144d053784e192f6d72b5e3aced2522",
    "checked_artifact/catalog.rs": "71e1b8de7e4e14cc33b5387155d2029e20086f57fcd8bbf62b6b286a8c2cf95d",
    "checked_artifact/platform.rs": "c464666735aae2028fa75f9d6063eb6122f95ea1e3f0a39b3e4f18cd9293d094",
    "workspace_ops/merge/v1_lifecycle/authority/observe.rs": "d16fa8bf67f8656c56b3c51d6625712efcc970dfd51afefa77557df5b3fcae38",
    "workspace_ops/merge/v1_lifecycle/mod.rs": "8e436f932fc1ee8718a9c64e26c8784517b87b308a26dcbec38ab14ed1d72bd8",
}

# Every permitted raw-rename reference in production checked-artifact source,
# by masked-source BARE-IDENTIFIER count (definitions excluded). Counting
# name references rather than call shapes makes use-aliasing, fn-pointer
# binding, and turbofish spellings fail closed: any rebinding must name the
# item at least once (rounds Code-4/State-3 [P3-1]). publication.rs is the
# sealed primitive's own platform pair; platform.rs is internal composition,
# the legacy Windows durability anchor, its in-file windows test module
# (imports included), and — added by R2-D Phase 4 Step 4.1 — the sealed
# leaf publication `publish_verified_leaf_no_replace`, which composes the same
# P1 pair for the legacy leaf family (freeze §4.1 row P1, §4.3 rows E18-E21).
#
# R2-D Phase 4 Step 4.1 RETIRED the transition.rs and residue.rs entries: the
# four legacy leaf edges the previous comment described now route through that
# one sealed composition, so no legacy leaf writer names a raw rename at all
# and the whole raw-rename surface of the subsystem is the two files below.
#
# R2-D Phase 4 Step 4.2 (freeze §4.3 row E22) took the four remaining
# `rename_relative` references with it. The legacy Windows durability anchor
# held them — two in its create/return arms and two in the barrier round trip —
# and its closed successor in `checked_artifact/platform/anchor.rs` publishes
# every anchor edge through the same P1 composition instead, so that file needs
# no entry here at all. `rename_relative` now has exactly ONE reference in the
# whole subsystem: `rename_open_source`'s own non-Windows delegation.
#
# Any other reference anywhere in the subsystem violates the single-seam rule
# (RemPlan publication-correction clause; amendment §8.13) and fails closed
# here.
RAW_RENAME_CALL_ALLOWLIST = {
    "checked_artifact/capability/pre_catalog/provider/publication.rs": {
        "open_rename_source": 1,
        "rename_open_source": 1,
    },
    "checked_artifact/platform.rs": {
        "open_rename_source": 6,
        "rename_open_source": 6,
        "rename_relative": 1,
    },
}
RAW_RENAME_TOKENS = ("open_rename_source", "rename_open_source", "rename_relative")

# --- R4b-G F-3 / inventory W2 / evidence row 2.6b -------------------------
#
# The other half of the call-graph gate: `v1_lifecycle/` must contain no call
# into the v0 merge persistence seam. The property holds today at 0 hits
# INCLUDING test code, so this pins a true statement rather than repairing a
# violation -- but nothing failed closed if it changed.
#
# LOAD-BEARING FOR JUDGMENT CALL J-1. The frozen M5b dependency statement
# (`GwzM5-8M5bNoFfDesign.md:976-989`, reason (c)) makes M5b's own
# unreachability argument lean on "R4b-G's call-graph gate". M5b-IMPL is
# already merged ahead of R4b-G, so that argument was leaning on an absent
# gate; this scan is the gate it names. Removing or weakening it re-opens
# J-1 and must not be done without the R4b-G lane owner's ruling.
#
# The seam is DERIVED from `workspace_ops/merge/mod.rs`'s own `use store::{..}`
# re-exports rather than hardcoded, so a newly exported v0 persistence item is
# covered the day it is added; `V0_PERSISTENCE_SEAM_FLOOR` fails the derivation
# closed if that re-export shape is restructured away.
#
# This is NOT subsumed by the `PROTECTED_SOURCE_TREE_DIGESTS` pin on
# `v1_lifecycle/mod.rs`. That digest says only "this tree changed, go look",
# and the lane refreshes it every time the tree legitimately moves; it states
# no property, so a refresh can carry a new v0 persistence call through
# unremarked. This scan states the property, and survives every refresh.
#
# Bare-identifier counting on MASKED source is what makes this exact: ten
# `"enter_finalizing"` occurrences inside `v1_lifecycle/` are action-name
# string literals, which `mask_non_code` blanks, so a naive grep's ten false
# positives become the true zero. Definitions (`fn <name>`) are excluded the
# same way the raw-rename scan excludes them.
V1_LIFECYCLE_TREE = "workspace_ops/merge/v1_lifecycle"
V0_STORE_REEXPORT = re.compile(r"\buse\s+store::\{([^}]*)\}\s*;")
V0_PERSISTENCE_SEAM_FLOOR = frozenset(
    {
        "FileMergeStore",
        "MergeStore",
        "archive_merge_record",
        "enter_finalizing",
        "persist_merge_record",
        "persist_operation_transition",
    }
)

# The authority both exception maps cite for the `:275`-`:279` carve-out (E4.4-6-B).
CAPABILITY_FREE_EXCEPTION = "the capability-free exception, dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md §3"

# O13 raw-writer pin (2026-08-27, R2-E E0 landing; `GwzM5-8R2E-Plan.md` §1.1
# O13, addendum §7.6.1). ConsumerCheckpoint §10 row `:280` forbids a legacy
# raw writer on the v1 checked store/root/bundle paths, and its "test-gated
# until A1" gate expired when A1 shipped (2026-08-25). The conversion that
# discharges the clause is R2-E E4.2/E4.3's; until it lands, the files below
# are the ACCEPTED RESIDUAL — the complete non-test `durable_fs` surface of
# `v1_lifecycle/` at the pin date. The set may only SHRINK: a new file naming
# `durable_fs` under `v1_lifecycle/` fails closed here, and E4.2/E4.3 retire
# entries to empty deliberately, in their own commits, each retirement taking
# a dated comment in this pin's established form.
#
# E4.2 (2026-09-01) WIDENS the pin from a bare file set to per-file raw-writer
# COUNTS, and moves them. The file set alone could not record this step: E4.2
# owns the store's CREATION path and E4.3 its rewrite path, both in
# `store/rewrite.rs`, so converting `create_open` retires no file -- while
# `archive.rs` and `store/archive.rs` are the terminal-archive row's, E4.4's,
# and were never E4.2/E4.3's to retire at all. (A correction on the record to
# the plan's E4.3 note, "the O13 checker inventory retires to empty across
# E4.2/E4.3": two of its three files belong to a third step.) Counting bare
# identifiers on masked source, in `RAW_RENAME_CALL_ALLOWLIST`'s own idiom,
# makes each conversion measurable in its own commit and stays fail-closed BOTH
# ways. `create_open`'s publication moved to the checked boundary, so
# `store/rewrite.rs` drops from three references of each writer to two -- the
# `use` and `commit`'s call, exactly E4.3's remaining half.
#
# R2-E E4.3-B (2026-09-02) makes the `store/rewrite.rs` row PERMANENT-DOCUMENTED
# rather than retire-on-conversion, under `dev-docs/GwzM5-8R2E-
# RecordRootAmendment.md` §2's RECORD-ROOT EXCEPTION and §3's P-1: a dated,
# dual-reviewed exception to row `:280`'s "no legacy raw writer" clause, scoped
# to `commit` and only it, because the record is the ROOT of reconciliation and
# the boundary's detach-then-publish shape opens a discovery-dead window no
# shipped reconciler closes (§1a, driven). So the carved row fails closed both
# ways with DIFFERENT meanings: growth is an unblessed new raw writer, while
# SHRINKAGE -- the direction that used to DEMAND retirement -- says a partial
# conversion may not land until the amendment is revised, at O14's fork. The
# marker is a per-row REASON in `V1_LIFECYCLE_PERMANENT_WRITER_EXCEPTIONS`,
# general by design: the 2026-09-02 capability-free ruling foresees further
# `:275`-`:279` carve-outs, one data row each AT THE DATA LAYER — each row carries
# its own authority and both directions fire naming it; widening the SCAN SET
# to the non-v1 carved files is the pins package's, LANDED BELOW as
# `CAPABILITY_FREE_RAW_WRITER_INVENTORY` (amendment §3 (i)-(iii)). The two
# ARCHIVE rows keep their retire-on-conversion marker until E4.4 (§6); class
# scope is `durable_fs` only, a std::fs writer here being backstopped by
# `PROTECTED_SOURCE_TREE_DIGESTS` and stated as a property by P-2.
#
# R2-E E4.4-6-B (2026-09-02) makes the two ARCHIVE rows permanent as well, under
# `dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md` §3: the terminal archive runs
# from every terminal disposition on the PLAIN lease, so converting it would put an
# operation E0.2 §5.2 keeps capability-free onto the durable-identity probe. ALL
# THREE ROWS ARE NOW PERMANENT-DOCUMENTED and this inventory NEVER EMPTIES -- the
# plan's ":90 empties across E4.2-E4.4" and ":110 empties for the CONVERTIBLE
# files" are superseded (§4). "Until E4.4" above is spent: E4.4 as chartered does
# not start and E4.7 retires none of the three. The archive rows' `std::fs` surface
# is measured ONCE, by the inventory below.
V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES = {
    "workspace_ops/merge/v1_lifecycle/archive.rs": {"sync_dir": 2},
    "workspace_ops/merge/v1_lifecycle/store/archive.rs": {"rename_noreplace": 2, "sync_dir": 7},
    "workspace_ops/merge/v1_lifecycle/store/rewrite.rs": {"rename_durable": 2, "sync_dir": 2},
}
V1_LIFECYCLE_RAW_DURABLE_WRITERS = ("rename_durable", "rename_noreplace", "sync_dir")

# Rows carved out PERMANENTLY by a dated amendment, each naming its own reason
# and authority. Membership changes nothing this checker MEASURES and everything
# it SAYS when a row moves, in BOTH directions. Adding a row is an
# amendment-tier act, not a checker edit; the shape is deliberately general so a
# follow-on amendment's further §10 carve-outs are one data row each within
# this v1_lifecycle scan; the wider scan set landed as
# `CAPABILITY_FREE_RAW_WRITER_INVENTORY` below.
V1_LIFECYCLE_PERMANENT_WRITER_EXCEPTIONS = {
    "workspace_ops/merge/v1_lifecycle/archive.rs": CAPABILITY_FREE_EXCEPTION,
    "workspace_ops/merge/v1_lifecycle/store/archive.rs": CAPABILITY_FREE_EXCEPTION,
    "workspace_ops/merge/v1_lifecycle/store/rewrite.rs": "the record-root exception, dev-docs/GwzM5-8R2E-RecordRootAmendment.md §2/§3",
}
for _key in sorted(V1_LIFECYCLE_PERMANENT_WRITER_EXCEPTIONS.keys() - V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES.keys()):
    raise SystemExit(
        f"check_checked_artifact_boundaries: permanent writer exception {_key!r} "
        f"({V1_LIFECYCLE_PERMANENT_WRITER_EXCEPTIONS[_key]}) names no pinned row -- "
        "an exception must sit on a row of V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES"
    )

# THE CAPABILITY-FREE RAW WRITER INVENTORY (R2-E E4.4-6-B, 2026-09-02), under
# `dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md` §3 and the operator ruling of
# the same date. E0.2 §5.2's capability-free list STANDS, so every §10 row
# `:275`-`:279` durable writer reached from `gwz repo create`, `init-from-sources`,
# an ordinary merge, `gwz commit`, either abort form, GC, or an operation under
# the mutation guard (read BROADLY: commit, stage, materialize, branch, repo
# lifecycle, pull, stash) KEEPS its raw publication primitive PERMANENTLY --
# converting one would place its operation on the durable-identity probe, which
# the list forbids. A DATED EXCEPTION, not unfinished work: no E4 step owes the
# conversion and E4.7 does not retire it.
#
# Mechanism: O13's above -- count per primitive per file on masked source,
# fail-closed both ways -- with the amendment's three generalizations. (i) An
# EXPLICIT carved-file list under `src/` replaces the `v1_lifecycle/` scan root:
# these writers live across the whole crate. (ii) The `\bdurable_fs\b` population
# gate is DROPPED; fourteen of these files never name it. (iii) The vocabulary
# widens to all THREE primitive classes of §1, every spelling countable by the
# bare-identifier idiom (`\bcreate_dir_all\b` matches `fs::create_dir_all`); an
# inventory counting only `durable_fs` names would read ZERO at most carved files.
# Counts are PIN REFERENCES, not call sites: `use` lines and definitions count, as
# O13's do. ONE departure from O13, load bearing: TOP-LEVEL `#[cfg(test)] mod`
# blocks are dropped first -- without it `stash/mod.rs` would pin eleven references
# of which eight are its own tests'. The drop is deliberately narrow: `cfg(all(test,
# ...))` and an INDENTED `#[cfg(test)] mod` are NOT dropped, so they OVER-count,
# which an exact-count pin raises immediately (round 1 [P3-3]). No carved file
# carries either shape today.
#
# MEASURED EXACTLY ONCE. Two rows are also O13 rows; O13 stays, being a SCAN --
# fail-closed against a `durable_fs` file it has never seen under `v1_lifecycle/`,
# which a list cannot be -- so for a file O13 already holds this map measures only
# the classes O13 does not name, and the assertion below refuses any overlap.
# `store/rewrite.rs` is NOT here: the record root is RR's carve, pinned by O13 and
# `tests/store/record_root_exception.rs`.
#
# Digest coverage, MEASURED with `source_tree_digest`'s own semantics (a `mod.rs`
# tree root digests its WHOLE parent subtree), because the amendment's §3 and Code
# axis [P2-5] state it wrongly: THREE of the twenty are pinned -- flat for
# `preserve/artifacts.rs`, and by the `v1_lifecycle/mod.rs` TREE root for both v1
# archive files, a root that also covers `store/rewrite.rs`, so RR §3 P-1's
# backstop and the `:366` note above are TRUE. Seventeen are unpinned. The CHOICE
# stands regardless: a digest only says "this tree changed, go look" and is
# refreshed on every legitimate edit, so the classes go into THIS map, which
# states the property and survives every refresh. THREE corrections to the
# amendment's §1 table, measured here: `store/archive.rs` has THREE raw `std::fs`
# mutations, not "four"; `handle_stash/commands.rs` is a second carved `:276` home
# it omits, under the StashMutate guard; and the v0 terminal archive
# `store/archived.rs::archive` -- reached from ordinary v0 merge finalization
# (`finalize_dispatch.rs:34`, `finalize_support.rs:99`) and from BOTH abort forms
# (`abort/mod.rs:111,:189,:218`) via `archive_merge_record` -- is named by NEITHER the
# amendment, the charter prep, NOR either axis of its dual (round 1 [P2-1]).
CAPABILITY_FREE_WRITER_TOKENS = (
    "rename_noreplace", "rename_durable", "sync_dir",  # `durable_fs`
    "create_dir_all", "remove_file",  # `std::fs`-direct
    "write_atomic", "write_marker", "write_lock", "write_manifest_and_lock",  # the
    "write_bundle", "publish_workspace_exclude_candidate",  # `write_atomic` family
    "sync_workspace_boundary", "ensure_workspace_exclude",
)

# Per carved file: its §10 row and reached operation, then the primitives and their
# pin-reference counts. Every row's authority is `CAPABILITY_FREE_EXCEPTION`. The
# key-set DIGEST below is the third direction -- a row added, deleted or swapped --
# for all twenty. (A crate-wide namer closure over the leaf-publication spellings
# would also catch a new raw writer in a file NOT listed here; it was drafted and
# dropped for the line budget, and its allowlist is NOT reconstructable from this
# text -- round 1 [P3-4].)
#
# SCOPE LIMIT, stated (round 1 [P3-5]): this map counts references to the
# `write_atomic` family's NAMES at their CALL sites, so converting the family's own
# implementation in `artifact/mod.rs` would convert every carved `:277`/`:278`/`:279`
# caller at a stroke while moving no count here and naming no door in any scanned
# file. That cheapest defeat is closed by the flat `PROTECTED_SOURCE_DIGESTS` row on
# `artifact/mod.rs`, which this package adds for exactly this reason.
CAPABILITY_FREE_RAW_WRITER_INVENTORY: dict[str, tuple[str, dict[str, int]]] = {
    "stash/mod.rs": (":276 the `gwz stash` bundle writer, mutation guard", {"write_atomic": 2, "write_bundle": 1}),
    "workspace_ops/handle_branch.rs": (":278/:279 `gwz branch`, BranchMutate guard", {"write_lock": 1, "sync_workspace_boundary": 1}),
    "workspace_ops/handle_commit.rs": (":277/:278/:279 `gwz commit`, mutation guard", {"create_dir_all": 1, "write_marker": 1, "write_lock": 1, "sync_workspace_boundary": 2}),
    "workspace_ops/handle_create_repo.rs": (":278/:279 workspace create (bare lock), repo create and add-existing (RepoMutate guard)", {"write_manifest_and_lock": 4, "sync_workspace_boundary": 4}),
    "workspace_ops/handle_init_from_sources.rs": (":278/:279 `init-from-sources`, bare lock", {"write_manifest_and_lock": 1, "sync_workspace_boundary": 1}),
    "workspace_ops/handle_materialize.rs": (":278/:279 `gwz materialize`, mutation guard", {"write_lock": 3, "sync_workspace_boundary": 3}),
    "workspace_ops/handle_repo_lifecycle.rs": (":278/:279 repo lifecycle, RepoMutate guard", {"write_manifest_and_lock": 3, "sync_workspace_boundary": 3}),
    "workspace_ops/handle_stage.rs": (":279 `gwz stage`, mutation guard", {"ensure_workspace_exclude": 1}),
    "workspace_ops/handle_stash/commands.rs": (":276 `gwz stash`'s bundle callers, StashMutate guard", {"remove_file": 1, "write_bundle": 6}),
    "workspace_ops/merge/abort/evidence.rs": (":277/:278/:279 v0 abort, the `rollback_evidence` ARM", {"remove_file": 1, "write_atomic": 1, "publish_workspace_exclude_candidate": 1}),
    "workspace_ops/merge/abort/preflight.rs": (":278 the abort preflight's `restore_baseline` ARM", {"write_atomic": 2}),
    "workspace_ops/merge/finalize.rs": (":277/:278/:279 ordinary v0 merge publication", {"write_atomic": 2, "publish_workspace_exclude_candidate": 2}),
    "workspace_ops/merge/preserve/artifacts.rs": (":276/:277/:279 v0 `--abort --preserve`", {"remove_file": 1, "write_atomic": 3, "write_bundle": 1, "publish_workspace_exclude_candidate": 1}),
    "workspace_ops/merge/store/archived.rs": (":275 the v0 terminal archive -- ordinary merge finalization and BOTH abort forms", {"rename_durable": 1, "sync_dir": 2, "create_dir_all": 1, "remove_file": 1}),
    "workspace_ops/merge/store/gc.rs": (":275 the LIVE GC deletion writer, WorkspaceMutatorLock", {"sync_dir": 1, "remove_file": 1}),
    "workspace_ops/merge/store/retention.rs": (":275 GC retention enforcement, the same lock", {"sync_dir": 1, "remove_file": 1}),
    "workspace_ops/merge/v1_lifecycle/archive.rs": (":275 the DEAD `remove_archive` arm behind the `:108-111` allowance", {"remove_file": 1}),
    "workspace_ops/merge/v1_lifecycle/store/archive.rs": (":275 terminal archive, every terminal disposition on the PLAIN lease", {"create_dir_all": 1, "remove_file": 2}),
    "workspace_ops/pull_head_member_preflight.rs": (":278/:279 `gwz pull`, Pull guard", {"write_lock": 3, "sync_workspace_boundary": 2}),
    "workspace_ops/sync_workspace_boundary.rs": (":279 the `.git/info/exclude` family itself", {"write_atomic": 2, "publish_workspace_exclude_candidate": 1, "sync_workspace_boundary": 1, "ensure_workspace_exclude": 2}),
}
if hashlib.sha256(
    "\n".join(sorted(CAPABILITY_FREE_RAW_WRITER_INVENTORY)).encode("utf-8")
).hexdigest() != "867c580f625d7efe0cf72dcc8e0ad01e36268d1478829a469eb0f57953dbd385":
    raise SystemExit(
        "check_checked_artifact_boundaries: the capability-free carved SET moved -- a row "
        "added, DELETED or swapped. It is the amendment's, not a checker edit: revise "
        "GwzM5-8R2E-CapabilityFreeAmendment.md §3"
    )
for _row, (_, _counts) in sorted(CAPABILITY_FREE_RAW_WRITER_INVENTORY.items()):
    if _row in V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES and set(_counts) & set(
        V1_LIFECYCLE_RAW_DURABLE_WRITERS
    ):
        raise SystemExit(
            f"check_checked_artifact_boundaries: carved site {_row!r} would be measured "
            "TWICE -- O13 above already counts that file's durable_fs class"
        )


ENTRY_REFERENCES = {
    # R2-E Phase E4 Step E4.1 (O2): the first production catalog activation.
    # `recover_or_create` is `pub(in crate::checked_artifact)`, so its caller
    # must live inside that tree and this boundary module is where it lives;
    # the operation that calls the door is the checked v1 prologue, which holds
    # `WorkspaceMutatorLock` across the whole operation (E0.2 §5.2). A SECOND
    # caller is an E4.2-E4.6 conversion and moves this row deliberately, exactly
    # as `interface_tests/catalog_activation_pin.rs` moves with it.
    # [2026-09-02, R2-E E4.4-6-B: the "E4.2-E4.6" RANGE is STALE -- E4.4-E4.6 as
    # chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); the movers
    # left are E4.5/6-B's three `finalization/execute.rs` arms. E4.7 expires or
    # re-reasons this allowance class, not this package, which only dates it.]
    # [2026-09-02, R2-E E4.7: the bracket above is CORRECTED on its second
    # sentence, by the operator's ruling (a) of the same date, quoted in full at
    # `finalization/execute.rs`. There are NO movers left. E4.5-B does not open;
    # none of the three `finalization/execute.rs` arms converts; all three stay
    # RAW as the [R2-P3-1] dated residual -- `:48`/`:51` on the
    # observation-dead-window ground and `:45` on the directional-residue
    # ground. Phase E4's conversions are E4.1 and E4.2 and nothing else, so no
    # remaining R2-E step adds a `recover_or_create` namer and this row's SECOND
    # caller, if one ever arrives, is DR-1's -- it moves this row and
    # `interface_tests/catalog_activation_pin.rs` together, deliberately. E4.7
    # is the step that expires or re-reasons the class, and it has: the class
    # is dispositioned at each site.]
    #
    # E4.1 review [P1-1] cure adds the SECOND caller: the A1 adapter, proving
    # the destination lifecycle viable before its durable v0->v1 upgrade.
    "activate_workspace_catalog": {
        "workspace_ops/merge/runtime/dispatch.rs",
        "workspace_ops/merge/v1_lifecycle/checked.rs",
    },

    # R2-E Step E4.2 — ConsumerCheckpoint §10 row `:273`, the first merge
    # record: the parent half is its own door because the frozen ordering makes
    # it a step that completes before the record's action begins; the leaf half
    # is O13's creation-path conversion, row `:280`. `recover_or_create`'s
    # further calls are inside THIS file, so `catalog_activation_pin.rs` — which
    # counts FILES outside the owner, not call sites — stays at one.
    "bootstrap_merge_start_parents": {
        "workspace_ops/merge/v1_lifecycle/checked.rs"
    },
    "create_merge_store_record": {
        "workspace_ops/merge/v1_lifecycle/store/rewrite.rs"
    },

    "MergeArtifactFact": {"workspace_ops/merge/root/artifact_facts.rs"},
    "MergeArtifactTransition": {
        "git/gitbackend/preservation_root.rs",
        "git/gitbackend/preservation_root/files.rs",
        "workspace_ops/merge/preserve/checked_bundle.rs",
        "workspace_ops/merge/root/artifact_facts.rs",
    },
    "classify_merge_preservation_bundle": {
        "workspace_ops/merge/preserve/checked_bundle.rs"
    },
    "classify_merge_preservation_workspace": {
        "git/gitbackend/preservation_root/files.rs"
    },
    "classify_remove_merge_root_artifact": {
        "workspace_ops/merge/root/artifact_facts.rs"
    },
    "classify_replace_merge_root_artifact": {
        "workspace_ops/merge/root/artifact_facts.rs"
    },
    "observe_merge_preservation_bundle": {
        "workspace_ops/merge/preserve/checked_bundle.rs"
    },
    "observe_merge_preservation_git_directory": {
        "git/gitbackend/preservation_root/files.rs"
    },
    "observe_merge_preservation_workspace": {
        "git/gitbackend/preservation_root/files.rs"
    },
    "observe_merge_root_artifact": {"workspace_ops/merge/root/artifact_facts.rs"},
    "prepare_merge_store_parents": {"workspace_ops/merge/store/mod.rs"},
    "remove_merge_root_artifact": {"workspace_ops/merge/root/artifact_facts.rs"},
    "replace_merge_preservation_bundle": {
        "workspace_ops/merge/preserve/checked_bundle.rs"
    },
    "replace_merge_preservation_workspace": {
        "git/gitbackend/preservation_root/files.rs"
    },
    "replace_merge_root_artifact": {"workspace_ops/merge/root/artifact_facts.rs"},
}

ENTRY_ITEMS = {
    "activate_workspace_catalog",
    # E4.2's four: rows `:273`/`:280`'s doors and [P3-2]'s renderer and label.
    "CATALOG_LABEL",
    "bootstrap_merge_start_parents",
    "create_merge_store_record",
    "render_catalog_refusal",
    "MergeArtifactFact",
    "MergeArtifactTransition",
    "classify_expected",
    "classify_merge_preservation_bundle",
    "classify_merge_preservation_workspace",
    "classify_remove_merge_root_artifact",
    "classify_replace_merge_root_artifact",
    "fact",
    "map_fact",
    "map_transition",
    "matches_expected",
    "observe_expected",
    "observe_expected_durable",
    "observe_merge_preservation_bundle",
    "observe_merge_preservation_git_directory",
    "observe_merge_preservation_workspace",
    "observe_merge_root_artifact",
    "prepare_merge_store_parents",
    "preservation_bundle",
    "preservation_git_directory",
    "preservation_workspace",
    "remove_merge_root_artifact",
    "replace_expected",
    "replace_merge_preservation_bundle",
    "replace_merge_preservation_workspace",
    "replace_merge_root_artifact",
    "require_canonical_bundle_parent",
    "root_artifact",
}

ENTRY_USES = {
    "crate::model::{ErrorCode, ModelError, ModelResult}",
    "std::path::Path",
    # E4.1's three: the lease the door takes, the subsystem error it renders,
    # and the sealed catalog entry point it calls.
    "super::bootstrap::CatalogMutationLeaseV1",
    "super::capability::CheckedFsError",
    "super::catalog::recover_or_create",
    # E4.2's one: the coordinator's two merge-start bootstrap sessions.
    "super::coordinator::execution::{ admit_merge_start_managed_parents, execute_merge_start_managed_parents, }",
    "super::{ CheckedArtifact, CheckedArtifactFact, CheckedArtifactPolicy, CheckedArtifactTransition, }",
}

ENTRY_CALLS = {
    "Bytes",
    "CheckedArtifact::acquire",
    "CheckedArtifact::prepare_parent",
    "CheckedArtifactFact::Bytes",
    "CheckedArtifactPolicy::git_directory",
    "CheckedArtifactPolicy::workspace",
    "Err",
    "MergeArtifactFact::Bytes",
    "ModelError::new",
    "Ok",
    "Path::new",
    "Some",
    # E4.2's three: row `:273`'s two sessions and the shared named renderer.
    "admit_merge_start_managed_parents",
    "execute_merge_start_managed_parents",
    "render_catalog_refusal",
    "classify_expected",
    "classify_remove",
    "classify_replace",
    "display",
    "fact",
    "format!",
    "is_some",
    # E4.1's first two: the combinators the activation door's error rendering
    # uses.
    "map",
    "map_err",
    "map_fact",
    "map_or",
    "map_transition",
    "match",
    "matches_expected",
    "observe",
    "observe_durable",
    "observe_expected",
    "observe_expected_durable",
    "parent_is_canonical",
    "preservation_bundle",
    "preservation_git_directory",
    "preservation_workspace",
    # E4.1's fourth: the sealed catalog entry point the activation door calls.
    "recover_or_create",
    # E4.1's third: the capability's actionable-remedy lookup.
    "remedy",
    "remove_exact",
    "replace_exact",
    "replace_expected",
    "require_canonical_bundle_parent",
    "root_artifact",
    "to_vec",
}

CHECKED_LEAF_ADAPTER_CALLS = {
    "workspace_ops/merge/root/artifact_facts.rs": {
        "Bytes",
        "MergeArtifactFact::Bytes",
        "Ok",
        "Path::new",
        "RegularFileFact::Bytes",
        "crate::checked_artifact::entry::classify_remove_merge_root_artifact",
        "crate::checked_artifact::entry::classify_replace_merge_root_artifact",
        "crate::checked_artifact::entry::observe_merge_root_artifact",
        "crate::checked_artifact::entry::remove_merge_root_artifact",
        "crate::checked_artifact::entry::replace_merge_root_artifact",
        "map_transition",
    },
    "git/gitbackend/preservation_root/files.rs": {
        "Component::Normal",
        "Err",
        "MetadataExt::dev",
        "MetadataExt::ino",
        "ModelError::new",
        "Ok",
        "Path::new",
        "PathBuf::new",
        "Some",
        "String::from_utf8",
        "as_bytes",
        "as_os_str",
        "as_ref",
        "as_slice",
        "components",
        "crate::checked_artifact::entry::classify_merge_preservation_workspace",
        "crate::checked_artifact::entry::observe_merge_preservation_git_directory",
        "crate::checked_artifact::entry::observe_merge_preservation_workspace",
        "crate::checked_artifact::entry::replace_merge_preservation_workspace",
        "evidence_error",
        "into",
        "is_absolute",
        "map",
        "map_err",
        "ok_or_else",
        "git2::Repository::open",
        "path",
        "pop",
        "push",
        "std::ffi::OsString::from_vec",
        "to_owned",
        "to_str",
        "to_vec",
    },
    "workspace_ops/merge/preserve/checked_bundle.rs": {
        "Err",
        "ModelError::new",
        "Ok",
        "PathBuf::from",
        "Some",
        "Vec::new",
        "as_deref",
        "as_ref",
        "as_slice",
        "as_str",
        "attach_owner",
        "bundle_relative",
        "clone",
        "cmp",
        "crate::checked_artifact::entry::classify_merge_preservation_bundle",
        "crate::checked_artifact::entry::observe_merge_preservation_bundle",
        "crate::checked_artifact::entry::replace_merge_preservation_bundle",
        "crate::git::GitPreservationDirtySummary::default",
        "crate::git::observe_preservation_stashes_read_only",
        "expected_bundle",
        "format!",
        "get",
        "into",
        "into_bytes",
        "is_empty",
        "is_none",
        "iter",
        "join",
        "map",
        "map_err",
        "ok_or_else",
        "owner_error",
        "owner_evidence",
        "owner_id",
        "owner_index",
        "owner_parts_error",
        "position",
        "push",
        "sort",
        "sort_by",
        "then",
        "then_some",
        "to_yaml",
        "transpose",
        "with_member",
    },
}

CHECKED_LEAF_ADAPTER_ITEMS = {
    "workspace_ops/merge/root/artifact_facts.rs": {
        "RegularFileFact",
        "RegularFileTransition",
        "classify_remove",
        "classify_write",
        "observe",
        "remove_exact",
        "write_checked",
    },
    "git/gitbackend/preservation_root/files.rs": {
        "identity",
        "observe_boundary",
        "observe_relative",
        "observe_required",
        "observe_transition",
        "path_to_raw",
        "raw_path_to_path",
        "replace_relative",
        "split_relative",
    },
    "workspace_ops/merge/preserve/checked_bundle.rs": {
        "V1BundleObservation",
        "v1_bundle_cursor_is_exact",
        "v1_bundle_observation",
        "v1_write_bundle_checked",
    },
}

CHECKED_LEAF_ADAPTER_USES = {
    "workspace_ops/merge/root/artifact_facts.rs": {
        "crate::checked_artifact::entry::{MergeArtifactFact, MergeArtifactTransition}",
        "crate::model::ModelResult",
        "std::path::Path",
    },
    "git/gitbackend/preservation_root/files.rs": {
        "cap_fs_ext::MetadataExt",
        "crate::checked_artifact::entry::MergeArtifactTransition",
        "std::os::unix::ffi::OsStrExt",
        "std::os::unix::ffi::OsStringExt",
        "std::path::{Component, Path, PathBuf}",
        "super::super::*",
    },
    "workspace_ops/merge/preserve/checked_bundle.rs": {
        "crate::checked_artifact::entry::MergeArtifactTransition",
        "crate::model::{ErrorCode, ModelError, ModelResult}",
        "crate::stash::{ STASH_BUNDLE_SCHEMA, StashBundle, StashBundleMember, StashDirtySummary, StashParticipation, StashPushLifecycle, StashRestoreState, }",
        "std::path::{Path, PathBuf}",
        "super::plan::V1PreservationOwnerPlan",
        "super::super::model::v1::PreservationOwnerV1",
    },
}

VISIBLE_ITEM = re.compile(
    r"\bpub\(crate\)\s+(?:unsafe\s+)?(?:async\s+)?"
    r"(fn|enum|struct|type|trait|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
ANY_VISIBLE_ITEM = re.compile(
    r"\bpub(?:\([^)]*\))?\s+(?:unsafe\s+)?(?:async\s+)?"
    r"(fn|enum|struct|type|trait|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
VISIBLE_REEXPORT = re.compile(r"\bpub(?:\([^)]*\))?\s+use\b")
USE = re.compile(r"\buse\s+([^;]+);")
CALL = re.compile(
    r"(?<![A-Za-z0-9_:])"
    r"([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)"
    r"\s*(!?)\s*\("
)
IGNORED_CALLS = {"cfg", "deny", "derive", "fn", "forbid", "not", "pub"}
PATH_ATTRIBUTE_START = re.compile(r"#\s*\[\s*path\b")
PATH_ATTRIBUTE_LITERAL = re.compile(
    r'#\s*\[\s*path\s*=\s*"([^"\r\n]+)"\s*\]'
)
INCLUDE_SOURCE_LOADER = re.compile(
    r"\binclude\s*!|\b(?:std|core)\s*::\s*include\b"
)
PRIVATE_CAPABILITIES = {
    "CheckedArtifact",
    "CheckedArtifactFact",
    "CheckedArtifactPolicy",
    "CheckedArtifactTransition",
}

CATALOG_LEASE_REFERENCE_SETS = {
    "CatalogOwnerEdgeV1": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_create_and_retry": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_publish_active": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_prepare_or_rewrite_staging": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_publish_final": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_retire_active": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_complete": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "execute_owner_scratch": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "create_git_private_parent": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/mutation.rs",
    },
    "write_or_rewrite_scratch": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/mutation.rs",
    },
    "publish_active_record": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/mutation.rs",
    },
    "prepare_or_rewrite_staging": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/directory_mutation.rs",
    },
    "publish_final_directory": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/directory_mutation.rs",
    },
    "retire_active_record": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/directory_mutation.rs",
    },
    "retain_completed_catalog": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/completed.rs",
    },
    "CompletedCatalogPermitV1": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "RetainedCompletedCatalogV1": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/completed.rs",
    },
    "owner_issue_for_catalog": {
        "checked_artifact/capability/pre_catalog/provider/interior.rs",
        "checked_artifact/protocol/infrastructure_record.rs",
    },
    "CatalogLeaseSetV1": {
        "checked_artifact/bootstrap.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease.rs",
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/preflight.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
    },
    "CatalogLeaseTargetRequestV1": {
        "checked_artifact/bootstrap.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease/target.rs",
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/preflight.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
    },
    "CatalogLeaseTargetBatchV1": {
        "checked_artifact/bootstrap.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease.rs",
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/preflight.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
    },
    "CatalogMutationLeaseV1": {
        "checked_artifact/bootstrap.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease/witness.rs",
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/catalog/bootstrap.rs",
        "checked_artifact/mod.rs",
        "operation/workspace_mutator_lock.rs",
    },
    "CatalogLeaseTargetWitnessV1": {
        "checked_artifact/bootstrap.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease.rs",
        "checked_artifact/bootstrap/runtime/catalog_lease/witness.rs",
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/filesystem.rs",
        "checked_artifact/capability/pre_catalog/provider/filesystem/bound.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "begin_preflight": {
        "checked_artifact/bootstrap/runtime/catalog_lease.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/grammar.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/preflight.rs",
        "checked_artifact/capability/pre_catalog/provider/mutation_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
        "checked_artifact/catalog/bootstrap.rs",
    },
    "inspect_bound_catalog_target": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/filesystem.rs",
        "checked_artifact/capability/pre_catalog/provider/filesystem/bound.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
    },
    "revalidate_lease_root_binding": {
        "checked_artifact/capability/pre_catalog.rs",
        "checked_artifact/capability/pre_catalog/provider.rs",
        "checked_artifact/capability/pre_catalog/provider/filesystem.rs",
        "checked_artifact/capability/pre_catalog/provider/filesystem/bound.rs",
    },
    # R2-E E4.1: the accessor's production callers outside the lock's own file
    # -- the forward v1 prologue and the A1 adapter's viability window.
    "catalog_mutation_lease": {
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/grammar.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/preflight.rs",
        "checked_artifact/capability/pre_catalog/provider/directory_mutation_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/mutation_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
        "operation/workspace_mutator_lock.rs",
        "workspace_ops/merge/runtime/dispatch.rs",
        "workspace_ops/merge/v1_lifecycle/checked.rs",
    },
}

FORBIDDEN_PROVISIONAL_CATALOG_INTERFACES = {
    "CatalogBootstrapV1",
    "PreCatalogOwnerV1",
    "PreCatalogPermitV1",
    "RevalidatedPreCatalogPermitV1",
    "lease_binding",
    "recover_or_create_git_directory",
    "recover_or_create_workspace",
}

# R2-D Phase 1 Step 1.2 extends this inventory deliberately, as
# GwzM5-8R2D-Plan.md §4 Step 1.2 and GwzM5-8R2DInterfaceFreeze.md §4.4 Class 1
# require ("The caller count stays at six production sites until a phase
# deliberately extends it"). Admission edges E4 (scratch -> active admission
# record) and E3 (staging -> deterministic final action name) add one more
# production caller file, holding both admission publications.
CATALOG_PUBLICATION_CALL_COUNTS = {
    "checked_artifact/capability/pre_catalog/provider/mutation.rs": 1,
    "checked_artifact/capability/pre_catalog/provider/directory_mutation.rs": 5,
    # R2-E Step E3.2 extends this inventory deliberately (freeze §4.4 Class 1;
    # GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md §6.2(a)): the terminal
    # retirement of edge E7's Phase-4 half renames the admitted action
    # directory out of the catalog root and into the catalog's own retired
    # root. That is cross-parent, and the namespace backend's shared
    # `execute_edge` call site is same-directory only, so the retirement needs
    # its own `publish_verified_no_replace` call here: 2 -> 3.
    #
    # This is the FIFTH dated extension of this dict and the FOURTEENTH
    # production call site, checkable rather than inherited: the freeze's base
    # is six sites (mutation.rs 1 + directory_mutation.rs 5); the four dated
    # extensions above added seven more, for thirteen; this adds the
    # fourteenth. The freeze's rule — "The caller count stays at six
    # production sites until a phase deliberately extends it" — is honoured by
    # recording the extension here, with its edge, rather than by bumping a
    # number.
    "checked_artifact/capability/pre_catalog/provider/admission_mutation.rs": 3,
    # R2-D Step 2.2 extends this inventory deliberately (freeze §4.4 rules;
    # Step-2.2 review [P2-1] discharge): the namespace backend's E12/E13
    # edges publish and retire through one shared sealed-primitive call site.
    "checked_artifact/capability/pre_catalog/provider/namespace_mutation.rs": 1,
    # R2-D Step 2.3 extends this inventory deliberately (freeze §4.4 Class 1):
    # edge E15 publishes the staged component inside the managed parent and
    # edge E16 retires the ownership marker out of the installed component
    # into the action directory's scheduled row — two no-replace moves through
    # the one primitive.
    "checked_artifact/capability/pre_catalog/provider/managed_mutation.rs": 2,
    # R2-D Step 2.4 extends this inventory deliberately (freeze §4.4 rules):
    # the authority record's own durable lifecycle publishes it onto the active
    # slot and retires it onto the scheduled retired alias, each through the
    # sealed no-replace primitive. Both are protocol-record moves; the streamed
    # source/goal payloads cross no namespace edge at all.
    "checked_artifact/capability/pre_catalog/provider/authority_record_binding.rs": 2,
    # R2-E Phase E2 does NOT extend this inventory, and banks the negative here
    # because the set-equality check below would otherwise be the first thing to
    # say so (GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT §6.2(a), §6.3's amended
    # §3.6 duty list). DECISION B-1 mints a new file under `provider/` —
    # `barrier_mutation.rs` — so a reader expects a new row. There is none: the
    # roaming anchor's alias is created through the P2 family (write-through
    # plus flush), and both its retirements route through
    # `RetainedActionNamespaceV1::execute_edge`, a same-directory rename that
    # passes `&self.handle` for both source and destination and is already
    # counted once against `namespace_mutation.rs`. `barrier_mutation.rs` opens
    # no sealed primitive of its own.
    #
    # This holds only while OPEN-B2's answer holds. E2.1 answered it "the target
    # parent stays action-directory-pinned"; if a later step widens the target
    # to a retained managed parent, those retirements become cross-parent
    # renames, `execute_edge` no longer serves them, and this dict moves with a
    # new `barrier_mutation.rs` row.
}


def production_rust_files(source: Path) -> list[Path]:
    return sorted(
        path
        for path in source.rglob("*.rs")
        if "tests" not in path.parts
        and "interface_tests" not in path.parts
        and not path.name.startswith("tests")
    )


TEST_MODULE = re.compile(
    r"(?m)^#\[cfg\(test\)\]\s*\n(?:#\[[^\n]*\]\s*\n)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{"
)


def without_test_modules(masked: str) -> str:
    """Drop each `#[cfg(test)] mod ... { ... }` body from ALREADY-MASKED source.

    Brace counting is exact only after `mask_non_code`, which blanks string and
    comment contents, so no brace inside either can unbalance the walk.
    """
    kept, index = [], 0
    for match in TEST_MODULE.finditer(masked):
        if match.start() < index:
            continue
        kept.append(masked[index : match.start()])
        depth, cursor = 1, match.end()
        while depth and cursor < len(masked):
            depth += (masked[cursor] == "{") - (masked[cursor] == "}")
            cursor += 1
        index = cursor
    kept.append(masked[index:])
    return "".join(kept)


def mask_non_code(text: str) -> str:
    """Replace comments and string/character contents while retaining newlines."""
    output = list(text)
    index = 0
    length = len(text)

    def blank(start: int, end: int) -> None:
        for offset in range(start, end):
            if output[offset] != "\n":
                output[offset] = " "

    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = length if end < 0 else end
            blank(index, end)
            index = end
        elif text.startswith("/*", index):
            depth = 1
            end = index + 2
            while end < length and depth:
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            blank(index, end)
            index = end
        elif text[index] == "r":
            match = re.match(r'r(#+)?"', text[index:])
            if not match:
                index += 1
                continue
            hashes = match.group(1) or ""
            close = '"' + hashes
            end = text.find(close, index + len(match.group(0)))
            end = length if end < 0 else end + len(close)
            blank(index, end)
            index = end
        elif text[index] == '"':
            end = index + 1
            while end < length:
                if text[end] == "\\":
                    end += 2
                elif text[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            blank(index, min(end, length))
            index = end
        elif text[index] == "'" and index + 2 < length:
            # A Rust lifetime is followed by an identifier and no closing quote;
            # only mask a syntactic character literal.
            end = index + 1
            if text[end] == "\\":
                end += 2
            else:
                end += 1
            if end < length and text[end] == "'":
                end += 1
                blank(index, end)
                index = end
            else:
                index += 1
        else:
            index += 1
    return "".join(output)


def source_tree_digest(source: Path, root_relative: str) -> str:
    root_file = source / root_relative
    descendant_root = (
        root_file.parent if root_file.name == "mod.rs" else root_file.with_suffix("")
    )
    paths = {root_file}
    if descendant_root.is_dir():
        paths.update(path for path in descendant_root.rglob("*") if path.is_file())
    digest = hashlib.sha256()
    for path in sorted(paths, key=lambda value: value.relative_to(source).as_posix()):
        relative = path.relative_to(source).as_posix().encode("utf-8")
        content = path.read_bytes()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    return digest.hexdigest()


def calls(text: str) -> set[str]:
    result = set()
    for match in CALL.finditer(text):
        name, bang = match.groups()
        prefix = text[max(0, match.start() - 3) : match.start()]
        if prefix == "fn " or name in IGNORED_CALLS:
            continue
        result.add(name + ("!" if bang else ""))
    return result


def imports(text: str) -> set[str]:
    return {re.sub(r"\s+", " ", value).strip() for value in USE.findall(text)}


def v0_persistence_seam(source: Path) -> set[str]:
    """Names `workspace_ops::merge` re-exports out of the v0 record store."""
    text = mask_non_code(
        (source / "workspace_ops/merge/mod.rs").read_text(encoding="utf-8")
    )
    names: set[str] = set()
    for match in V0_STORE_REEXPORT.finditer(text):
        for item in match.group(1).split(","):
            name = item.strip().split()[0].strip() if item.strip() else ""
            if name:
                names.add(name)
    return names


def check(source: Path) -> list[str]:
    findings: list[str] = []
    crate_root = source.parent.resolve()
    manifest_path = crate_root / "Cargo.toml"
    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        manifest = None
    if (
        manifest_path.is_symlink()
        or not isinstance(manifest, dict)
        or manifest.get("lib") != {"path": "src/lib.rs"}
    ):
        findings.append("compiler root manifest changed: Cargo.toml [lib]")
    for relative, expected_digest in sorted(PROTECTED_COMPILER_ROOT_DIGESTS.items()):
        path = crate_root / relative
        if (
            path.is_symlink()
            or not path.is_file()
            or hashlib.sha256(path.read_bytes()).hexdigest() != expected_digest
        ):
            findings.append(f"compiler root manifest changed: {relative}")
    forbid = "#![forbid(clippy::disallowed_methods)]"
    for relative, expected_digest in sorted(PROTECTED_SOURCE_DIGESTS.items()):
        path = source / relative
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != expected_digest:
            findings.append(f"protected source allowlist changed: {relative}")
    for relative, expected_digest in sorted(PROTECTED_SOURCE_TREE_DIGESTS.items()):
        if source_tree_digest(source, relative) != expected_digest:
            findings.append(f"protected source tree changed: {relative}")
    publication_relative = (
        "checked_artifact/capability/pre_catalog/provider/publication.rs"
    )
    publication = mask_non_code(
        (source / publication_relative).read_text(encoding="utf-8")
    )
    publication_callers = {
        path.relative_to(source).as_posix()
        for path in production_rust_files(
            source / "checked_artifact/capability/pre_catalog/provider"
        )
        if "publish_verified_no_replace"
        in calls(mask_non_code(path.read_text(encoding="utf-8")))
    }
    publication_shape_is_exact = (
        len(re.findall(r"\bfn\s+publish_verified_no_replace\s*\(", publication)) == 1
        and len(
            re.findall(
                r"\bcrate::checked_artifact::platform::open_rename_source\s*\(",
                publication,
            )
        )
        == 1
        and len(
            re.findall(
                r"\bcrate::checked_artifact::platform::rename_open_source\s*\(",
                publication,
            )
        )
        == 1
        and "rename_relative" not in publication
    )
    for relative, expected_count in CATALOG_PUBLICATION_CALL_COUNTS.items():
        text = mask_non_code((source / relative).read_text(encoding="utf-8"))
        actual_count = len(
            re.findall(r"(?m)^\s*publish_verified_no_replace\s*\(", text)
        )
        if actual_count != expected_count or "rename_relative" in text:
            publication_shape_is_exact = False
    if (
        not publication_shape_is_exact
        or publication_callers != set(CATALOG_PUBLICATION_CALL_COUNTS)
    ):
        findings.append(
            "catalog publication seam changed: all six physical moves must use "
            "the single source-associated publication primitive"
        )
    for path in production_rust_files(source / "checked_artifact"):
        relative = path.relative_to(source).as_posix()
        text = mask_non_code(path.read_text(encoding="utf-8"))
        expected_counts = RAW_RENAME_CALL_ALLOWLIST.get(relative, {})
        for token in RAW_RENAME_TOKENS:
            actual = len(
                [
                    match
                    for match in re.finditer(r"\b" + token + r"\b", text)
                    if text[max(0, match.start() - 3) : match.start()] != "fn "
                ]
            )
            if actual != expected_counts.get(token, 0):
                findings.append(
                    "raw rename caller outside the sealed publication seam: "
                    f"{relative} ({token})"
                )
    seam = v0_persistence_seam(source)
    missing_seam = V0_PERSISTENCE_SEAM_FLOOR - seam
    if missing_seam:
        findings.append(
            "v0 persistence seam inventory is underivable: "
            f"workspace_ops/merge/mod.rs no longer re-exports {sorted(missing_seam)}"
        )
    for path in sorted((source / V1_LIFECYCLE_TREE).rglob("*.rs")):
        relative = path.relative_to(source).as_posix()
        text = mask_non_code(path.read_text(encoding="utf-8"))
        for token in sorted(seam | V0_PERSISTENCE_SEAM_FLOOR):
            if any(
                text[max(0, match.start() - 3) : match.start()] != "fn "
                for match in re.finditer(r"\b" + re.escape(token) + r"\b", text)
            ):
                findings.append(
                    "v1 lifecycle names the v0 persistence seam: "
                    f"{relative} ({token})"
                )
    raw_writer_files: dict[str, dict[str, int]] = {}
    for path in production_rust_files(source / V1_LIFECYCLE_TREE):
        relative = path.relative_to(source).as_posix()
        text = mask_non_code(path.read_text(encoding="utf-8"))
        if re.search(r"\bdurable_fs\b", text):
            raw_writer_files[relative] = {
                writer: count
                for writer in V1_LIFECYCLE_RAW_DURABLE_WRITERS
                if (count := len(re.findall(r"\b" + writer + r"\b", text)))
            }
    expected_files = set(V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES)
    carved = V1_LIFECYCLE_PERMANENT_WRITER_EXCEPTIONS

    def carved_finding(relative: str, moved: str) -> str:
        return (
            f"permanent writer exception ({carved[relative]}), {relative}: {moved}. "
            "That path is PERMANENT-DOCUMENTED: revise its amendment first -- the "
            "conversion is re-decided there (each row's amendment names its own re-decision point), "
            "not by a commit that moves this pin"
        )

    for relative in sorted(set(raw_writer_files) - expected_files):
        findings.append(
            # Unreachable while the module-scope guard above stands (an exception key
            # must sit on a pinned row); kept as the second belt if that guard goes.
            carved_finding(relative, "its pin row was DELETED while the file still names durable_fs")
            if relative in carved
            else "v1 lifecycle gained a raw durable_fs writer outside the O13 "
            f"accepted residual: {relative}"
        )
    for relative in sorted(expected_files - set(raw_writer_files)):
        findings.append(
            carved_finding(relative, "it no longer names durable_fs AT ALL -- a conversion of it may not land")
            if relative in carved
            else "O13 accepted-residual entry no longer names durable_fs and must "
            f"be retired from the pin deliberately: {relative}"
        )
    for relative in sorted(expected_files & set(raw_writer_files)):
        counts = V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES[relative]
        actual = raw_writer_files[relative]
        if actual == counts:
            continue
        moved = f"expected={counts} actual={actual}"
        if relative in carved:
            shrank = any(actual.get(w, 0) < c for w, c in counts.items())
            findings.append(carved_finding(relative, (
                f"the raw-writer count SHRANK ({moved}) -- a PARTIAL conversion may not land"
                if shrank
                else f"the raw-writer count GREW ({moved}) -- the exception blesses "
                "the carved path's EXISTING publication primitive only, not a new raw writer"
            )))
        else:
            findings.append(
                "O13 raw-writer count moved and must move the pin with it: "
                f"{relative}: {moved}"
            )
    for relative, (row, counts) in sorted(CAPABILITY_FREE_RAW_WRITER_INVENTORY.items()):
        path = source / relative
        if not path.is_file():
            findings.append(
                f"capability-free carved file is GONE: {relative} ({row}); "
                f"{CAPABILITY_FREE_EXCEPTION} names it and must be revised first"
            )
            continue
        text = without_test_modules(mask_non_code(path.read_text(encoding="utf-8")))
        o13 = relative in V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES
        actual = {
            token: found
            for token in CAPABILITY_FREE_WRITER_TOKENS
            if not (o13 and token in V1_LIFECYCLE_RAW_DURABLE_WRITERS)
            and (found := len(re.findall(r"\b" + token + r"\b", text)))
        }
        if actual == counts:
            continue
        shrank = any(actual.get(token, 0) < count for token, count in counts.items())
        findings.append(
            f"capability-free raw writer inventory moved, {relative} ({row}): "
            f"expected={counts} actual={actual}. "
            + (
                "a PARTIAL CONVERSION of a carved arm may not land without revising "
                "GwzM5-8R2E-CapabilityFreeAmendment.md §3 -- these writers are a DATED "
                "EXCEPTION, not unfinished work, and E4.7 does not retire them"
                if shrank
                else "A NEW RAW WRITER IS NOT BLESSED: the exception covers the arms it "
                "enumerates and nothing else, and converting this one instead would put "
                "a capability-free operation on the durable-identity probe"
            )
        )
    for relative in sorted(PROTECTED_COMPILER_MODULES):
        raw = (source / relative).read_bytes()
        if forbid not in mask_non_code(raw.decode("utf-8")):
            findings.append(
                f"compiler-resolved writer boundary is not fail-closed: {relative}"
            )
    path_edges = set()
    malformed_path_edges = []
    include_sources = []
    invalid_path_targets = []
    for path in sorted(source.rglob("*.rs")):
        raw = path.read_text(encoding="utf-8")
        masked = mask_non_code(raw)
        relative = path.relative_to(source).as_posix()
        if INCLUDE_SOURCE_LOADER.search(masked):
            include_sources.append(relative)
        for start in PATH_ATTRIBUTE_START.finditer(masked):
            literal = PATH_ATTRIBUTE_LITERAL.match(raw, start.start())
            if literal is None:
                malformed_path_edges.append(relative)
                continue
            target = literal.group(1)
            path_edges.add((relative, target))
            resolved = (path.parent / target).resolve()
            try:
                resolved.relative_to(crate_root)
            except ValueError:
                invalid_path_targets.append((relative, target, "outside crate"))
                continue
            if resolved.suffix != ".rs" or not resolved.is_file():
                invalid_path_targets.append((relative, target, "not a regular .rs file"))
    if (
        path_edges != APPROVED_RUST_PATH_EDGES
        or malformed_path_edges
        or include_sources
        or invalid_path_targets
    ):
        findings.append(
            "Rust source-loading edge inventory changed: "
            f"expected={sorted(APPROVED_RUST_PATH_EDGES)} actual={sorted(path_edges)} "
            f"malformed={sorted(malformed_path_edges)} "
            f"include={sorted(include_sources)} "
            f"invalid_targets={sorted(invalid_path_targets)}"
        )
    backend = mask_non_code(
        (source / "git/gitbackend.rs").read_text(encoding="utf-8")
    )
    expected_concrete_observer = re.compile(
        r"pub\(crate\)\s+fn\s+observe_preservation_stashes_read_only\b[\s\S]*?"
        r"\{\s*preservation_image::preservation_stashes\(path,\s*merge_id\)\s*\}"
    )
    if expected_concrete_observer.search(backend) is None:
        findings.append(
            "production preservation observer no longer terminates in its protected leaf"
        )
    contract = mask_non_code(
        (source / "git/gitbackend/contract.rs").read_text(encoding="utf-8")
    )
    if re.search(r"\bfn\s+preservation_stashes\s*\(", contract):
        findings.append(
            "open GitBackend preservation observer was reintroduced into the trait contract"
        )
    open_merge_observer_calls = []
    concrete_observer_references = []
    for path in production_rust_files(source / "workspace_ops/merge"):
        text = mask_non_code(path.read_text(encoding="utf-8"))
        relative = path.relative_to(source).as_posix()
        if re.search(r"\bpreservation_stashes\b", text):
            open_merge_observer_calls.append(relative)
    if open_merge_observer_calls:
        findings.append(
            "authority-sensitive merge code reintroduced the open GitBackend "
            f"preservation observer: {open_merge_observer_calls}"
        )
    for path in production_rust_files(source):
        text = mask_non_code(path.read_text(encoding="utf-8"))
        if re.search(r"\bobserve_preservation_stashes_read_only\b", text):
            concrete_observer_references.append(path.relative_to(source).as_posix())
    if set(concrete_observer_references) != CONCRETE_PRESERVATION_OBSERVER_REFERENCES:
        findings.append(
            "concrete preservation observer caller set changed: "
            f"expected={sorted(CONCRETE_PRESERVATION_OBSERVER_REFERENCES)} "
            f"actual={sorted(concrete_observer_references)}"
        )
    entry_path = source / "checked_artifact/entry.rs"
    entry_text = mask_non_code(entry_path.read_text(encoding="utf-8"))
    definitions = {name for _, name in VISIBLE_ITEM.findall(entry_text)}
    expected = set(ENTRY_REFERENCES)
    if definitions != expected or VISIBLE_REEXPORT.search(entry_text):
        findings.append(
            "checked entry visible-item inventory changed: "
            f"expected={sorted(expected)} actual={sorted(definitions)}"
        )
    all_entry_items = {name for _, name in ANY_VISIBLE_ITEM.findall(entry_text)} | {
        name
        for name in re.findall(
            r"(?m)^\s*(?:fn|enum|struct|type|trait|const|static|mod)\s+"
            r"([A-Za-z_][A-Za-z0-9_]*)",
            entry_text,
        )
    }
    if all_entry_items != ENTRY_ITEMS:
        findings.append(
            "checked entry complete item inventory changed: "
            f"expected={sorted(ENTRY_ITEMS)} actual={sorted(all_entry_items)}"
        )
    entry_uses = imports(entry_text)
    if entry_uses != ENTRY_USES:
        findings.append(
            "checked entry import inventory changed: "
            f"expected={sorted(ENTRY_USES)} actual={sorted(entry_uses)}"
        )
    entry_calls = calls(entry_text)
    if entry_calls != ENTRY_CALLS:
        findings.append(
            "checked entry call graph changed: "
            f"expected={sorted(ENTRY_CALLS)} actual={sorted(entry_calls)}"
        )

    actual_references: dict[str, set[str]] = {name: set() for name in expected}
    entry_path_users: set[str] = set()
    escaped_capabilities: dict[str, set[str]] = {}
    masked_sources: dict[str, str] = {}
    for path in production_rust_files(source):
        relative = path.relative_to(source).as_posix()
        # KNOWN SCAN HOLE, recorded 2026-09-01 (E4.1 review [P3-4], carried to
        # E4.2). `masked_sources` is what `CATALOG_LEASE_REFERENCE_SETS` and
        # `FORBIDDEN_PROVISIONAL_CATALOG_INTERFACES` scan, so both are BLIND at
        # `entry.rs`: E4.1's `use super::bootstrap::CatalogMutationLeaseV1`
        # moved no lease-reference row even though that name IS a key in the
        # set, and a reintroduced provisional spelling here would not fire
        # either. The skip exists so the four `ENTRY_*` equality inventories
        # above do not also count entry.rs as a consumer of itself. A RECORD,
        # not a repair: the hole stays P3 because entry.rs is byte-pinned in
        # `PROTECTED_SOURCE_DIGESTS`, listed in `PROTECTED_COMPILER_MODULES`,
        # and equality-checked four ways -- no edit passes unreviewed.
        if relative == "checked_artifact/entry.rs":
            continue
        text = mask_non_code(path.read_text(encoding="utf-8"))
        masked_sources[relative] = text
        if re.search(r"\bchecked_artifact\s*::\s*entry\b", text):
            entry_path_users.add(relative)
        for symbol in expected:
            if re.search(rf"\b{re.escape(symbol)}\b", text):
                actual_references[symbol].add(relative)
        if not relative.startswith("checked_artifact/"):
            for capability in PRIVATE_CAPABILITIES:
                if re.search(rf"\b{capability}\b", text):
                    escaped_capabilities.setdefault(capability, set()).add(relative)

    for symbol in sorted(expected):
        actual = actual_references[symbol]
        allowed = ENTRY_REFERENCES[symbol]
        if actual != allowed:
            findings.append(
                f"checked entry reference set changed: {symbol}: "
                f"expected={sorted(allowed)} actual={sorted(actual)}"
            )

    allowed_entry_users = set().union(*ENTRY_REFERENCES.values())
    if entry_path_users != allowed_entry_users:
        findings.append(
            "checked entry module user set changed: "
            f"expected={sorted(allowed_entry_users)} actual={sorted(entry_path_users)}"
        )
    if escaped_capabilities:
        findings.append(
            "general checked capability escaped its private module: "
            + ", ".join(
                f"{name}={sorted(paths)}"
                for name, paths in sorted(escaped_capabilities.items())
            )
        )

    for symbol, allowed in sorted(CATALOG_LEASE_REFERENCE_SETS.items()):
        actual = {
            relative
            for relative, text in masked_sources.items()
            if re.search(rf"\b{re.escape(symbol)}\b", text)
        }
        if actual != allowed:
            findings.append(
                f"catalog lease reference set changed: {symbol}: "
                f"expected={sorted(allowed)} actual={sorted(actual)}"
            )
    for symbol in sorted(FORBIDDEN_PROVISIONAL_CATALOG_INTERFACES):
        actual = sorted(
            relative
            for relative, text in masked_sources.items()
            if re.search(rf"\b{re.escape(symbol)}\b", text)
        )
        if actual:
            findings.append(
                f"provisional catalog interface was reintroduced: {symbol}: {actual}"
            )
    catalog_target = masked_sources[
        "checked_artifact/bootstrap/runtime/catalog_lease/target.rs"
    ]
    if re.search(r"\bfn\s+git_directory\s*\(", catalog_target) or not re.search(
        r"\bfn\s+repository_common_git_directory\s*\(", catalog_target
    ):
        findings.append(
            "catalog Git lease target must be derived from repository common-directory state"
        )
    catalog_lease = masked_sources[
        "checked_artifact/bootstrap/runtime/catalog_lease.rs"
    ]
    if re.search(r"\.sort_by\s*\(", catalog_lease) or len(
        re.findall(r"\.sort_unstable_by\s*\(", catalog_lease)
    ) != 2:
        findings.append(
            "catalog lease batch ordering must use exactly two allocation-free unstable sorts"
        )

    for relative, expected_calls in CHECKED_LEAF_ADAPTER_CALLS.items():
        adapter = masked_sources[relative]
        actual_calls = calls(adapter)
        if actual_calls != expected_calls:
            findings.append(
                f"checked adapter call graph changed: {relative}: "
                f"expected={sorted(expected_calls)} actual={sorted(actual_calls)}"
            )
        actual_items = {name for _, name in ANY_VISIBLE_ITEM.findall(adapter)}
        expected_items = CHECKED_LEAF_ADAPTER_ITEMS[relative]
        if actual_items != expected_items or VISIBLE_REEXPORT.search(adapter):
            findings.append(
                f"checked adapter visible-item inventory changed: {relative}: "
                f"expected={sorted(expected_items)} actual={sorted(actual_items)}"
            )
        actual_uses = imports(adapter)
        expected_uses = CHECKED_LEAF_ADAPTER_USES[relative]
        if actual_uses != expected_uses:
            findings.append(
                f"checked adapter import inventory changed: {relative}: "
                f"expected={sorted(expected_uses)} actual={sorted(actual_uses)}"
            )

    checked_mod = mask_non_code(
        (source / "checked_artifact/mod.rs").read_text(encoding="utf-8")
    )
    if "pub(crate) mod entry;" not in checked_mod:
        findings.append("checked entry module is not the exported architectural boundary")
    for declaration in (
        "struct CheckedArtifact",
        "enum CheckedArtifactFact",
        "enum CheckedArtifactTransition",
    ):
        if f"pub(crate) {declaration}" in checked_mod:
            findings.append(f"general capability is crate-visible: {declaration}")
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, default=ROOT / "src")
    args = parser.parse_args()
    findings = check(args.source.resolve())
    if findings:
        print("checked-artifact boundary: failed", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print(
        "checked-artifact boundary: ok "
        f"({len(ENTRY_REFERENCES)} visible entries, "
        f"{len(set().union(*ENTRY_REFERENCES.values()))} classified modules)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
