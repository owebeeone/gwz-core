#!/usr/bin/env python3
"""Source guard for the local carrier's peer rule (convergence plan 2.1): no hook that weakens it.

The local carrier (src/transport_host/carrier/) accepts a peer only through `PeerPolicy::verify`, a pure function of
two identities, and builds a policy only through `PeerPolicy::current_user`. This check fails when the carrier's
production code (every file under the module except its `tests/` directory) holds anything that could stand in for a
test-only or configurable override:

- a read of the environment (`std::env`, `env!`, `option_env!`, `getenv`): no variable changes what is accepted;
- a conditional-compilation switch other than the listed ones: the module's `mod tests;` and the re-export of
  platform items for the rows, so no build variant has a different rule. `cfg!(..)` in expression position is a
  run-time branch on a build variant (`if !cfg!(test) { return Ok(()); }` is strict in a test build and lax in
  production, and no row can see it), so it has a list of its own, which is empty for every file: no carrier file
  branches on a build variant at run time, and the platform conditions (Android's `SO_PEERPIDFD`) are allowed only as
  a `cfg_if!` or attribute condition;
- an identifier that names a bypass (`skip_peer`, `insecure`, `accept_any`, `for_tests`, `test_only`, ...);
- a `PeerPolicy { .. }` struct literal outside peer.rs, where the one constructor lives;
- a use of the policy's `own` field outside peer.rs (an assignment such as `policy.own = ...` would replace the identity
  the rule compares against): the field is private, the accessor `own()` is a call, and any other `.own` fails;
- the test-build constructor `from_identity` named outside peer.rs, where it is defined under `cfg(test)`;
- a `verify` function outside peer.rs, or a platform module (unix.rs, windows.rs) that does not call `.verify(` with both
  relations (the listener's check of its client, and the client's check of its listener);
- the sandbox probes (plan 2.3a, local/sandbox.rs and sandbox/): they read /proc and the system, never the environment
  (the environment rule above covers them), may switch on the platform only as sandbox.rs lists, and are called: unix.rs
  must call `sandbox::of_peer(` and peer.rs `sandbox::of_self(`, so that a refactor cannot drop a probe;
- the hold's order (unix.rs `check_peer`): the probe is followed by the consuming confirmation, `hold.confirm(`, which
  unix.rs must call, and `check_peer` calls no `.check(` (the non-consuming test of a hold, which a probe could follow
  and which would let the confirmation be dropped without a compile error).

A token scan, like its siblings: conservative, not name resolution. Comments and string literals are not scanned.
Run with no arguments from anywhere; `--root DIR` points it at another gwz-core checkout (the unit tests do).
"""
from pathlib import Path
import argparse
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
MODULE = 'src/transport_host/carrier'

FORBIDDEN_ENV = re.compile(r'\b(?:std\s*::\s*)?env\s*::|\benv!\s*\(|\boption_env!\s*\(|\bgetenv\b|\bsecure_getenv\b')
FORBIDDEN_NAMES = re.compile(
    r'\b\w*(?:skip_peer|no_peer|disable_peer|peer_check_off|insecure|bypass|override_peer|accept_any|'
    r'allow_any_peer|trust_all|unchecked_peer|dangerous|test_only|for_tests?|for_testing|set_policy|'
    r'peer_override|policy_override)\w*\b',
    re.IGNORECASE,
)
CFG = re.compile(r'cfg\s*\(([^\n]*?)\)\s*[\]{]')
# Exactly these switches may appear, in these files.
ALLOWED_CFG = {
    'mod.rs': ['test'],
    'local/peer.rs': ['test'],
    'local/windows.rs': ['test'],
    'local/mod.rs': ['unix', 'windows', 'all(test, unix)', 'all(test, windows)'],
}
# `cfg!(..)` in expression position: no switch at all in any carrier file (a run-time branch on a build variant).
ALLOWED_CFG_EXPRESSION: dict[str, list[str]] = {}
# Platform items (not switches on the peer rule) in the platform file that picks an OS call.
PLATFORM_CFG_FILES = {
    'local/unix.rs': ['target_vendor = "apple"', 'any(target_os = "linux", target_os = "android")', 'target_os = "android"'],
    'local/sandbox.rs': ['target_vendor = "apple"', 'any(target_os = "linux", target_os = "android")'],
}
# Calls a file must make, so that a probe or the rule cannot be dropped by an edit: (pattern, spelling, why).
REQUIRED_CALLS = {
    'local/unix.rs': [
        (r'sandbox::of_peer\(', 'sandbox::of_peer(', 'the sandbox probes must run on every peer and on the caller'),
        (r'\bhold\s*\.\s*confirm\s*\(', 'hold.confirm(', 'the probe must be followed by the consuming confirmation of the hold'),
    ],
    'local/peer.rs': [
        (r'sandbox::of_self\(', 'sandbox::of_self(', 'the sandbox probes must run on every peer and on the caller'),
    ],
}
# A call `check_peer` (unix.rs) must not make: the hold's non-consuming test, `hold.check(fd)`.
HOLD_CHECK = re.compile(r'(?:\.|::)\s*check\s*\(')


def strip(source: str) -> str:
    """The source without comments and string literals (their text replaced by spaces, newlines kept)."""
    out = []
    i = 0
    n = len(source)
    while i < n:
        two = source[i:i + 2]
        if two == '//':
            while i < n and source[i] != '\n':
                out.append(' ')
                i += 1
        elif two == '/*':
            depth = 0
            while i < n:
                if source[i:i + 2] == '/*':
                    depth += 1
                    out.append('  ')
                    i += 2
                elif source[i:i + 2] == '*/':
                    depth -= 1
                    out.append('  ')
                    i += 2
                    if depth == 0:
                        break
                else:
                    out.append('\n' if source[i] == '\n' else ' ')
                    i += 1
        elif source[i] == 'r' and re.match(r'r#*"', source[i:]):
            hashes = len(re.match(r'r(#*)"', source[i:]).group(1))
            end = source.find('"' + '#' * hashes, i + 2 + hashes)
            end = n if end < 0 else end + 1 + hashes
            out.append(re.sub(r'[^\n]', ' ', source[i:end]))
            i = end
        elif source[i] == '"':
            j = i + 1
            while j < n and source[j] != '"':
                j += 2 if source[j] == '\\' else 1
            out.append(re.sub(r'[^\n]', ' ', source[i:j + 1]))
            i = j + 1
        else:
            out.append(source[i])
            i += 1
    return ''.join(out)


def function_body(text: str, name: str) -> tuple[int, str] | None:
    """The offset and the text between the braces of `fn name` in comment- and string-free `text`, or None when there is
    no such function."""
    found = re.search(rf'\bfn\s+{name}\b', text)
    if not found:
        return None
    start = text.find('{', found.end())
    if start < 0:
        return None
    depth, j = 0, start
    while j < len(text):
        depth += {'{': 1, '}': -1}.get(text[j], 0)
        j += 1
        if depth == 0:
            break
    return start + 1, text[start + 1:j - 1]


def production_files(root: Path) -> list[Path]:
    base = root / MODULE
    return sorted(p for p in base.rglob('*.rs') if 'tests' not in p.relative_to(base).parts[:-1])


def violations(root: Path) -> list[str]:
    base = root / MODULE
    found = []
    files = production_files(root)
    if not files:
        return [f'{MODULE}: no production files found']
    for path in files:
        rel = path.relative_to(base).as_posix()
        original = path.read_text(encoding='utf-8')
        text = strip(original)
        for number, line in enumerate(text.splitlines(), 1):
            if FORBIDDEN_ENV.search(line):
                found.append(f'{rel}:{number}: reads the environment: {line.strip()}')
            match = FORBIDDEN_NAMES.search(line)
            if match:
                found.append(f'{rel}:{number}: names a bypass ({match.group(0)}): {line.strip()}')
            if rel != 'local/peer.rs' and re.search(r'(?<!struct )(?<!impl )\bPeerPolicy\s*\{', line):
                found.append(f'{rel}:{number}: builds a PeerPolicy outside peer.rs: {line.strip()}')
            if rel != 'local/peer.rs' and re.search(r'\.own\b(?!\s*\()', line):
                found.append(f'{rel}:{number}: uses the policy\'s own field outside peer.rs: {line.strip()}')
            if rel != 'local/peer.rs' and re.search(r'\w*from_identity\b', line):
                found.append(f'{rel}:{number}: names the test-build policy constructor outside peer.rs: {line.strip()}')
            if rel != 'local/peer.rs' and re.search(r'\bfn\s+verify\b', line):
                found.append(f'{rel}:{number}: a second verify function: {line.strip()}')
        # An attribute or `cfg_if` condition `cfg(..)` takes the module's list; the expression `cfg!(..)`, which a probe
        # could branch on at run time as well as at build time, takes its own (empty).
        for match in re.finditer(r'cfg\s*\(|\bcfg\s*!\s*\(', text):
            if '!' in match.group(0):
                allowed = ALLOWED_CFG_EXPRESSION.get(rel, [])
            else:
                allowed = ALLOWED_CFG.get(rel, []) + PLATFORM_CFG_FILES.get(rel, [])
            depth, j = 1, match.end()
            while j < len(text) and depth:
                depth += {'(': 1, ')': -1}.get(text[j], 0)
                j += 1
            switch = ' '.join(original[match.end():j - 1].split())
            if switch not in allowed:
                line = text.count('\n', 0, match.start()) + 1
                found.append(f'{rel}:{line}: a conditional-compilation switch cfg({switch}) the rule does not list')
        for pattern, spelling, why in REQUIRED_CALLS.get(rel, []):
            if not re.search(pattern, text):
                found.append(f'{rel}: never calls {spelling}; {why}')
        if rel == 'local/unix.rs':
            located = function_body(text, 'check_peer')
            if located is None:
                found.append(f'{rel}: has no check_peer function; the hold, the probes and the confirmation are ordered there')
            else:
                offset, body = located
                for match in HOLD_CHECK.finditer(body):
                    line = text.count('\n', 0, offset + match.start()) + 1
                    found.append(f'{rel}:{line}: check_peer calls .check(, the hold\'s non-consuming test; only the consuming hold.confirm( may follow the probe')
        if rel in ('local/unix.rs', 'local/windows.rs'):
            for relation in ('Relation::ListenerChecksClient', 'Relation::ClientChecksListener'):
                if relation not in text:
                    found.append(f'{rel}: never checks with {relation}; the listener and the client must each check')
            if '.verify(' not in text:
                found.append(f'{rel}: never calls .verify(')
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--root', type=Path, default=ROOT)
    options = parser.parse_args(argv)
    found = violations(options.root)
    for line in found:
        print(f'check_carrier_peer_rule: {line}', file=sys.stderr)
    if found:
        return 1
    print(f'check_carrier_peer_rule: ok ({len(production_files(options.root))} files)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
