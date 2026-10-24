"""Every gwz-core workflow job that builds or runs gwz-core has every path-dependency sibling beside it.

Round 1 remediation C-P2-1 (the Phase 5 code review). Step 5.2 made `gwz-git = { path = "../gwz-git" }` an
unconditional dependency and taught `.github/checkout-siblings.sh` to fetch it, but the Transport candidate workflow
checks its siblings out by hand and never got gwz-git: both its jobs were red from the commit on, and every gate that
passed locally had the sibling beside gwz-core. Cargo reads a path dependency's manifest in every build, so a job that
runs `cargo`, `prepare.py` or `run_tests.py` needs every `path = "../X"` of Cargo.toml checked out: by calling
`checkout-siblings.sh` (which names its siblings in its `for name in ...` line), `checkout-git2-rs.sh`, or an
`actions/checkout` step whose `path:` is X. This reads the workflows as text (no YAML library: CI's Python has none).
"""
import os
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / '.github' / 'workflows'
RUNS_GWZ_CORE = re.compile(r'(?<![\w./-])cargo(?![\w-])|prepare\.py|run_tests\.py')
SCRIPTS = {'checkout-git2-rs.sh': {'git2-rs'}}


def siblings(cargo_toml):
    """The sibling directories a manifest names by path (`path = "../X"` or `"../X/sub"`), whatever its table."""
    return set(re.findall(r'path\s*=\s*"\.\./([^/"]+)', cargo_toml))


def script_siblings(script):
    """The siblings checkout-siblings.sh clones: the names in its `for name in ...; do` line."""
    match = re.search(r'^for name in ([^;\n]+);? ?do', script, re.MULTILINE)
    return set(match.group(1).split()) if match else set()


def jobs(text):
    """{job name: its text} under the top-level `jobs:` key (jobs sit at two spaces of indentation)."""
    lines = text.splitlines()
    start = next((i for i, line in enumerate(lines) if re.fullmatch(r'jobs:\s*', line)), None)
    if start is None:
        return {}
    result, name = {}, None
    for line in lines[start + 1:]:
        if line and not line.startswith((' ', '#')):
            break
        header = re.fullmatch(r'  ([A-Za-z0-9_-]+):\s*', line)
        if header:
            name = header.group(1)
            result[name] = []
        elif name is not None:
            result[name].append(line)
    return {key: '\n'.join(value) for key, value in result.items()}


def uncovered(job, needed, siblings_script):
    """The siblings of `needed` that `job` (a text) does not check out; empty when it runs no cargo or Python gate."""
    # The bootstrap crate is a package of its own (.github/bootstrap-crate): cargo on it does not read gwz-core's manifest.
    code = '\n'.join(line for line in job.splitlines()
                     if not line.lstrip().startswith('#') and '--manifest-path .github/bootstrap-crate/' not in line)
    if not RUNS_GWZ_CORE.search(code):
        return set()
    have = {path.strip().strip('"\'') for path in re.findall(r'^\s*path:\s*(\S+)', code, re.MULTILINE)}
    for script, names in {**SCRIPTS, 'checkout-siblings.sh': script_siblings(siblings_script)}.items():
        if script in code:
            have |= names
    return needed - have


def tuple_gaps(workflows, cargo_toml, siblings_script):
    """[(workflow file name, job name, missing siblings)] over {file name: text}."""
    needed = siblings(cargo_toml)
    return [(file, name, sorted(missing))
            for file, text in sorted(workflows.items())
            for name, job in jobs(text).items()
            if (missing := uncovered(job, needed, siblings_script))]


JOB = """\
name: W
jobs:
  build:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v6
        with:
          path: gwz-core
{extra}
      - run: cargo test --lib
  docs:
    steps:
      - run: echo nothing to build
"""
MANIFEST = '[dependencies]\ngwz-git = { path = "../gwz-git" }\ngit2 = { path = "../git2-rs", version = "=1" }\n' \
           '[dev-dependencies]\nlibgit2-sys = { path = "../git2-rs/libgit2-sys" }\n'
SCRIPT = 'for name in gwz-transport gwz-sspi gwz-git; do\n  echo $name\ndone\n'


class WorkflowSiblings(unittest.TestCase):
    def gaps(self, extra, manifest=MANIFEST):
        return tuple_gaps({'w.yml': JOB.format(extra=extra)}, manifest, SCRIPT)

    def test_the_manifest_names_its_siblings_in_every_table(self):
        self.assertEqual(siblings(MANIFEST), {'gwz-git', 'git2-rs'})
        self.assertEqual(script_siblings(SCRIPT), {'gwz-transport', 'gwz-sspi', 'gwz-git'})

    def test_a_job_that_builds_without_a_sibling_is_named_with_the_sibling(self):
        self.assertEqual(self.gaps(''), [('w.yml', 'build', ['git2-rs', 'gwz-git'])])

    def test_each_way_of_checking_a_sibling_out_covers_it(self):
        self.assertEqual(self.gaps('      - run: bash .github/checkout-git2-rs.sh\n      - run: bash .github/checkout-siblings.sh'), [])
        by_hand = ('      - uses: actions/checkout@v6\n        with:\n          path: gwz-git\n'
                   '      - run: bash .github/checkout-git2-rs.sh')
        self.assertEqual(self.gaps(by_hand), [])
        self.assertEqual(self.gaps('      - run: bash .github/checkout-git2-rs.sh'), [('w.yml', 'build', ['gwz-git'])])

    def test_a_comment_is_not_a_checkout_and_a_job_with_no_build_needs_none(self):
        self.assertEqual(self.gaps('      # bash .github/checkout-siblings.sh\n      # path: gwz-git'),
                         [('w.yml', 'build', ['git2-rs', 'gwz-git'])])
        names = [name for _, name, _ in self.gaps('')]
        self.assertNotIn('docs', names)

    def test_every_way_of_running_the_build_counts(self):
        for command in ('cargo test', 'cargo\tclippy', 'python tests/transport_backend/prepare.py x',
                        'python scripts/run_tests.py --lib'):
            with self.subTest(command=command):
                text = JOB.format(extra='').replace('cargo test --lib', command)
                self.assertTrue(tuple_gaps({'w.yml': text}, MANIFEST, SCRIPT))
        for command in ('echo cargo-deny', 'rustup toolchain install 1.95.0', 'pip install cargo-binstall'):
            with self.subTest(command=command):
                text = JOB.format(extra='').replace('cargo test --lib', command)
                self.assertEqual(tuple_gaps({'w.yml': text}, MANIFEST, SCRIPT), [])

    def test_cargo_on_the_bootstrap_crate_alone_needs_no_sibling(self):
        text = JOB.format(extra='').replace('cargo test --lib',
                                            'cargo publish --locked --manifest-path .github/bootstrap-crate/Cargo.toml')
        self.assertEqual(tuple_gaps({'w.yml': text}, MANIFEST, SCRIPT), [])

    def test_the_transport_candidate_workflow_at_the_tuple_before_the_remedy_is_red(self):
        # The workflow as it stood at gwz-core f0bff70f: both jobs hand-check-out their siblings, gwz-git is not one.
        text = (ROOT / '.github/workflows/transport-candidate.yml').read_text(encoding='utf-8')
        before = text.replace('run: bash .github/checkout-siblings.sh', 'run: echo no sibling script')
        gaps = tuple_gaps({'transport-candidate.yml': before},
                          (ROOT / 'Cargo.toml').read_text(encoding='utf-8'),
                          (ROOT / '.github/checkout-siblings.sh').read_text(encoding='utf-8'))
        self.assertEqual([(name, missing) for _, name, missing in gaps],
                         [('candidate', ['gwz-git']), ('candidate-windows', ['gwz-git'])])

    def test_every_real_workflow_job_has_every_sibling(self):
        workflows = {path.name: path.read_text(encoding='utf-8') for path in sorted(WORKFLOWS.glob('*.yml'))}
        gaps = tuple_gaps(workflows, (ROOT / 'Cargo.toml').read_text(encoding='utf-8'),
                          (ROOT / '.github/checkout-siblings.sh').read_text(encoding='utf-8'))
        self.assertEqual(gaps, [], 'jobs that run cargo, prepare.py or run_tests.py without every path sibling beside them')

    def test_gwz_git_tests_run_on_linux_and_macos_and_windows(self):
        # C-P2-2: the moved native commit/tag and reads tests ran on every platform in gwz-core's platform-matrix and
        # must still run on each of them somewhere; gwz-git's own workflow is that place.
        sibling = Path(os.environ.get('GWZ_GIT_CHECKOUT') or ROOT.parent / 'gwz-git')
        workflow = sibling / '.github' / 'workflows' / 'ci.yml'
        if not workflow.is_file():
            self.skipTest(f'no gwz-git checkout at {sibling}')
        text = workflow.read_text(encoding='utf-8')
        for runner in ('ubuntu-24.04-arm', 'macos-14', 'windows-2022'):
            self.assertIn(runner, text)
        self.assertEqual(len(re.findall(r'cargo test --locked', text)), 2)  # the unix matrix job and the windows job
        for command in ('cargo fmt --check', 'cargo clippy --locked --all-targets -- -D warnings'):
            self.assertEqual(text.count(command), 2)


if __name__ == '__main__':
    unittest.main()
