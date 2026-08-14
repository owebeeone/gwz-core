#!/usr/bin/env python3
"""Fail-closed structural inventory for the checked-artifact boundary."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

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
    "checked_artifact/entry.rs": "33f05b79dbbbc81cb995ba6d94ff0076731faf310f4cd8b1ade396aaca3b7228",
    "git/gitbackend/authority_backend.rs": "0abb856d03118b0d304170beab3fcd8e18e3ae4c3b7860f66771351849c14ff1",
    "git/gitbackend.rs": "b85dfd3f32671886a34d2bee5c79200dc6da74a9f99fd5cfa0fe1d801667b3fb",
    "git/gitbackend/preservation_root/files.rs": "7a6b72ac62a91a48992b04a563d85354dcef950aad420c610e7a08c3c2409b35",
    "git/gitbackend/preservation_image.rs": "1a96e1921052895c836837def6d7c3c19fb6bf383ad8df9482a5960c1b2cbdac",
    "workspace_ops/merge/preserve/artifacts.rs": "489006a9a7550503fd051b2d6136f8a08728ab9355e6281ede684047732f6da5",
    "workspace_ops/merge/preserve/checked_bundle.rs": "dbc3e4de328afefbedd3ee343c0bf384b2852d499e3f007960159ff229595251",
    "workspace_ops/merge/preserve/plan.rs": "880d4905eeab96ff52a746a360043628cd4c11e5324b5f92b5889419ade53c7a",
    "workspace_ops/merge/root/artifact_facts.rs": "d4bb3d895070c4bafbb6ee8fed2664768b6e4d6be43fe764f877add4f4c42f19",
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
    ("checked_artifact/mod.rs", "tests.rs"),
    ("checked_artifact/tests.rs", "tests/durability.rs"),
    ("checked_artifact/tests.rs", "tests/exact_source.rs"),
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
PROTECTED_SOURCE_TREE_DIGESTS = {
    "workspace_ops/merge/v1_lifecycle/authority/observe.rs": "ff6574fc1bde70c81dc72bd58373eaa50ef7d1b26fc6468412f9e041a1e90788",
    "workspace_ops/merge/v1_lifecycle/mod.rs": "21ab2154e58ebe091a62bfb5bdf36afdbe98c30b9181d0ba08e29415aa6af00d",
}

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


def check(source: Path) -> list[str]:
    findings: list[str] = []
    forbid = "#![forbid(clippy::disallowed_methods)]"
    for relative, expected_digest in sorted(PROTECTED_SOURCE_DIGESTS.items()):
        path = source / relative
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != expected_digest:
            findings.append(f"protected source allowlist changed: {relative}")
    for relative, expected_digest in sorted(PROTECTED_SOURCE_TREE_DIGESTS.items()):
        if source_tree_digest(source, relative) != expected_digest:
            findings.append(f"protected source tree changed: {relative}")
    for relative in sorted(PROTECTED_COMPILER_MODULES):
        raw = (source / relative).read_bytes()
        if forbid not in mask_non_code(raw.decode("utf-8")):
            findings.append(
                f"compiler-resolved writer boundary is not fail-closed: {relative}"
            )
    path_edges = set()
    malformed_path_edges = []
    include_sources = []
    crate_root = source.parent.resolve()
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
