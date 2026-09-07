#!/usr/bin/env python3
"""Tripwire against raw selector interpretation outside its core owner.

This source check complements resolver tests; it is not a Rust data-flow proof.
It catches direct and multiline selection-field rescans and envelope-presence
checks. Frozen merge participants and literal lifecycle grammar live in the same
owner. Driver request construction and forwarding remain permitted.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
OWNER = "src/workspace_ops/target_selection.rs"
RAW = re.compile(r"\bselection\s*\.\s*(all|member_ids|paths|targets|exclude_targets|is_some|is_none)\b")


def findings(source: str) -> list[int]:
    return [source.count("\n", 0, match.start()) + 1 for match in RAW.finditer(source)]


def scan(root: Path) -> list[str]:
    failures = []
    for path in sorted((root / "src").rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if relative == OWNER or "/tests/" in relative or path.name in {"generated.rs", "tests.rs"}:
            continue
        source = path.read_text()
        # Inline test modules are exempt; production after such a module is
        # unusual and must not acquire an exemption. Remove only its balanced
        # brace span, preserving line numbers.
        for match in reversed(list(re.finditer(r"#\[cfg\(test\)\]\s*mod\s+tests\s*\{", source))):
            start = match.end() - 1
            depth, end = 1, start + 1
            while end < len(source) and depth:
                depth += (source[end] == "{") - (source[end] == "}")
                end += 1
            source = source[:match.start()] + "\n" * source[match.start():end].count("\n") + source[end:]
        failures += [f"{relative}:{line}: raw selection interpretation belongs in {OWNER}" for line in findings(source)]
    return failures


if __name__ == "__main__":
    errors = scan(ROOT)
    print("\n".join(errors) if errors else "target selection boundary: ok")
    raise SystemExit(bool(errors))
