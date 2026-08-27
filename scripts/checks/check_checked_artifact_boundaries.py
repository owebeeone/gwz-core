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
PROTECTED_SOURCE_DIGESTS = {
    "checked_artifact/bootstrap.rs": "d85d894032512125ee5ad0cab770db25dd37cee32096ad84be3061eaab94b2aa",
    "checked_artifact/bootstrap/runtime/mod.rs": "1bddf4b40e4bd6454300e7b08b54119875ec19daacb819a14dbd0c483784230d",
    "checked_artifact/capability.rs": "a1cdb1d5b2ff92507f6138322a51a0dec1d6b4cd788421b10f27736d47e7566c",
    "checked_artifact/entry.rs": "33f05b79dbbbc81cb995ba6d94ff0076731faf310f4cd8b1ade396aaca3b7228",
    "checked_artifact/authority.rs": "fd300c5b8fb9dfacd41a4f0c6c39923fc8decbb07a6933af2eaa471c4ebdf1ed",
    "checked_artifact/mod.rs": "48d9261bd994dcf81ca77d3b3195b2e2e1ac6f2231f8c506580ab79571057e7c",
    "checked_artifact/residue.rs": "717c2417cd3707a72c5d1c4aee4845bdab525a566c5ea9d99b74ea6850480150",
    "checked_artifact/transition.rs": "13b483bc0dc3099082727a5d499b97f627ba7d41a65b929ec557416ac59b37ca",
    "git/gitbackend/authority_backend.rs": "0abb856d03118b0d304170beab3fcd8e18e3ae4c3b7860f66771351849c14ff1",
    "git/gitbackend.rs": "b85dfd3f32671886a34d2bee5c79200dc6da74a9f99fd5cfa0fe1d801667b3fb",
    "git/gitbackend/preservation_root/files.rs": "7a6b72ac62a91a48992b04a563d85354dcef950aad420c610e7a08c3c2409b35",
    "git/gitbackend/preservation_image.rs": "b1f193279069fd84492c423ccead5ac051415550c0024b01d4c75f4e51c93981",
    "workspace_ops/merge/preserve/artifacts.rs": "a4e9f7055c3d8aeacd494c08629223dbb15fea0a2e271303b0c27f11833be696",
    "workspace_ops/merge/preserve/checked_bundle.rs": "dbc3e4de328afefbedd3ee343c0bf384b2852d499e3f007960159ff229595251",
    "workspace_ops/merge/preserve/plan.rs": "3730179e156151c4a853752ec769712d3ae81bd21e7729b892ab4cb14474ff89",
    "workspace_ops/merge/root/artifact_facts.rs": "d4bb3d895070c4bafbb6ee8fed2664768b6e4d6be43fe764f877add4f4c42f19",
    "operation/workspace_mutator_lock.rs": "0d9b034edab7e66a5e83b4bc86b7325afa001666220a07e428d4fa73f9384e28",
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
PROTECTED_SOURCE_TREE_DIGESTS = {
    "checked_artifact/bootstrap/runtime/catalog_lease.rs": "1a13b93320660755f4e53190288d08c1b6d92bc6ecb8ec7a1719de48123f1a0e",
    "checked_artifact/capability/path.rs": "23e46dbde50a0530c331c34dd68a9d40096394c6817075d3f66ad3f0e27a91c6",
    "checked_artifact/capability/pre_catalog.rs": "5c65d5a248ec5b03812db5cfe2874943a4d6b4f0df5f0ad01bfd26f4f38e2b24",
    "checked_artifact/catalog.rs": "cc845e20eae62f601e6c730799e77b69ba184324e7dfd58b4791dd139f133708",
    "checked_artifact/platform.rs": "2f938dd93af7f3f065ab1ce3e5105df1d0045d9ca3ea05067f993cfa7ae5ecaa",
    "workspace_ops/merge/v1_lifecycle/authority/observe.rs": "d16fa8bf67f8656c56b3c51d6625712efcc970dfd51afefa77557df5b3fcae38",
    "workspace_ops/merge/v1_lifecycle/mod.rs": "74a416a623ad421b1646e8692d6b449d76cc754045ff43348c5e897c39eae96c",
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
V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES = frozenset(
    {
        "workspace_ops/merge/v1_lifecycle/archive.rs",
        "workspace_ops/merge/v1_lifecycle/store/archive.rs",
        "workspace_ops/merge/v1_lifecycle/store/rewrite.rs",
    }
)

ENTRY_REFERENCES = {
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
    "classify_expected",
    "classify_remove",
    "classify_replace",
    "display",
    "fact",
    "format!",
    "is_some",
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
    "catalog_mutation_lease": {
        "checked_artifact/bootstrap/runtime/mod.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/grammar.rs",
        "checked_artifact/capability/pre_catalog/provider/catalog_tests/preflight.rs",
        "checked_artifact/capability/pre_catalog/provider/directory_mutation_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/mutation_tests.rs",
        "checked_artifact/capability/pre_catalog/provider/production_tests.rs",
        "operation/workspace_mutator_lock.rs",
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
    raw_writer_files = set()
    for path in production_rust_files(source / V1_LIFECYCLE_TREE):
        relative = path.relative_to(source).as_posix()
        text = mask_non_code(path.read_text(encoding="utf-8"))
        if re.search(r"\bdurable_fs\b", text):
            raw_writer_files.add(relative)
    for relative in sorted(raw_writer_files - V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES):
        findings.append(
            "v1 lifecycle gained a raw durable_fs writer outside the O13 "
            f"accepted residual: {relative}"
        )
    for relative in sorted(V1_LIFECYCLE_RAW_DURABLE_WRITER_FILES - raw_writer_files):
        findings.append(
            "O13 accepted-residual entry no longer names durable_fs and must "
            f"be retired from the pin deliberately: {relative}"
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
