"""Tests for the Windows compile gate script (GwzTransportWindowsParityPlan.md, step 0.4). No host is used."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('windows_lane_check', Path(__file__).with_name('windows_lane_check.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
parity = gate.parity


class FakeRun:
    """Stands in for subprocess.run: records every call and answers by the command's first words."""

    def __init__(self, claim_exit=0, shape_exits=(0, 0, 0), warnings=7, blocked_first=False):
        self.calls, self.claim_exit, self.shape_exits, self.warnings = [], claim_exit, list(shape_exits), warnings
        self.blocked_first = blocked_first

    def __call__(self, argv, **kwargs):
        self.calls.append((list(argv), kwargs))
        out = ''
        if argv[0] == 'git' and 'rev-parse' in argv:
            out = 'c0ffee' + '0' * 34 + '\n'
        elif argv[0] == 'git' and 'diff' in argv:
            out = self.diff_names
        elif argv[0] == 'ssh' and 'bash' in argv:
            script = kwargs.get('input', '')
            if script.startswith('mkdir'):
                return subprocess.CompletedProcess(argv, self.claim_exit, 'exists' if self.claim_exit else '', '')
            if 'cargo check' in script:
                if self.blocked_first:
                    self.blocked_first = False
                    return subprocess.CompletedProcess(argv, 101, 'An Application Control policy has blocked this file. (os error 4551)\n', '')
                code = self.shape_exits.pop(0)
                return subprocess.CompletedProcess(
                    argv, code, f'warning: `gwz-core` (lib) generated {self.warnings} warnings\n', '')
            if 'rustc --version' in script:
                out = 'rustc 1.95.0 (abc 2026-01-01)\n'
        elif argv[0] == 'git' and 'archive' in argv:
            out = b'TAR'
        return subprocess.CompletedProcess(argv, 0, out, '' if isinstance(out, str) else b'')

    diff_names = ''

    def ssh_scripts(self):
        return [k['input'] for a, k in self.calls if a[:1] == ['ssh'] and 'input' in k and 'bash' in a]


class Clock:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        self.t += 2.5
        return self.t


def options(*argv):
    return gate.parser().parse_args(['--receipt-dir', 'unused', *argv])


def run_main(runner, *argv, receipts):
    host = gate.Host(run=runner, clock=Clock())
    return gate.main(['--receipt-dir', str(receipts), *argv], host=host, core=Path('/nonexistent-core'))


class Labels(unittest.TestCase):
    def test_good_labels(self):
        for label in ('winp0-20261009-a', 'winp0-20261009', 'lane-20261101-cold-2'):
            self.assertEqual(gate.check_label(label), label)

    def test_bad_labels_are_refused_before_any_host_call(self):
        for label in ('', 'Winp0-20261009-a', 'winp0-a', 'winp0_20261009', '../x-20261009', 'a/b-20261009',
                      'a b-20261009', '-20261009', 'x' * 70 + '-20261009', 'winp0-20261009-', 'winp0-2026100-a'):
            with self.subTest(label), self.assertRaises(SystemExit) as caught:
                run_main(FakeRun(), '--label', label, receipts=Path('unused'))
            self.assertEqual(caught.exception.code, 2, label)


class Arguments(unittest.TestCase):
    def test_label_and_receipt_dir_are_required(self):
        with self.assertRaises(SystemExit):
            gate.parser().parse_args(['--receipt-dir', 'x'])
        with self.assertRaises(SystemExit):
            gate.parser().parse_args(['--label', 'a-20261009'])

    def test_ref_and_worktree_conflict_and_abbreviations_are_refused(self):
        with self.assertRaises(SystemExit):
            options('--label', 'a-20261009', '--ref', 'main', '--worktree')
        with self.assertRaises(SystemExit):
            options('--label', 'a-20261009', '--work')

    def test_defaults(self):
        o = options('--label', 'a-20261009')
        self.assertEqual((o.ref, o.worktree, o.tests, o.cache_from, o.if_triggered), ('HEAD', False, False, None, None))

    def test_the_ssh_options_are_the_host_rules(self):
        self.assertEqual(gate.SSH, ['ssh', '-o', 'ClearAllForwardings=yes', '-o', 'ForwardAgent=no', '-o', 'ForwardX11=no',
                                    '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', 'gianni@dabeest'])

    def test_the_three_shapes_are_the_ci_jobs(self):
        self.assertEqual([n for n, _ in gate.SHAPES], ['ordinary', 'transport', 'qualification'])
        workflow = (gate.CORE / '.github' / 'workflows' / 'transport-candidate.yml').read_text(encoding='utf-8')
        for _, flags in gate.SHAPES[1:]:
            self.assertIn(f'RUSTFLAGS: {flags}', workflow)


class Triggers(unittest.TestCase):
    def test_trigger_paths(self):
        self.assertEqual(gate.triggered(['src/git/endpoint/ssh_network.rs', 'src/transport_host/mod.rs', 'Cargo.toml',
                                         'tests/transport_backend/prepare.py', 'README.md', 'src/git/mod.rs',
                                         'scripts/run_tests.py', 'src/transport_host_extra.rs', 'docs/Cargo.toml']),
                         ['src/git/endpoint/ssh_network.rs', 'src/transport_host/mod.rs', 'Cargo.toml',
                          'tests/transport_backend/prepare.py'])

    def test_the_trigger_paths_are_the_inventory_roots_plus_the_build_inputs(self):
        data, _ = parity.load_inventory(parity.DEFAULT_INVENTORY)
        self.assertEqual(gate.trigger_patterns(), data['roots'] + list(gate.TRIGGER_EXTRA))
        self.assertEqual(sorted(gate.TRIGGER_EXTRA),
                         ['.github/*.commit', 'Cargo.lock', 'Cargo.toml', 'tests/transport_backend/prepare.py'])

    def test_every_scope_root_triggers_the_gate(self):
        """P3-3: the gate fires on every path the parity inventory covers (the old list missed three roots)."""
        hits = ['src/git/gitbackend/transport_binding.rs', 'src/git/gitbackend/transport_candidate_tests/drivers.rs',
                'src/git/gitbackend/https_transport_binding_tests.rs', 'src/git/gitbackend.rs',
                'src/transport_setting.rs', 'src/transport_setting/home.rs', 'Cargo.lock', '.github/gwz-transport.commit',
                '.github/gwz-sspi.commit']
        self.assertEqual(gate.triggered(hits), hits)

    def test_neighbours_of_the_roots_do_not_trigger_it(self):
        for path in ('src/git/gitbackend/preservation.rs', 'src/git/gitbackend/transport.rs', 'src/git/endpoint.rs',
                     'src/transport_settings.rs', 'src/transport_host_extra.rs', '.github/workflows/ci.yml',
                     '.github/checkout-git2-rs.sh', 'docs/Cargo.lock', 'scripts/checks/windows_parity/1.4.json'):
            self.assertEqual(gate.triggered([path]), [], path)

    def test_a_lane_that_touches_no_trigger_path_does_not_call_the_host(self):
        runner = FakeRun()
        runner.diff_names = 'README.md\nsrc/git/mod.rs\n'
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(run_main(runner, '--label', 'a-20261009', '--if-triggered', 'base', receipts=Path(tmp)), 0)
        self.assertEqual(runner.ssh_scripts(), [])

    def test_a_lane_that_touches_one_runs_the_gate(self):
        runner = FakeRun()
        runner.diff_names = 'README.md\nsrc/transport_host/mod.rs\n'
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(run_main(runner, '--label', 'a-20261009', '--if-triggered', 'base', receipts=Path(tmp)), 0)
        self.assertTrue(runner.ssh_scripts())


class Run(unittest.TestCase):
    def go(self, runner, *argv):
        with tempfile.TemporaryDirectory() as tmp:
            code = run_main(runner, '--label', 'winp0-20261009-a', *argv, receipts=Path(tmp))
            path = Path(tmp) / 'winp0-20261009-a' / 'receipt.json'
            return code, json.loads(path.read_text(encoding='utf-8')) if path.exists() else None

    def test_a_passing_run_writes_a_receipt_with_each_shape_timed(self):
        runner = FakeRun()
        code, receipt = self.go(runner)
        self.assertEqual(code, 0)
        self.assertEqual([(s['shape'], s['exit'], s['warnings']) for s in receipt['shapes']],
                         [('ordinary', 0, 7), ('transport', 0, 7), ('qualification', 0, 7)])
        self.assertTrue(receipt['passed'] and receipt['total_seconds'] > 0 and receipt['stage_seconds'] > 0)
        self.assertEqual((receipt['label'], receipt['source'], receipt['toolchain']),
                         ('winp0-20261009-a', 'ref HEAD', 'rustc 1.95.0 (abc 2026-01-01)'))
        self.assertEqual(receipt['host_dir'], '/e/gwz-tests/winp0-20261009-a')

    def test_a_build_script_blocked_by_the_host_is_run_once_more_and_both_attempts_are_recorded(self):
        code, receipt = self.go(FakeRun(blocked_first=True))
        self.assertEqual(code, 0)
        self.assertEqual([[a['exit'] for a in s['attempts']] for s in receipt['shapes']], [[101, 0], [0], [0]])

    def test_a_genuine_failure_is_not_retried(self):
        runner = FakeRun(shape_exits=(101, 0, 0))
        code, receipt = self.go(runner)
        self.assertEqual((code, [len(s['attempts']) for s in receipt['shapes']]), (1, [1, 1, 1]))

    def test_a_failing_shape_fails_the_run_and_the_receipt(self):
        code, receipt = self.go(FakeRun(shape_exits=(0, 101, 0)))
        self.assertEqual((code, receipt['passed']), (1, False))
        self.assertEqual([s['exit'] for s in receipt['shapes']], [0, 101, 0])

    def test_the_host_directory_is_claimed_first_and_a_taken_label_is_refused(self):
        runner = FakeRun(claim_exit=1)
        with self.assertRaises(SystemExit) as caught:
            self.go(runner)
        self.assertEqual(caught.exception.code, 2)
        scripts = runner.ssh_scripts()
        self.assertEqual(len(scripts), 1)
        self.assertTrue(scripts[0].startswith('mkdir /e/gwz-tests/winp0-20261009-a 2>&1'))
        self.assertNotIn('-p /e/gwz-tests/winp0-20261009-a ', scripts[0].split('&&')[0])  # the leaf is made without -p

    def test_an_existing_receipt_directory_is_refused_without_a_host_call(self):
        runner = FakeRun()
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / 'winp0-20261009-a').mkdir()
            with self.assertRaises(SystemExit) as caught:
                run_main(runner, '--label', 'winp0-20261009-a', receipts=Path(tmp))
        self.assertEqual(caught.exception.code, 2)
        self.assertEqual(runner.ssh_scripts(), [])

    def test_the_archive_is_a_ref_or_the_worktree_and_the_script_deletes_nothing(self):
        for argv, expected in (((), ['git', 'archive', '--format=tar', 'HEAD']),
                               (('--ref', 'abc123'), ['git', 'archive', '--format=tar', 'abc123']),
                               (('--worktree',), None)):
            runner = FakeRun()
            self.go(runner, *argv)
            commands = [a for a, k in runner.calls]
            if expected:
                self.assertIn(expected, commands)
            else:
                self.assertTrue(any(a[:2] == ['bash', '-c'] and 'ls-files' in a[2] and 'exclude)dev-docs' in a[2] and 'COPYFILE_DISABLE=1' in a[2] for a in commands))
            for script in runner.ssh_scripts():
                for word in ('rm ', 'rmdir', 'rustup default', 'rustup install', 'setx', 'reg add', 'net '):
                    self.assertNotIn(word, script)

    def test_stage_and_shape_scripts(self):
        stage = gate.stage_script('winp0-20261009-a', None)
        self.assertIn('checkout-git2-rs.sh', stage)
        self.assertIn('prepare.py /e/gwz-tests/winp0-20261009-a/candidate', stage)
        self.assertNotIn('cp -a', stage)
        self.assertIn('cargo metadata --format-version 1 --manifest-path /e/gwz-tests/winp0-20261009-a/candidate/Cargo.toml', stage)
        warm = gate.stage_script('winp0-20261009-b', 'winp0-20261009-a')
        self.assertIn('cp -a /e/gwz-tests/winp0-20261009-a/target /e/gwz-tests/winp0-20261009-b/target', warm)
        shape = gate.shape_script('winp0-20261009-a', '--cfg gwz_transport_candidate', True, 'transport')
        self.assertIn('RUSTFLAGS="--cfg gwz_transport_candidate"', shape)
        self.assertIn('--lib --profile test', shape)
        self.assertIn('RUSTUP_TOOLCHAIN=1.95.0', shape)
        self.assertNotIn('--profile', gate.shape_script('x-20261009', '', False, 'ordinary'))

    def test_the_cache_label_is_validated(self):
        with self.assertRaises(SystemExit):
            self.go(FakeRun(), '--cache-from', '../etc')


if __name__ == '__main__':
    unittest.main()
