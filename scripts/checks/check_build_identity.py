#!/usr/bin/env python3
"""Require every GWZ Bazel artifact to use its Cargo package version."""
import ast
from pathlib import Path
import sys
import tomllib


def check(root: Path) -> list[str]:
    failures = []
    for member in ("gwz-core", "gwz-cli"):
        manifest = tomllib.loads((root / member / "Cargo.toml").read_text())
        version = manifest["package"]["version"]
        tree = ast.parse((root / member / "BUILD.bazel").read_text())
        rules = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
                 and isinstance(node.func, ast.Name) and node.func.id in {"rust_library", "rust_binary"}]
        if not rules:
            failures.append(f"{member}: no artifact rules")
        for rule in rules:
            fields = {entry.arg: entry.value for entry in rule.keywords}
            actual = fields.get("version")
            if not isinstance(actual, ast.Constant) or actual.value != version:
                failures.append(f"{member}/BUILD.bazel:{rule.lineno}: version must equal Cargo package {version}")
    return failures


if __name__ == '__main__':
    findings = check(Path(__file__).resolve().parents[3])
    print('\n'.join(findings) if findings else 'build identity versions: ok')
    sys.exit(bool(findings))
