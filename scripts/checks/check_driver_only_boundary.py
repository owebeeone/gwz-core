#!/usr/bin/env python3
"""The driver-only builder cannot make a local endpoint (step 1.7 of the transport convergence plan; the type-level row).

`src/transport_host/driver_only.rs` is what `gwz server` runs: a core-side host with no endpoint settings, no handoff and
no local endpoint, whose every request is the `cli` placement. It is that by what it can name, so this check fails when
the module's production code (the module's own file; its rows are in `driver_only_tests.rs`, which is not scanned) names
anything that makes or feeds a local endpoint:

- the types that configure or are an endpoint: `EndpointSettings`, `SshEndpointConfig`, `SshSettings`,
  `HttpsEndpointConfig`, `Handoff`, `AgentSource`, `HelperSlots`, `NativeCaller`, `NativeHost`, `CliEndpoint`, the
  connection host and its factory (`ConnectionHost`, `CoreFactory`);
- the native gate: `ProcessSafety`, `ProcessGate`, `process_safety` (the spawn gate a request's backend holds comes from
  `composition::spawn_gate()`, which this check does not forbid and `check_spawn_gate_minting.py` keeps the one site of);
- the modules that read or hold an agent, a key, a helper or the environment: `endpoint_environment`, `agent_auth`,
  `agent_source`, `agent_job`, `ssh_pool`, `ssh_key_snapshot`, `ssh_handoff`, `ssh_home`, `ssh_local`,
  `connection_factory`, `connection_host`;
- the local endpoint's constructors: `Session::endpoint`, `Session::endpoint_with_https`,
  `Session::endpoint_with_https_native`; and `Session::driver` given anything but `None` for its handoff, or
  `Session::driver_with_ssh_budgets` at all (it takes a handoff and the SSH budgets of a local driver). Every path
  through `Session` must be a call of `Session::driver`; one that is not a call (a function-pointer alias) fails;
- the local transport entry points, which build an SSH and HTTPS `TransportRuntime` from the snapshot the builder holds,
  with agent and helpers: `with_cancellable_local_transport*`, `with_local_transport*`, `TransportRuntime`, the
  `cancellable` module and `local_command`;
- an `as` alias in a `use` (it carries a forbidden name past the spelling scan), and any import outside the allow-list
  (`ALLOWED_IMPORTS`: the names the module imports today; a new import is a reviewed edit of this check);
- a read of the environment: `std::env`, `env::`, `env!`, `option_env!`, `getenv`, or a credential variable's name
  (`SSH_AUTH_SOCK`, `GH_TOKEN`, `GITHUB_TOKEN`, `SSH_ASKPASS`, `GIT_ASKPASS`) in a string: the builder takes the
  caller's `EnvironmentSnapshot` and reads `GWZ_TRANSPORT` through `transport_setting::resolve`, nothing else;
- a glob import (`use ...::*`), which would take any of the above in unseen, and any `cfg(...)`: the module builds the
  same on every platform and carries no test branch (its rows are in the separate file).

The parent `mod.rs` must declare the module. A token scan, like its siblings: conservative, not name resolution.
Comments and string literals are not scanned for identifiers. Run with no arguments from anywhere; `--root DIR`
points it at another gwz-core checkout (the unit tests do).
"""
from pathlib import Path
import argparse
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
MODULE = 'src/transport_host/driver_only.rs'
PARENT = 'src/transport_host/mod.rs'

FORBIDDEN_TYPES = [
    'EndpointSettings', 'SshEndpointConfig', 'SshSettings', 'HttpsEndpointConfig', 'Handoff', 'AgentSource',
    'HelperSlots', 'NativeCaller', 'NativeHost', 'CliEndpoint', 'ConnectionHost', 'CoreFactory',
    'ProcessSafety', 'ProcessGate', 'process_safety', 'TransportRuntime',
]
FORBIDDEN_MODULES = [
    'endpoint_environment', 'agent_auth', 'agent_source', 'agent_job', 'ssh_pool', 'ssh_key_snapshot',
    'ssh_handoff', 'ssh_home', 'ssh_local', 'connection_factory', 'connection_host', 'cancellable', 'local_command',
]
# The names `driver_only.rs` may import. `self` stands for the module it sits in.
ALLOWED_IMPORTS = {
    'CleanupReport', 'TransportPort', 'TransportRequest', 'cli_placement', 'composition', 'request',
    'RequestContext', 'invalid', 'Session', 'unavailable', 'unsupported', 'Git2Backend', 'SpawnGate',
    'EnvironmentSnapshot', 'transport_setting', 'Driver', 'Source', 'Transport', 'RequestMeta', 'ModelResult',
    'CancellationToken', 'Arc', 'Mutex',
}
FORBIDDEN_CALLS = [
    r'Session\s*::\s*endpoint\w*', r'Session\s*::\s*driver_with_ssh_budgets', r'\bwith_\w*local_transport\w*',
]
# Every `Session::name` must be a call of `Session::driver`: not `Session::driver` as a value.
SESSION_PATH = re.compile(r'Session\s*::\s*(\w+)(\s*\()?')
USE_STATEMENT = re.compile(r'\buse\b([^;]*);')
# `Session::driver(a, b, None)`: the third argument must be `None`.
DRIVER_CALL = re.compile(r'Session\s*::\s*driver\s*\(([^;]*?)\)\s*[?;,.]', re.DOTALL)
FORBIDDEN_ENV = re.compile(r'\b(?:std\s*::\s*)?env\s*::|\benv!\s*\(|\boption_env!\s*\(|\bgetenv\b|\bsecure_getenv\b')
CREDENTIAL_NAMES = re.compile(r'\b(?:SSH_AUTH_SOCK|GH_TOKEN|GITHUB_TOKEN|SSH_ASKPASS|GIT_ASKPASS)\b')
GLOB = re.compile(r'\buse\b[^;]*::\s*\*|\buse\b[^;]*::\s*\{[^;]*\*[^;]*\}')
CFG = re.compile(r'\bcfg(?:_attr|_if)?\s*[!(]')


def strip(source: str, keep_strings: bool = False) -> str:
    """The source without comments and, unless `keep_strings`, string literals (replaced by spaces, newlines kept)."""
    out = []
    i = 0
    n = len(source)

    def blank(text: str) -> str:
        return re.sub(r'[^\n]', ' ', text)

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
            out.append(source[i:end] if keep_strings else blank(source[i:end]))
            i = end
        elif source[i] == '"':
            j = i + 1
            while j < n and source[j] != '"':
                j += 2 if source[j] == '\\' else 1
            out.append(source[i:j + 1] if keep_strings else blank(source[i:j + 1]))
            i = j + 1
        else:
            out.append(source[i])
            i += 1
    return ''.join(out)


def expand_use(tree: str) -> list[str]:
    """The paths a use tree names, braces expanded: `a::{b, c::d}` is `a::b` and `a::c::d`."""
    tree = ' '.join(tree.split())
    if '{' not in tree:
        return [tree.strip()]
    head, rest = tree.split('{', 1)
    body = rest.rsplit('}', 1)[0]
    parts, depth, current = [], 0, ''
    for char in body:
        if char == '{':
            depth += 1
        elif char == '}':
            depth -= 1
        if char == ',' and depth == 0:
            parts.append(current)
            current = ''
        else:
            current += char
    parts.append(current)
    return [head + path for part in parts if part.strip() for path in expand_use(part.strip())]


def imported_names(path: str) -> list[str]:
    """The name a use path brings in: its last segment, or the one before it for `self`."""
    segments = [segment.strip() for segment in path.split('::') if segment.strip()]
    if segments and segments[-1] == 'self':
        segments = segments[:-1]
    return segments[-1:] if segments else []


def violations(root: Path) -> list[str]:
    module = root / MODULE
    parent = root / PARENT
    if not module.is_file():
        return [f'{MODULE}: missing']
    found = []
    if not parent.is_file() or not re.search(r'^\s*(?:pub\s+)?mod\s+driver_only\s*;', parent.read_text(encoding='utf-8'), re.M):
        found.append(f'{PARENT}: does not declare `mod driver_only;`')
    original = module.read_text(encoding='utf-8')
    text = strip(original)
    with_strings = strip(original, keep_strings=True)
    lines = text.splitlines()
    for number, line in enumerate(lines, 1):
        for name in FORBIDDEN_TYPES:
            if re.search(rf'\b{name}\b', line):
                found.append(f'{MODULE}:{number}: names {name}: {line.strip()}')
        for name in FORBIDDEN_MODULES:
            if re.search(rf'\b{name}\b', line):
                found.append(f'{MODULE}:{number}: names the module {name}: {line.strip()}')
        for call in FORBIDDEN_CALLS:
            match = re.search(call, line)
            if match:
                found.append(f'{MODULE}:{number}: calls {match.group(0)}: {line.strip()}')
        if FORBIDDEN_ENV.search(line):
            found.append(f'{MODULE}:{number}: reads the environment: {line.strip()}')
        if CFG.search(line):
            found.append(f'{MODULE}:{number}: a conditional-compilation switch: {line.strip()}')
    for number, line in enumerate(with_strings.splitlines(), 1):
        match = CREDENTIAL_NAMES.search(line)
        if match:
            found.append(f'{MODULE}:{number}: names the credential variable {match.group(0)}: {line.strip()}')
    for match in GLOB.finditer(text):
        number = text.count('\n', 0, match.start()) + 1
        found.append(f'{MODULE}:{number}: a glob import: {" ".join(match.group(0).split())}')
    for match in USE_STATEMENT.finditer(text):
        number = text.count('\n', 0, match.start()) + 1
        statement = ' '.join(match.group(1).split())
        if re.search(r'\bas\b', statement):
            found.append(f'{MODULE}:{number}: an `as` alias in a use statement: use {statement}')
        for path in expand_use(statement):
            for name in imported_names(path):
                if name not in ALLOWED_IMPORTS and name != '*':
                    found.append(f'{MODULE}:{number}: imports {name}, which is not in the allow-list: use {statement}')
    for match in SESSION_PATH.finditer(text):
        if match.group(1) != 'driver' or not match.group(2):
            number = text.count('\n', 0, match.start()) + 1
            found.append(f'{MODULE}:{number}: Session::{match.group(1)} is not a call of Session::driver')
    for match in DRIVER_CALL.finditer(text):
        arguments = [part.strip() for part in match.group(1).split(',')]
        if len(arguments) != 3 or arguments[2] != 'None':
            number = text.count('\n', 0, match.start()) + 1
            found.append(f'{MODULE}:{number}: Session::driver is given a handoff: ({" ".join(match.group(1).split())})')
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--root', type=Path, default=ROOT)
    options = parser.parse_args(argv)
    found = violations(options.root)
    for line in found:
        print(f'check_driver_only_boundary: {line}', file=sys.stderr)
    if found:
        return 1
    print('check_driver_only_boundary: ok')
    return 0


if __name__ == '__main__':
    sys.exit(main())
