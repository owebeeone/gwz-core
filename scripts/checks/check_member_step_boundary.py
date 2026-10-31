#!/usr/bin/env python3
"""Tripwire for the member step boundary of the handler layer (core session design 5.1; step 2.3c).

Two rules over `src/workspace_ops` production files (test files, `tests/` trees and inline `#[cfg(test)]` modules are
exempt), checked on the source with comments, strings and character literals blanked, so a mention in a doc comment is
not a use:

(a) The handler layer names no `session_host`. The member lock manager lives in `operation`, beside the `MemberSteps`
    port the handlers take; the session host owns and serves the one manager. A `session_host` path in a handler
    closes the module cycle `session_host::served` -> `workspace_ops` -> `session_host` that remediation round 1 of
    step 2.3c removed.
(b) Every network update of a member's refs, a call `backend.fetch(`, `.push(`, `.push_prepared(` or `.tag_fetch(` on
    a backend (a receiver whose name ends in `backend`), follows a member step entry `.enter(` that holds its guard
    over the call: some block enclosing the call, inside the same function, has the `.enter(` earlier at its own
    depth (`let _guard = steps.enter(..)?;`, `match steps.enter(..) { Ok(_guard) => backend.push_prepared(..) }`,
    `steps.enter(..).and_then(|_guard| { backend.fetch(..) })`). An `.enter(` in a block that closed before the call,
    or in another function, guards nothing.

It is a token scan, not Rust name resolution, like `check_target_selection_boundary.py`; it catches the unlocked call a
later handler would add and the lock a refactor would drop. `ALLOWED` names a call that needs no member step, by file
and function, with the reason; an entry nothing needs any more fails until it is removed.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
OPS = "src/workspace_ops"

# (relative path, function name) -> why a call there needs no member step. None today: every call is under a step.
ALLOWED: dict[tuple[str, str], str] = {}

SESSION_HOST = re.compile(r"(?<![A-Za-z0-9_])session_host(?![A-Za-z0-9_])")
UPDATE = re.compile(r"\w*backend\s*\.\s*(fetch|push|push_prepared|tag_fetch)\s*\(")
ENTER = re.compile(r"\.\s*enter\s*\(")
FN = re.compile(r"(?<![A-Za-z0-9_])fn\s+([A-Za-z_][A-Za-z0-9_]*)")
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*(?:mod\s+\w+\s*)?\{")


def blank(source: str) -> str:
    """`source` with comments, string literals and character literals replaced by spaces (newlines kept), so every
    offset and line number stays the same."""
    out = list(source)
    i, n = 0, len(source)

    def wipe(start: int, end: int) -> None:
        for k in range(start, min(end, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        two = source[i:i + 2]
        char = source[i]
        if two == "//":
            end = source.find("\n", i)
            end = n if end < 0 else end
            wipe(i, end)
            i = end
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if source[j:j + 2] == "/*":
                    depth, j = depth + 1, j + 2
                elif source[j:j + 2] == "*/":
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            wipe(i, j)
            i = j
        elif char == "r" and re.match(r'r#*"', source[i:i + 12]) and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
            hashes = len(re.match(r"r(#*)", source[i:]).group(1))
            end = source.find('"' + "#" * hashes, i + 2 + hashes)
            end = n if end < 0 else end + 1 + hashes
            wipe(i, end)
            i = end
        elif char == '"':
            j = i + 1
            while j < n and source[j] != '"':
                j += 2 if source[j] == "\\" else 1
            wipe(i, j + 1)
            i = j + 1
        elif char == "'":
            literal = re.match(r"'(?:\\.[^']*|[^\\'])'", source[i:])
            if literal:
                wipe(i, i + literal.end())
                i += literal.end()
            else:
                i += 1  # a lifetime
        else:
            i += 1
    return "".join(out)


def blocks(code: str) -> tuple[list[tuple[int, int]], list[int]]:
    """Every `{..}` span as (open, close) in open order, and for each offset the index of its innermost enclosing span
    (-1 outside every span)."""
    spans: list[list[int]] = []
    innermost = [-1] * len(code)
    stack: list[int] = []
    for offset, char in enumerate(code):
        if char == "{":
            spans.append([offset, len(code)])
            stack.append(len(spans) - 1)
        elif char == "}" and stack:
            spans[stack.pop()][1] = offset
        innermost[offset] = stack[-1] if stack else -1
    return [(a, b) for a, b in spans], innermost


def without_test_modules(code: str) -> str:
    """`code` with each inline `#[cfg(test)] mod x { .. }` (and `#[cfg(test)] { .. }` arm of a `cfg_if!`) blanked."""
    for match in reversed(list(TEST_MODULE.finditer(code))):
        start = match.end() - 1
        depth, end = 1, start + 1
        while end < len(code) and depth:
            depth += (code[end] == "{") - (code[end] == "}")
            end += 1
        code = code[:match.start()] + "".join("\n" if c == "\n" else " " for c in code[match.start():end]) + code[end:]
    return code


def functions(code: str, spans: list[tuple[int, int]], innermost: list[int]) -> list[tuple[str, int, int]]:
    """Each `fn` with a body as (name, body open, body close)."""
    found = []
    for match in FN.finditer(code):
        depth, j = 0, match.end()
        while j < len(code):
            if code[j] in "([":
                depth += 1
            elif code[j] in ")]":
                depth -= 1
            elif depth == 0 and code[j] == ";":
                break
            elif depth == 0 and code[j] == "{":
                open_, close = spans[innermost[j]]
                found.append((match.group(1), open_, close))
                break
            j += 1
    return found


def findings(source: str, relative: str, allowed: dict[tuple[str, str], str] | None = None,
             used: set[tuple[str, str]] | None = None) -> list[str]:
    """The rule violations of one production file, as `path:line: message`. `used` collects the allow-list entries
    that excused a call."""
    allowed = ALLOWED if allowed is None else allowed
    code = without_test_modules(blank(source))
    line = lambda offset: code.count("\n", 0, offset) + 1  # noqa: E731
    failures = [
        f"{relative}:{line(m.start())}: `session_host` named in the handler layer; the member lock manager is in "
        "`operation` and the session host serves it"
        for m in SESSION_HOST.finditer(code)
    ]
    spans, innermost = blocks(code)
    fns = functions(code, spans, innermost)
    enters = [m.start() for m in ENTER.finditer(code)]
    for call in UPDATE.finditer(code):
        at = call.start()
        owner = min((f for f in fns if f[1] < at < f[2]), key=lambda f: f[2] - f[1], default=None)
        if owner is None:
            failures.append(f"{relative}:{line(at)}: `{call.group(1)}` outside any function")
            continue
        guarded, block = False, innermost[at]
        while block >= 0 and spans[block][0] >= owner[1]:
            if any(spans[block][0] < q < at and innermost[q] == block for q in enters):
                guarded = True
                break
            block = innermost[spans[block][0] - 1] if spans[block][0] > 0 else -1
        if guarded:
            continue
        key = (relative, owner[0])
        if key in allowed:
            if used is not None:
                used.add(key)
            continue
        failures.append(
            f"{relative}:{line(at)}: `{call.group(1)}` in `{owner[0]}` does not follow a member step `.enter(` that "
            "holds its guard over the call"
        )
    return failures


def production_files(root: Path) -> list[Path]:
    files = []
    for path in sorted((root / OPS).rglob("*.rs")):
        relative = path.relative_to(root / OPS)
        if "tests" in relative.parts[:-1] or path.name == "tests.rs" or path.name.endswith("_tests.rs"):
            continue
        files.append(path)
    return files


def scan(root: Path, allowed: dict[tuple[str, str], str] | None = None) -> list[str]:
    allowed = ALLOWED if allowed is None else allowed
    failures: list[str] = []
    used: set[tuple[str, str]] = set()
    for path in production_files(root):
        relative = path.relative_to(root).as_posix()
        failures += findings(path.read_text(encoding="utf-8"), relative, allowed, used)
    failures += [
        f"{relative}: the allow-list entry for `{name}` excuses no call; remove it"
        for (relative, name) in sorted(set(allowed) - used)
    ]
    return failures


def main() -> int:
    errors = scan(ROOT)
    print("\n".join(errors) if errors else "member step boundary: ok")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
