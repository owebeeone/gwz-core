#!/usr/bin/env python3
"""Rule (a)'s source test: each candidate switch's sites equal the repository's inventory file.

Rule (a) of dev-docs/GwzTransportReleasePlanAmendment-2.md §3.13 (TR2.12) names
two switches: `gwz_transport_candidate`, until S7.1 (1.1.0) removes it, and
`gwz_session_candidate`, until S7.1 (1.2.0) removes it. Each product repository
(gwz-core, gwz-cli, gwz-py) keeps `scripts/candidate_switch_inventory.txt`: a
`#` header, then one line per site in sorted order, the switch, the file and the
symbol, which any run of whitespace separates (`--list` writes two spaces). The
program checkpoint records each file's digest, and the S7.1 steps update the
files with the switches.

A site is a `#[cfg(...)]`, `#![cfg(...)]` or `#[cfg_attr(...)]` attribute, a
`cfg_if!` arm's condition among them, or a `cfg!(...)` whose tokens name a
switch, once per switch it names. Comments and literals never name one, so a
lint's `reason` string is not a site. Its symbol is the function the site sits
in. Outside a function it is the item an attribute gates, which in a `cfg_if!`
arm is the arm's first item, a `use` only when the arm declares nothing else, or
the item whose expression holds a `cfg!`; failing those (a field, a variant, an
inner attribute), the item around the site, or `(file)` at a file's top. An item
is named as check_cfg_boundaries.py names a target: `fn name`, `mod name` and so
on, and a `use`, an `impl` or an `extern` block by its rendered text.

The check fails on a site the inventory does not list (NEW), on a listed line
that is no site (STALE), on lines out of order (ORDER), and on a switch that
neither the repository's `build.rs` nor its `Cargo.toml` declares in
check-cfg (UNDECLARED). `--list` prints the sites in the inventory's form. The
scan is lexical, reuses check_cfg_boundaries.py's lexer and file walk, and
inspects every platform branch without compiling any of them.
"""
import argparse
from collections import Counter
import importlib.util
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
INVENTORY = Path('scripts') / 'candidate_switch_inventory.txt'
SWITCHES = ('gwz_transport_candidate', 'gwz_session_candidate')
NAMED = {'fn', 'mod', 'struct', 'enum', 'union', 'trait', 'type', 'const', 'static'}

_spec = importlib.util.spec_from_file_location('check_cfg_boundaries',
                                               Path(__file__).with_name('check_cfg_boundaries.py'))
cfgb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cfgb)


def name(a, k: int) -> str | None:
    """The name of the item whose keyword is token k, else None."""
    word = a.text(k)
    if word in ('use', 'impl', 'extern'):
        return cfgb.render(a.toks[k:a.end(k, braces=word == 'use')])
    n = k + 1 + (word == 'static' and a.text(k + 1) == 'mut')
    return f'{word} {a.text(n)}' if word in NAMED and a.kind(n) == 'id' else None


def header(a, brace: int) -> int:
    """The keyword of the item or statement whose body opens at `brace` (the scan in_item_list makes)."""
    h = brace - 1
    while h >= 0:
        t = a.text(h)
        if (t in (')', ']') or (t == '}' and a.generic_block(a.match.get(h, -1)))) and h in a.match:
            h = a.match[h] - 1
        elif t in (';', '{', '}'):
            break
        else:
            h -= 1
    h += 1
    while a.attribute(h) is not None:
        h = a.attribute(h) + 1
    return a.keyword(h)


def gated(a, i: int) -> int | None:
    """The keyword of the first item the outer attribute at token i gates, else None."""
    if a.text(i + 1) != '[':
        return None
    k = a.match[i + 1] + 1
    while a.attribute(k) is not None:
        k = a.attribute(k) + 1
    if a.text(i - 1) != 'if' or a.text(k) != '{':
        return a.keyword(k)
    first_use, k, stop = None, k + 1, a.match[k]
    while k < stop:
        while a.attribute(k) is not None:
            k = a.attribute(k) + 1
        k = a.keyword(k)
        if a.text(k) == 'use':
            first_use = k if first_use is None else first_use
        elif name(a, k):
            return k
        end = a.end(k, braces=a.text(k) == 'use')
        k = (a.match.get(end, end) if a.text(end) == '{' else end) + 1
    return first_use


def symbol(a, i: int) -> str:
    """The symbol of the site at token i, the `#` of its attribute or the `cfg` of its `cfg!`."""
    around, p = [], a.parent[i]  # the named items whose braces hold the site, innermost first
    while p is not None:
        k = header(a, p) if a.text(p) == '{' else None
        if k is not None and a.text(k) != 'use' and name(a, k):
            around.append(k)
        p = a.parent[p]
    k = next((k for k in around if a.text(k) == 'fn'), None)
    if k is None and a.text(i) == '#':
        k = gated(a, i)
    elif k is None:  # a cfg! in an item's expression: that item
        t = i
        while a.parent[t] is not None and a.text(a.parent[t]) != '{':
            t = a.parent[t]
        k = header(a, t)
    if k is None or not name(a, k):
        k = around[0] if around else None
    return name(a, k) if k is not None else '(file)'


def sites(source: str) -> list[tuple[str, str]]:
    """Each (switch, symbol) site in source order."""
    a, found = cfgb.Analysis(source), []
    for i in range(a.n):
        j = i + 1 + (a.text(i + 1) == '!')
        if a.text(i) == '#' and a.text(j) == '[' and a.text(j + 1) in ('cfg', 'cfg_attr') and j in a.match:
            span = range(j + 2, a.match[j])
        elif a.text(i) == 'cfg' and a.text(i + 1) == '!' and a.text(i + 2) in ('(', '[', '{') and (i + 2) in a.match:
            span = range(i + 3, a.match[i + 2])
        else:
            continue
        named = {a.text(k) for k in span if a.kind(k) == 'id'}
        found += [(switch, symbol(a, i)) for switch in SWITCHES if switch in named]
    return found


def scan(repo: Path) -> list[str]:
    """Every site in the repository's Rust files, as sorted inventory lines."""
    rows = []
    for path in cfgb.rust_files(repo, ['.']):
        source = path.read_text(encoding='utf-8')
        if any(switch in source for switch in SWITCHES):
            rows += [f'{switch}  {path.relative_to(repo).as_posix()}  {symbol}' for switch, symbol in sites(source)]
    return sorted(rows)


def declared(repo: Path) -> set[str]:
    """The cfg names build.rs's `rustc-check-cfg` lines and Cargo.toml's `check-cfg` lists declare."""
    names = set()
    if (repo / 'build.rs').is_file():
        names.update(re.findall(r'rustc-check-cfg=cfg\((\w+)\)', (repo / 'build.rs').read_text(encoding='utf-8')))
    if (repo / 'Cargo.toml').is_file():
        for listed in re.findall(r'check-cfg\s*=\s*\[([^\]]*)\]', (repo / 'Cargo.toml').read_text(encoding='utf-8')):
            names.update(re.findall(r'cfg\((\w+)\)', listed))
    return names


def listed(inventory: Path) -> list[str]:
    """The inventory's lines past its `#` header, each with its three fields two spaces apart."""
    lines = map(str.strip, inventory.read_text(encoding='utf-8').splitlines())
    return ['  '.join(line.split(None, 2)) for line in lines if line and not line.startswith('#')]


def check(repo: Path, inventory: Path | None = None) -> list[str]:
    repo = repo.resolve()
    inventory = inventory or repo / INVENTORY
    if not inventory.is_file():
        return [f'MISSING {inventory}: create it from --list']
    listed_rows, found, names = listed(inventory), scan(repo), declared(repo)
    errors = [f'UNDECLARED {switch}: neither build.rs nor Cargo.toml declares it in check-cfg'
              for switch in SWITCHES if switch not in names]
    errors += [f'NEW    {row}' for row in sorted((Counter(found) - Counter(listed_rows)).elements())]
    errors += [f'STALE  {row}' for row in sorted((Counter(listed_rows) - Counter(found)).elements())]
    if not errors and listed_rows != found:
        errors.append(f'ORDER  {inventory} lists the sites out of order; write them as --list prints them')
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    parser.add_argument('--repo', type=Path, default=ROOT, help='repository to scan (default: this gwz-core)')
    parser.add_argument('--inventory', type=Path, help=f'inventory file (default: <repo>/{INVENTORY.as_posix()})')
    parser.add_argument('--list', action='store_true', help="print the repository's sites in inventory form")
    options = parser.parse_args(argv)
    if options.list:
        for row in scan(options.repo.resolve()):
            print(row)
        return 0
    repo = options.repo.resolve()
    inventory = options.inventory or repo / INVENTORY
    errors = check(repo, inventory)
    if errors:
        print(f'candidate switch inventory check failed ({repo}):', file=sys.stderr)
        for error in errors:
            print(f'  {error}', file=sys.stderr)
        print('Every site where a candidate switch appears in a cfg is an inventory line, and every line a site: '
              'update the inventory in the change that moves a site (rule (a), TR2.12).', file=sys.stderr)
        return 1
    print(f'candidate switch inventory: the {len(listed(inventory))} sites in {repo} match {inventory}, and both '
          'switches are declared')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
