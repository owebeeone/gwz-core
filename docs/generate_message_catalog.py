#!/usr/bin/env python3
"""Render the GWZ taut message catalog.

This is intentionally docs-local: it derives the mechanical catalog from
`protocol/gwz.taut.py` without touching generated Rust or corpus artifacts.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path


DOCS_DIR = Path(__file__).resolve().parent
CORE_ROOT = DOCS_DIR.parent
WORKSPACE_ROOT = CORE_ROOT.parent
SCHEMA_PATH = CORE_ROOT / "protocol" / "gwz.taut.py"
OUT_PATH = DOCS_DIR / "MessageCatalog.md"


REQUEST_MATRIX = {
    "CreateWorkspaceRequest": ("CreateWorkspaceResponse", "workspace_ops::handle_create_workspace", "init"),
    "InitFromSourcesRequest": ("InitFromSourcesResponse", "workspace_ops::handle_init_from_sources", "init"),
    "AddExistingRepoRequest": ("AddExistingRepoResponse", "workspace_ops::handle_add_existing_repo", "add"),
    "CreateRepoRequest": ("CreateRepoResponse", "workspace_ops::handle_create_repo", "repo/create"),
    "MaterializeRequest": ("MaterializeResponse", "workspace_ops::handle_materialize", "materialize/clone"),
    "StatusRequest": ("StatusResponse", "status::handle_status", "status"),
    "LsRequest": ("LsResponse", "workspace_ops::handle_ls", "ls"),
    "SnapshotRequest": ("SnapshotResponse", "workspace_ops::handle_snapshot", "snapshot"),
    "TagRequest": ("TagResponse", "workspace_ops::handle_tag", "tag"),
    "CaptureRequest": ("CaptureResponse", "workspace_ops::handle_capture", "capture"),
    "CommitRequest": ("CommitResponse", "workspace_ops::handle_commit", "commit"),
    "StageRequest": ("StageResponse", "workspace_ops::handle_stage", "stage"),
    "PullHeadRequest": ("PullHeadResponse", "workspace_ops::handle_pull_head", "pull"),
    "PullSnapshotRequest": ("PullSnapshotResponse", "workspace_ops::handle_pull_snapshot", "pull"),
    "PushRequest": ("PushResponse", "workspace_ops::handle_push", "push"),
    "ExecRequest": ("ExecResponse", "none", "forall"),
}

CLI_LOCAL = {"ExecMode", "ExecRequest", "ExecResponse", "ExecResult"}


def load_schema():
    taut_src = WORKSPACE_ROOT / "taut" / "src"
    if taut_src.is_dir():
        sys.path.insert(0, str(taut_src))
        # The vendored taut tree can be checked out at GWZ tag names that are
        # not Python package versions. This mirrors the inspection workaround in
        # protocol/regen.py without changing release regeneration.
        os.environ.setdefault("SETUPTOOLS_SCM_PRETEND_VERSION", "0.6.0")

    spec = importlib.util.spec_from_file_location("gwz_taut_schema", SCHEMA_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load schema at {SCHEMA_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.SCHEMA


def type_name(t) -> str:
    cls = type(t).__name__
    if cls == "Scalar":
        return t.kind
    if cls in {"EnumRef", "MsgRef"}:
        return t.name
    if cls == "ListOf":
        return f"List<{type_name(t.elem)}>"
    if cls == "MapOf":
        return f"Map<{type_name(t.key)}, {type_name(t.value)}>"
    return repr(t)


def table(headers: list[str], rows: list[list[object]]) -> list[str]:
    out = ["| " + " | ".join(headers) + " |", "| " + " | ".join("---" for _ in headers) + " |"]
    for row in rows:
        out.append("| " + " | ".join(escape(str(cell)) for cell in row) + " |")
    return out


def escape(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ")


def render_service(schema) -> list[str]:
    rows = []
    for service in schema.services.values():
        for method in service.methods:
            params = ", ".join(f"{name}: {type_name(t)}" for name, t in method.params) or "-"
            out = ", ".join(f"{slot}: {type_name(t)}" for slot, t in method.out) or "-"
            rows.append([service.name, method.name, method.role, method.shape, params, out])
    return table(["Service", "Method", "Role", "Shape", "Params", "Out"], rows)


def render_enums(schema) -> list[str]:
    out: list[str] = []
    for enum in schema.enums.values():
        out.extend([f"### {enum.name}", ""])
        rows = [[name, value] for name, value in enum.members.items()]
        out.extend(table(["Member", "Wire"], rows))
        out.append("")
    return out


def render_messages(schema) -> list[str]:
    out: list[str] = []
    for message in schema.messages.values():
        out.extend([f"### {message.name}", ""])
        if message.reserved_tags or message.reserved_names or message.next_id is not None:
            out.append(
                f"Reserved tags: `{list(message.reserved_tags)}`; "
                f"reserved names: `{list(message.reserved_names)}`; "
                f"next id: `{message.next_id}`."
            )
            out.append("")
        rows = [
            [
                field.name,
                field.tag,
                type_name(field.type),
                "yes" if field.optional else "no",
                "yes" if field.transient else "no",
                field.merge or "-",
            ]
            for field in message.fields
        ]
        out.extend(table(["Field", "Tag", "Type", "Optional", "Transient", "Merge"], rows))
        out.append("")
    return out


def render_matrix(schema) -> list[str]:
    service_requests = {
        type_name(param_type)
        for service in schema.services.values()
        for method in service.methods
        for param_name, param_type in method.params
        if param_name == "request"
    }
    rows = []
    for request, (response, handler, cli) in REQUEST_MATRIX.items():
        rows.append(
            [
                request,
                response,
                handler,
                cli,
                "core service" if request in service_requests else "CLI-local support data",
            ]
        )
    return table(["Request", "Response", "Core Handler", "CLI Family", "Contract"], rows)


def main() -> None:
    schema = load_schema()
    lines: list[str] = [
        "# GWZ Message Catalog",
        "",
        "> Generated by `docs/generate_message_catalog.py` from `protocol/gwz.taut.py`.",
        "> Do not hand-edit the mechanical tables; update the schema or generator and rerun it.",
        "",
        "Schema identity: `gwz.protocol/v0`.",
        "",
        "Source file: `gwz-core/protocol/gwz.taut.py`.",
        "",
        "The catalog is the taut protocol layer. It is separate from the Rust library",
        "API and from workspace artifact YAML schemas.",
        "",
        "## Service Methods",
        "",
    ]
    lines.extend(render_service(schema))
    lines.extend(
        [
            "",
            "## CLI-Local Protocol Values",
            "",
            "`ExecMode`, `ExecRequest`, `ExecResponse`, and `ExecResult` support",
            "`gwz forall` machine rendering. They are schema values only; `gwz-core`",
            "has no service method and no handler that executes commands.",
            "",
            "## Request/Response Matrix",
            "",
        ]
    )
    lines.extend(render_matrix(schema))
    lines.extend(["", "## Enums", ""])
    lines.extend(render_enums(schema))
    lines.extend(["## Messages", ""])
    lines.extend(render_messages(schema))
    lines.extend(
        [
            "## Evolution Notes",
            "",
            "- Field tags are stable wire identifiers. Do not reuse retired tags or names.",
            "- Additive optional fields are the normal compatibility path.",
            "- Regenerate `src/protocol/generated.rs`, `src/cbor.rs`, and `protocol/corpus/` after schema edits.",
            "- Regenerate this catalog after schema edits and review the request/response matrix for new handlers.",
            "- Keep CLI-local values clearly marked until taut module support lets the CLI own its own IR.",
            "",
        ]
    )
    OUT_PATH.write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    main()
