"""Tests for the Windows compile gate script (GwzTransportWindowsParityPlan.md, step 0.4). No host is used."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest

spec = importlib.util.spec_from_file_location('windows_lane_check', Path(__file__).with_name('windows_lane_check.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
parity = gate.parity


HOSTED_PASS = (
    'HOSTED_LIKE_TEMP C:\\Users\\gianni\\AppData\\Local\\Temp\\WINP0-~1\n'
    'HOSTED_LIKE_GIT git version 2.55.0.windows.3\n'
    'HOSTED_LIKE_SSHD OpenSSH_for_Windows_9.8p2\n'
    'HOSTED_LIKE_LIBRARY passed=3433\n'
    'HOSTED_LIKE_RESULT passed=3501 failed=0\n')
HOSTED_FAIL = (HOSTED_PASS.replace('failed=0', 'failed=2')
               + 'HOSTED_LIKE_FAILED git::endpoint::ssh_tests::background_close::a\n'
               + 'HOSTED_LIKE_FAILED transport_host::ssh_helper_projection_tests::b\n')


class FakeRun:
    """Stands in for subprocess.run: records every call and answers by the command's first words."""

    def __init__(self, claim_exit=0, shape_exits=(0, 0), warnings=7, blocked_first=False, hosted_exit=0,
                 hosted_output=None):
        self.calls, self.claim_exit, self.shape_exits, self.warnings = [], claim_exit, list(shape_exits), warnings
        self.blocked_first = blocked_first
        self.hosted_exit = hosted_exit
        self.hosted_output = HOSTED_PASS if hosted_output is None else hosted_output

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
            if 'run_tests.py' in script:
                if self.blocked_first:
                    self.blocked_first = False
                    return subprocess.CompletedProcess(argv, 101, 'An Application Control policy has blocked this file. (os error 4551)\n', '')
                return subprocess.CompletedProcess(argv, self.hosted_exit, self.hosted_output, '')
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


class Refusals(unittest.TestCase):
    def test_a_refusal_prints_its_message_on_stderr_and_exits_2(self):
        # The script as run: a bad label is refused before any host call, and the reason reaches the person.
        done = subprocess.run([sys.executable, str(Path(gate.__file__)), '--label', 'Fix-20261014-a',
                               '--receipt-dir', 'unused'], capture_output=True, text=True, check=False)
        self.assertEqual(done.returncode, 2)
        self.assertIn("windows_lane_check: label 'Fix-20261014-a' must be lowercase letters", done.stderr)

    def test_run_returns_the_status_and_prints_for_a_refusal_and_returns_main_otherwise(self):
        self.assertEqual(gate.run(['--label', 'x', '--receipt-dir', 'unused']), 2)
        refused = gate.Refused('nope')
        self.assertEqual((refused.code, refused.message), (2, 'windows_lane_check: nope'))


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

    def test_the_two_shapes_are_the_ci_jobs(self):
        # Step 5.1 retired the qualification shape: the candidate on Windows is the transport switch alone.
        self.assertEqual([n for n, _ in gate.SHAPES], ['ordinary', 'transport'])
        workflow = (gate.CORE / '.github' / 'workflows' / 'transport-candidate.yml').read_text(encoding='utf-8')
        for _, flags in gate.SHAPES[1:]:
            self.assertIn(f'RUSTFLAGS: {flags}', workflow)
        self.assertEqual(gate.CANDIDATE, '--cfg gwz_transport_candidate')
        self.assertFalse(hasattr(gate, 'QUALIFICATION'))


class HostedLike(unittest.TestCase):
    """The mode that RUNS the candidate library suites under the hosted job's conditions (cifix's proposal)."""

    def go(self, runner, *argv):
        with tempfile.TemporaryDirectory() as tmp:
            code = run_main(runner, '--label', 'cifix2-20261013-a', *argv, receipts=Path(tmp))
            path = Path(tmp) / 'cifix2-20261013-a' / 'receipt.json'
            return code, json.loads(path.read_text(encoding='utf-8')) if path.exists() else None

    def test_the_mode_is_off_by_default_and_its_options_need_it(self):
        o = options('--label', 'a-20261009')
        self.assertEqual((o.hosted_like, o.git_root, o.hosted_like_filter, o.no_check_shapes),
                         (False, gate.DEFAULT_GIT_ROOT, None, False))
        for extra in (['--git-root', 'E:\\g'], ['--hosted-like-filter', 'x'], ['--no-check-shapes']):
            runner = FakeRun()
            with self.subTest(extra), self.assertRaises(SystemExit) as caught:
                self.go(runner, *extra)
            self.assertEqual(caught.exception.code, 2)
            self.assertEqual(runner.ssh_scripts(), [])

    def test_without_the_mode_no_suite_is_run(self):
        runner = FakeRun()
        code, receipt = self.go(runner)
        self.assertEqual(code, 0)
        self.assertFalse(any('run_tests.py' in s for s in runner.ssh_scripts()))
        self.assertNotIn('hosted-like', [s['shape'] for s in receipt['shapes']])

    def test_a_passing_run_adds_a_hosted_like_entry_with_the_suite_counts(self):
        runner = FakeRun()
        code, receipt = self.go(runner, '--hosted-like')
        self.assertEqual(code, 0)
        self.assertEqual([s['shape'] for s in receipt['shapes']], ['ordinary', 'transport', 'hosted-like'])
        entry = receipt['shapes'][-1]
        self.assertEqual((entry['exit'], entry['rustflags']), (0, gate.CANDIDATE))
        self.assertEqual(entry['tests'], {'passed': 3501, 'failed': 0, 'failed_names': [], 'filter': None})
        self.assertTrue(entry['temp'].endswith('WINP0-~1') and entry['git'].startswith('git version'))
        self.assertTrue(receipt['passed'])

    def test_failed_tests_fail_the_receipt_and_are_named_and_never_retried(self):
        runner = FakeRun(hosted_exit=101, hosted_output=HOSTED_FAIL)
        code, receipt = self.go(runner, '--hosted-like')
        self.assertEqual((code, receipt['passed']), (1, False))
        entry = receipt['shapes'][-1]
        self.assertEqual((entry['exit'], entry['tests']['failed']), (101, 2))
        self.assertEqual(entry['tests']['failed_names'][0], 'git::endpoint::ssh_tests::background_close::a')
        self.assertEqual(len(entry['attempts']), 1)
        self.assertEqual(len([s for s in runner.ssh_scripts() if 'run_tests.py' in s]), 1)

    def test_a_blocked_build_script_is_run_once_more_as_in_the_shapes(self):
        # The first attempt of the first call that can hit it (the shapes' cargo check) takes the block.
        runner = FakeRun(blocked_first=True)
        code, receipt = self.go(runner, '--hosted-like', '--no-check-shapes')
        self.assertEqual(code, 0)
        self.assertEqual([[a['exit'] for a in s['attempts']] for s in receipt['shapes']], [[101, 0]])

    def test_no_check_shapes_runs_only_the_suite(self):
        runner = FakeRun()
        code, receipt = self.go(runner, '--hosted-like', '--no-check-shapes')
        self.assertEqual([s['shape'] for s in receipt['shapes']], ['hosted-like'])
        self.assertFalse(any('cargo check' in s for s in runner.ssh_scripts()))

    def test_the_filter_is_recorded_and_reaches_the_suite_as_its_test_filter(self):
        """A bare name before `--` is one more libtest filter, ORed with each group's own: the fake-filesystem groups
        then ran real-filesystem tests and failed them on every host (2026-10-14-fixc). run_tests.py intersects."""
        runner = FakeRun()
        code, receipt = self.go(runner, '--hosted-like', '--hosted-like-filter', 'background_close')
        self.assertEqual(receipt['shapes'][-1]['tests']['filter'], 'background_close')
        script = [s for s in runner.ssh_scripts() if 'run_tests.py' in s][0]
        self.assertIn('--lib --test-filter background_close ', script)
        self.assertNotIn('--lib background_close ', script)

    def test_tests_that_need_what_the_host_lacks_are_named_gaps_and_do_not_fail_the_receipt(self):
        """The embedded-Python rows need a gwz-py checkout and taut-proto in the interpreter's site-packages; the mode
        installs neither (a host change). They fail there, are named as gaps, and are the only failures a receipt passes."""
        python = ['transport_host::message_embedding_tests::python_messages_close_while_delivery_is_paused']
        output = HOSTED_PASS.replace('failed=0', 'failed=1') + f'HOSTED_LIKE_FAILED {python[0]}\n'
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=output), '--hosted-like')
        entry = receipt['shapes'][-1]
        self.assertEqual((code, receipt['passed'], entry['exit'], entry['passed']), (0, True, 101, True))
        self.assertEqual((entry['gaps'], entry['failures']), (python, []))
        output += 'HOSTED_LIKE_FAILED git::endpoint::ssh_tests::background_close::a\n'
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=output), '--hosted-like')
        entry = receipt['shapes'][-1]
        self.assertEqual((code, receipt['passed'], entry['passed']), (1, False, False))
        self.assertEqual((entry['gaps'], entry['failures']), (python, ['git::endpoint::ssh_tests::background_close::a']))

    def test_the_protocol_target_rows_that_run_the_taut_generator_are_named_gaps_too(self):
        """tests/protocol.rs regenerates the checked-in sources with `python -m taut.cli`, which the hosted job installs
        (taut-proto) and the host does not; the four rows that need it fail there the same way the embedding rows do."""
        names = ['generated_checked_artifact_protocol_is_current', 'generated_protocol_is_current',
                 'log_addition_check_refuses_a_schema_that_declares_a_taut_option',
                 'log_addition_preserves_the_complete_pre_log_wire_projection']
        output = HOSTED_PASS.replace('failed=0', 'failed=4') + ''.join(f'HOSTED_LIKE_FAILED {n}\n' for n in names)
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=output), '--hosted-like')
        entry = receipt['shapes'][-1]
        self.assertEqual((code, receipt['passed'], entry['gaps'], entry['failures']), (0, True, names, []))
        # a test that only starts with a gap's name is not one
        output = output.replace('failed=4', 'failed=5') + 'HOSTED_LIKE_FAILED generated_protocol_is_current_for_x\n'
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=output), '--hosted-like')
        self.assertEqual((code, receipt['shapes'][-1]['failures']), (1, ['generated_protocol_is_current_for_x']))

    def test_the_protocol_rows_are_gaps_only_while_the_host_has_no_taut_proto(self):
        """Plan step 2.2 review, S-P2-1: with taut-proto installed the three rows must pass, so a row failing there (a CRLF
        export the pin refused, say) is a failure, not a gap; and a receipt records what the probe and the checkout showed."""
        names = ['generated_protocol_is_current',
                 'log_addition_check_refuses_a_schema_that_declares_a_taut_option',
                 'log_addition_preserves_the_complete_pre_log_wire_projection']
        failed = HOSTED_PASS.replace('failed=0', 'failed=3') + ''.join(f'HOSTED_LIKE_FAILED {n}\n' for n in names)
        absent = failed + 'HOSTED_LIKE_TAUT missing\n'
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=absent), '--hosted-like')
        self.assertEqual((code, receipt['shapes'][-1]['gaps'], receipt['shapes'][-1]['failures']), (0, names, []))
        present = failed + 'HOSTED_LIKE_TAUT 0.10.0\nHOSTED_LIKE_OWNER_EOL crlf\n'
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=present), '--hosted-like', '--taut-proto', '0.10.0')
        entry = receipt['shapes'][-1]
        self.assertEqual((code, receipt['passed'], entry['gaps'], entry['failures']), (1, False, [], names))
        self.assertEqual((entry['taut_proto'], entry['owner_export_line_endings']), ('0.10.0', 'crlf'))

    def test_an_embedded_python_row_is_a_gap_only_when_its_own_text_says_the_tooling_is_missing(self):
        row = 'transport_host::message_embedding_tests::python_messages_close_while_delivery_is_paused'
        base = HOSTED_PASS.replace('failed=0', 'failed=1') + f'HOSTED_LIKE_FAILED {row}\n'
        for text, gap in (("RuntimeError: codec input is missing: E:/x/gwz-py/src", True),
                          ("ModuleNotFoundError: No module named 'taut'", True),
                          ("assertion `left == right` failed: the bytes differ", False)):
            with self.subTest(text=text):
                output = base + f'HOSTED_LIKE_FAILURE_TEXT {row}\t{text}\n'
                code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=output), '--hosted-like')
                entry = receipt['shapes'][-1]
                self.assertEqual((code, entry['gaps'] == [row]), (0 if gap else 1, gap))
                self.assertEqual(entry['failure_texts'][row], text)

    def test_the_owner_export_line_ending_probe_finds_a_carriage_return(self):
        """The probe that records whether the checkout of gwz-transport has CRLF endings must detect one: the first version
        escaped the carriage return twice, searched for a backslash and an r, and recorded `lf` for a CRLF file."""
        import shutil
        import tempfile
        script = gate.hosted_like_script('a-20261025-a', gate.DEFAULT_GIT_ROOT, None)
        line = [l for l in script.splitlines() if 'printf' in l and 'eol=' in l][0]
        bash = shutil.which('bash')
        if bash is None:
            self.skipTest('no bash to run the probe')
        with tempfile.TemporaryDirectory() as tmp:
            export = Path(tmp) / 'ws' / 'gwz-transport' / 'protocol' / 'transport.ir.json'
            export.parent.mkdir(parents=True)
            for body, expected in ((b'{\r\n}\r\n', 'crlf'), (b'{\n}\n', 'lf')):
                export.write_bytes(body)
                probe = line.replace('/e/gwz-tests/a-20261025-a', tmp)
                result = subprocess.run([bash, '-c', f'{probe}; echo $eol'], capture_output=True, text=True, check=True)
                self.assertEqual(result.stdout.strip(), expected)

    def test_taut_proto_goes_in_a_venv_under_the_runs_directory_and_needs_the_mode(self):
        script = gate.hosted_like_script('a-20261025-a', gate.DEFAULT_GIT_ROOT, None, '0.10.0')
        self.assertIn('python -m venv /e/gwz-tests/a-20261025-a/taut-venv', script)
        self.assertIn('"taut-proto==0.10.0"', script)
        self.assertIn('TAUT_PYTHON', script)
        self.assertNotIn('taut-venv', gate.hosted_like_script('a-20261025-a', gate.DEFAULT_GIT_ROOT, None))
        self.assertIn('HOSTED_LIKE_TAUT', gate.hosted_like_script('a-20261025-a', gate.DEFAULT_GIT_ROOT, None))
        with self.assertRaises(SystemExit) as caught:
            self.go(FakeRun(), '--taut-proto', '0.10.0')
        self.assertEqual(caught.exception.code, 2)
        with self.assertRaises(SystemExit) as caught:
            self.go(FakeRun(), '--hosted-like', '--taut-proto', 'latest')
        self.assertEqual(caught.exception.code, 2)

    def test_an_integration_step_that_died_is_a_failure_even_beside_named_gaps(self):
        """The integration step exits nonzero when a gap row fails, as the suite does; what tells that from a step that
        never ran its targets (a build error, a binary that could not load) is that every target reported a result."""
        python = 'transport_host::message_embedding_tests::python_messages_close_while_delivery_is_paused'
        base = HOSTED_PASS.replace('failed=0', 'failed=1') + f'HOSTED_LIKE_FAILED {python}\n'
        targets = len(gate.INTEGRATION_TARGETS)
        for exit_code, results, passes in ((101, 0, False), (101, targets - 1, False), (101, targets, True), (0, targets, True)):
            output = base + f'HOSTED_LIKE_INTEGRATION_EXIT {exit_code}\nHOSTED_LIKE_INTEGRATION_RESULTS {results}\n'
            with self.subTest(exit_code=exit_code, results=results):
                code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=output), '--hosted-like')
                entry = receipt['shapes'][-1]
                self.assertEqual((code, entry['passed']), (0 if passes else 1, passes))
                self.assertEqual((entry['integration_exit'], entry['integration_results']), (exit_code, results))

    def test_the_script_prints_the_integration_exit_and_fails_when_either_failed(self):
        script = gate.hosted_like_script('cifix2-20261013-a', gate.DEFAULT_GIT_ROOT, None)
        self.assertIn('echo "HOSTED_LIKE_INTEGRATION_EXIT $integration_status"', script)
        # the results of the step alone: the log's lines from where the step began
        self.assertIn('tail -n +$((before + 1)) /e/gwz-tests/cifix2-20261013-a/logs/hosted-like.log | grep -a -c', script)
        self.assertIn('echo "HOSTED_LIKE_INTEGRATION_RESULTS $results"', script)
        self.assertIn('if [ "$status" -eq 0 ]; then status=$integration_status; fi', script)

    def test_a_failed_suite_with_no_named_test_is_a_failure(self):
        """A suite that died (exit 101, no test named: the first run's test binary that could not load) never passes."""
        code, receipt = self.go(FakeRun(hosted_exit=101, hosted_output=HOSTED_PASS), '--hosted-like')
        self.assertEqual((code, receipt['shapes'][-1]['passed']), (1, False))

    def test_a_suite_that_ran_no_library_tests_is_a_failure_whatever_the_integration_targets_passed(self):
        """Step 5.2's receipt printed PASS with 68 passed, all from the integration targets: the library binary never ran."""
        for output in (HOSTED_PASS.replace('HOSTED_LIKE_LIBRARY passed=3433', 'HOSTED_LIKE_LIBRARY passed=0'),
                       HOSTED_PASS.replace('HOSTED_LIKE_LIBRARY passed=3433\n', '')):
            code, receipt = self.go(FakeRun(hosted_output=output.replace('passed=3501', 'passed=68')), '--hosted-like')
            self.assertEqual((code, receipt['shapes'][-1]['passed'], receipt['passed']), (1, False, False))
        self.assertIn('echo "HOSTED_LIKE_LIBRARY passed=$library"', gate.hosted_like_script('a-20261025', gate.DEFAULT_GIT_ROOT, None))

    def test_the_filter_must_be_a_test_path_fragment(self):
        for bad in ('a b', 'a;b', '$(x)', '', '"x"', '--exact'):
            with self.subTest(bad), self.assertRaises(SystemExit) as caught:
                self.go(FakeRun(), '--hosted-like', '--hosted-like-filter', bad)
            self.assertEqual(caught.exception.code, 2)

    def test_the_git_root_is_a_windows_directory_and_goes_first_on_path(self):
        script = gate.hosted_like_script('cifix2-20261013-a', 'E:\\gwz-tests\\git-2.56', None)
        self.assertIn("GIT_ROOT='E:\\gwz-tests\\git-2.56'", script)
        self.assertIn('export PATH="$GIT_UNIX/cmd:$PATH"', script)
        # `cygpath -u` answers "/" for a path with a space on the host (the first run put "\\cmd" first on PATH and
        # every test binary died at load, exit 0xc00000be): the conversion is spelled out, and read back.
        self.assertNotIn('cygpath -u "$GIT_ROOT"', script)
        self.assertIn("sed -E 's#\\\\#/#g; s#^([A-Za-z]):#\\L\\1#'", script)
        self.assertIn('HOSTED_LIKE_GIT $(command -v git)', script)
        self.assertIn('GWZ_TEST_SH="$GIT_ROOT\\usr\\bin\\sh.exe"', script)
        for bad in ('x;y', 'a b"c', '$(x)', 'C:\\a`b'):
            with self.subTest(bad), self.assertRaises(SystemExit):
                gate.check_git_root(bad)
        self.assertEqual(gate.check_git_root('C:\\Program Files\\Git'), 'C:\\Program Files\\Git')

    def test_the_script_makes_the_hosted_conditions(self):
        script = gate.hosted_like_script('cifix2-20261013-a', gate.DEFAULT_GIT_ROOT, None)
        d = '/e/gwz-tests/cifix2-20261013-a'
        # a short TEMP, refused when the host would not spell it short
        self.assertIn('cygpath -d', script)
        self.assertIn('HOSTED_LIKE_REFUSED', script)
        self.assertIn('export TEMP TMP', script)
        # the portable sshd of the job, checked, inside the run's directory; the real backend; Git's sh
        self.assertIn(gate.OPENSSH_SHA256, script)
        self.assertIn(gate.OPENSSH_URL, script)
        self.assertIn(f'{d}/openssh', script)
        self.assertIn('GWZ_TEST_SSHD="$(cygpath -w', script)
        self.assertIn('GWZ_TEST_GIT=real', script)
        self.assertIn('GWZ_TEST_SH=', script)
        self.assertIn(f'RUSTFLAGS="{gate.CANDIDATE}"', script)
        self.assertIn('RUSTUP_TOOLCHAIN=1.95.0', script)
        self.assertIn(f'CARGO_TARGET_DIR={d}/target-validator', script)
        self.assertIn(f'{gate.HOSTED_SUITE} --manifest-path={d}/candidate/Cargo.toml --target-dir={d}/target-test', script)
        for word in ('rm ', 'rmdir', 'rustup default', 'rustup install', 'setx', 'reg add', 'net ', 'taskkill', 'pkill', 'killall'):
            self.assertNotIn(word, script)

    def test_the_temp_directory_has_a_short_spelling_and_a_short_long_one(self):
        """The long name is nine characters (so Windows keeps an 8.3 spelling of it) and no longer: the suite builds paths
        near MAX_PATH from TEMP, and the first run, with the label in the name, failed 40 tests as "path too long"."""
        name = gate.temp_directory_name('cifix2-20261013-a')
        self.assertEqual((len(name), name[:2]), (9, 'hl'))
        self.assertNotEqual(name, gate.temp_directory_name('cifix2-20261013-b'))
        self.assertEqual(name, gate.temp_directory_name('cifix2-20261013-a'))
        script = gate.hosted_like_script('cifix2-20261013-a', gate.DEFAULT_GIT_ROOT, None)
        self.assertIn(f'/Temp/{name}"', script)
        self.assertNotIn('hosted-like-temp', script)

    def test_the_warm_cache_also_carries_the_suite_target_directories(self):
        stage = gate.stage_script('cifix2-20261013-b', 'cifix2-20261013-a')
        for name in ('target', 'target-test', 'target-validator'):
            self.assertIn(f'/e/gwz-tests/cifix2-20261013-a/{name} /e/gwz-tests/cifix2-20261013-b/{name}', stage)

    def test_the_job_it_mirrors_is_the_workflows(self):
        workflow = (gate.CORE / '.github' / 'workflows' / 'transport-candidate.yml').read_text(encoding='utf-8')
        self.assertIn(f'OPENSSH_RELEASE: {gate.OPENSSH_RELEASE}', workflow)
        self.assertIn(f'OPENSSH_SHA256: {gate.OPENSSH_SHA256}', workflow)
        suites = workflow.split('Run the candidate build\'s library suites', 1)[1]
        self.assertIn(f'RUSTFLAGS: {gate.CANDIDATE}', suites)
        # the job's command, but for the checkouts the host does not have (gwz-py) and its own paths
        self.assertIn(gate.HOSTED_SUITE.replace(' --skip-cfg-sibling gwz-py', ''), suites)

    def test_the_integration_targets_run_after_the_suite_and_are_the_workflows(self):
        """Step 5.4: the hosted job's "Integration targets" step is part of the mode, so a target that cannot run on
        Windows (publish_workflow read its files through `..`) shows in a receipt and not first on a hosted runner."""
        import re
        workflow = (gate.CORE / '.github' / 'workflows' / 'transport-candidate.yml').read_text(encoding='utf-8')
        step = workflow.split('- name: Integration targets', 1)[1].split('\n      - name:', 1)[0]
        self.assertEqual(list(gate.INTEGRATION_TARGETS), re.findall(r'--test +([\w-]+)', step))
        self.assertIn('publish_workflow', gate.INTEGRATION_TARGETS)
        d = '/e/gwz-tests/cifix2-20261013-a'
        script = gate.hosted_like_script('cifix2-20261013-a', gate.DEFAULT_GIT_ROOT, None)
        tests = ' '.join(f'--test {name}' for name in gate.INTEGRATION_TARGETS)
        command = (f'cargo test --locked --no-fail-fast --manifest-path {d}/candidate/Cargo.toml '
                   f'--target-dir {d}/target-test {tests}')
        self.assertIn(command, script)
        self.assertLess(script.index(gate.HOSTED_SUITE), script.index(command))
        # one log, so the counts and the failed names cover both; the run fails when either failed
        self.assertIn(f'>> {d}/logs/hosted-like.log 2>&1', script[script.index(command):])
        self.assertIn('GWZ_TEST_GIT=real', script[:script.index(command)])

    def test_a_filtered_run_leaves_the_integration_targets_out(self):
        script = gate.hosted_like_script('cifix2-20261013-a', gate.DEFAULT_GIT_ROOT, 'background_close')
        self.assertNotIn('--test publish_workflow', script)

    def test_parse(self):
        parsed = gate.parse_hosted_like(HOSTED_FAIL, None)
        self.assertEqual((parsed['tests']['passed'], parsed['tests']['failed']), (3501, 2))
        self.assertEqual(len(parsed['tests']['failed_names']), 2)
        self.assertEqual(gate.parse_hosted_like('garbage', None)['tests'], {'passed': 0, 'failed': 0, 'failed_names': [], 'filter': None})


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
                         [('ordinary', 0, 7), ('transport', 0, 7)])
        self.assertTrue(receipt['passed'] and receipt['total_seconds'] > 0 and receipt['stage_seconds'] > 0)
        self.assertEqual((receipt['label'], receipt['source'], receipt['toolchain']),
                         ('winp0-20261009-a', 'ref HEAD', 'rustc 1.95.0 (abc 2026-01-01)'))
        self.assertEqual(receipt['host_dir'], '/e/gwz-tests/winp0-20261009-a')

    def test_a_build_script_blocked_by_the_host_is_run_once_more_and_both_attempts_are_recorded(self):
        code, receipt = self.go(FakeRun(blocked_first=True))
        self.assertEqual(code, 0)
        self.assertEqual([[a['exit'] for a in s['attempts']] for s in receipt['shapes']], [[101, 0], [0]])

    def test_a_genuine_failure_is_not_retried(self):
        runner = FakeRun(shape_exits=(101, 0))
        code, receipt = self.go(runner)
        self.assertEqual((code, [len(s['attempts']) for s in receipt['shapes']]), (1, [1, 1]))

    def test_a_failing_shape_fails_the_run_and_the_receipt(self):
        code, receipt = self.go(FakeRun(shape_exits=(0, 101)))
        self.assertEqual((code, receipt['passed']), (1, False))
        self.assertEqual([s['exit'] for s in receipt['shapes']], [0, 101])

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


LIBTEST_LOG = (
    'running 4 tests\n'
    'test a::fast ... ok\n'
    'test a::slow_but_finishes has been running for over 60 seconds\n'
    'test a::hung_one has been running for over 60 seconds\r\n'
    'test b::also_hung has been running for over 60 seconds\n'
    'test a::slow_but_finishes ... ok\n'
    'test c::failed_late has been running for over 60 seconds\n'
    'test c::failed_late ... FAILED\n'
    'test d::skipped has been running for over 60 seconds\n'
    'test d::skipped ... ignored, reason\n'
    'test doc::src/lib.rs - thing (line 12) has been running for over 120 seconds\n')

POLL_MARK, EVIDENCE_MARK, KILL_MARK, VERIFY_MARK = ('# watchdog-poll', '# watchdog-evidence', '# watchdog-kill', '# watchdog-verify')
STARTED = '2026-10-30T01:00:00.0000000Z'


def proc(pid, ppid, image, cmd='', cpu=1.5, working_set=9000000):
    return json.dumps({'pid': pid, 'ppid': ppid, 'image': image, 'cmd': cmd or image, 'cpu_seconds': cpu,
                       'started': STARTED, 'working_set': working_set})


def evidence_output(valid=True, root=4242, children=((4300, 4242, 'cargo.exe'), (4311, 4300, 'rustc.exe'))):
    lines = ['WATCHDOG_ROOT ' + json.dumps({'pid': root, 'started': STARTED, 'pidfile_utc': STARTED, 'valid': valid,
                                            'why': '' if valid else 'the process is not the one that wrote the pid file'}),
             'WATCHDOG_PROC ' + proc(root, 100, 'bash.exe', 'bash.exe -s')]
    lines += ['WATCHDOG_PROC ' + proc(pid, ppid, image) for pid, ppid, image in children]
    lines += ['WATCHDOG_LOAD ' + json.dumps({'cpu_percent': 97, 'logical_processors': 16, 'memory_free_mb': 2048}),
              'WATCHDOG_TESTS_BEGIN', *LIBTEST_LOG.replace('\r', '').splitlines(), 'WATCHDOG_TESTS_END',
              'WATCHDOG_TAIL_BEGIN', 'last line one', 'last line two', 'WATCHDOG_TAIL_END', 'WATCHDOG_LOGSIZE 1234']
    return '\n'.join(lines) + '\n'


class ManualClock:
    """Time moves only when the watchdog waits, so a deadline is reached without anyone sleeping."""

    def __init__(self):
        self.t = 1000.0

    def __call__(self):
        return self.t

    def wait(self, done, seconds):
        if done.wait(0.05):
            return True
        self.t += seconds
        return False


class HangingRun(FakeRun):
    """A host whose `hang` phase never returns until its process tree is killed (a taskkill script arrives)."""

    def __init__(self, hang='hosted-like', sizes=(100,), evidence=None, survivors=(), **kwargs):
        super().__init__(**kwargs)
        self.hang, self.sizes, self.survivors = hang, list(sizes), [list(s) for s in survivors]
        self.evidence = evidence_output() if evidence is None else evidence
        self.released = threading.Event()
        self.polls, self.kills, self.verifies, self.evidence_scripts, self.stored = [], [], [], [], []

    def __call__(self, argv, **kwargs):
        script = kwargs.get('input', '')
        if argv[0] == 'ssh' and 'bash' in argv and isinstance(script, str):
            if script.startswith(POLL_MARK):
                self.polls.append(script)
                size = self.sizes.pop(0) if len(self.sizes) > 1 else self.sizes[0]
                self.calls.append((list(argv), kwargs))
                return subprocess.CompletedProcess(argv, 0, f'WATCHDOG_SIZE {size}\n', '')
            if script.startswith(EVIDENCE_MARK):
                self.calls.append((list(argv), kwargs))
                self.evidence_scripts.append(script)
                return subprocess.CompletedProcess(argv, 0, self.evidence, '')
            if script.startswith(KILL_MARK):
                self.calls.append((list(argv), kwargs))
                self.kills.append(script)
                self.released.set()
                return subprocess.CompletedProcess(argv, 0, 'WATCHDOG_KILL taskkill rc=0\n', '')
            if script.startswith(VERIFY_MARK):
                self.calls.append((list(argv), kwargs))
                self.verifies.append(script)
                alive = self.survivors.pop(0) if self.survivors else []
                out = ''.join('WATCHDOG_ALIVE ' + proc(pid, 1, 'rustc.exe') + '\n' for pid in alive)
                if alive and 'taskkill' in script:
                    self.released.set()
                return subprocess.CompletedProcess(argv, 0, out + 'WATCHDOG_ALIVE_DONE\n', '')
            if 'cat > /e/gwz-tests/' in script and 'watchdog-' in script:
                self.calls.append((list(argv), kwargs))
                self.stored.append(script)
                return subprocess.CompletedProcess(argv, 0, '', '')
            if f'/logs/{self.hang}.log' in script and 'mkdir /e/' not in script[:12]:
                self.calls.append((list(argv), kwargs))
                self.released.wait(10)
                return subprocess.CompletedProcess(argv, 255, 'HOSTED_LIKE_TEMP C:\\x\\HL1234~1\n', '')
        return super().__call__(argv, **kwargs)


def watched_host(runner):
    clock = ManualClock()
    return gate.Host(run=runner, clock=clock, wait=clock.wait)


class LibtestStalls(unittest.TestCase):
    def test_a_test_reported_running_for_over_60_seconds_that_never_finished_is_stalled(self):
        self.assertEqual(gate.stalled_tests(LIBTEST_LOG),
                         ['a::hung_one', 'b::also_hung', 'doc::src/lib.rs - thing (line 12)'])

    def test_a_healthy_log_has_none_and_garbage_has_none(self):
        self.assertEqual(gate.stalled_tests('test a ... ok\ntest b ... ok\n'), [])
        self.assertEqual(gate.stalled_tests(''), [])
        self.assertEqual(gate.stalled_tests('Compiling gwz-core v0.1.0\n'), [])

    def test_a_test_that_is_reported_slow_twice_and_finishes_once_is_not_stalled(self):
        text = ('test x has been running for over 60 seconds\ntest x has been running for over 120 seconds\n'
                'test x ... ok\n')
        self.assertEqual(gate.stalled_tests(text), [])
        self.assertEqual(gate.stalled_tests(text.replace('test x ... ok\n', '')), ['x'])


class WatchdogOptions(unittest.TestCase):
    def test_defaults_are_generous_against_the_recorded_history_and_far_below_hours(self):
        limits = options('--label', 'a-20261009').watchdog
        self.assertEqual(sorted(gate.DEFAULT_DEADLINES), ['hosted-like', 'ordinary', 'stage', 'transport'])
        # recorded maxima (247 receipts): ordinary 117 s, transport 109 s, hosted-like ~1480 s (a failing run),
        # healthy cold hosted-like 900 s; staging 2905 s once.
        self.assertGreaterEqual(limits.deadline['ordinary'], 6 * 117)
        self.assertGreaterEqual(limits.deadline['transport'], 6 * 109)
        self.assertGreaterEqual(limits.deadline['hosted-like'], 1800)
        self.assertGreaterEqual(limits.deadline['stage'], 2905)
        for phase, seconds in limits.deadline.items():
            self.assertLessEqual(seconds, 3600, phase)
        self.assertEqual(sorted(limits.quiet), ['hosted-like', 'ordinary', 'transport'])
        for phase, seconds in limits.quiet.items():
            self.assertLess(seconds, limits.deadline[phase], phase)
        self.assertGreater(limits.quiet['hosted-like'], 900)  # longer than the slowest healthy cold phase
        self.assertGreaterEqual(limits.poll, 10)

    def test_flags_override_one_phase_at_a_time(self):
        limits = options('--label', 'a-20261009', '--deadline', 'hosted-like=60', '--no-progress', 'ordinary=30',
                         '--deadline', 'stage=100', '--watchdog-poll', '5').watchdog
        self.assertEqual((limits.deadline['hosted-like'], limits.quiet['ordinary'], limits.deadline['stage'], limits.poll),
                         (60, 30, 100, 5))
        self.assertEqual(limits.deadline['ordinary'], gate.DEFAULT_DEADLINES['ordinary'])

    def test_bad_values_are_refused_before_any_host_call(self):
        for argv in (['--deadline', 'nonsense=60'], ['--deadline', 'hosted-like'], ['--deadline', 'hosted-like=abc'],
                     ['--deadline', 'hosted-like=-5'], ['--no-progress', 'stage=60'], ['--watchdog-poll', '0']):
            runner = FakeRun()
            with self.subTest(argv), self.assertRaises(SystemExit) as caught:
                with tempfile.TemporaryDirectory() as tmp:
                    run_main(runner, '--label', 'a-20261009', *argv, receipts=Path(tmp))
            self.assertEqual(caught.exception.code, 2)
            self.assertEqual(runner.ssh_scripts(), [])

    def test_the_exit_code_is_documented_in_the_docstring_and_the_usage_text(self):
        self.assertEqual(gate.EXIT_TIMEOUT, 124)
        self.assertNotIn(gate.EXIT_TIMEOUT, (0, 1, 2, 3))
        self.assertIn('124', gate.__doc__)
        self.assertIn('--deadline', gate.__doc__)
        usage = gate.parser().format_help()
        self.assertIn('124', usage)
        self.assertIn('--no-progress', usage)
        self.assertIn('--watchdog-poll', usage)


class WatchdogScripts(unittest.TestCase):
    def test_every_phase_records_the_windows_pid_of_its_own_root_process(self):
        scripts = {'stage': gate.stage_script('a-20261009', None),
                   'ordinary': gate.shape_script('a-20261009', '', False, 'ordinary'),
                   'transport': gate.shape_script('a-20261009', gate.CANDIDATE, False, 'transport'),
                   'hosted-like': gate.hosted_like_script('a-20261009', gate.DEFAULT_GIT_ROOT, None)}
        for name, script in scripts.items():
            with self.subTest(name):
                self.assertIn(f'echo "$(cat /proc/$$/winpid) $$" > /e/gwz-tests/a-20261009/logs/{name}.pid', script)

    def test_the_kill_script_kills_one_pid_tree_and_only_if_it_is_still_the_recorded_process(self):
        script = gate.kill_script(4242, STARTED)
        self.assertTrue(script.startswith(KILL_MARK))
        self.assertIn('taskkill.exe /T /F /PID', script)
        self.assertIn('4242', script)
        self.assertIn(STARTED, script)
        for word in ('/IM', '-Name', 'Stop-Process', 'pkill', 'killall', 'wmic'):
            self.assertNotIn(word, script)

    def test_the_evidence_script_walks_down_from_the_pid_file_and_reports_load(self):
        script = gate.evidence_script('a-20261009', 'hosted-like')
        self.assertTrue(script.startswith(EVIDENCE_MARK))
        for word in ('Win32_Process', 'ParentProcessId', 'CommandLine', 'WorkingSetSize', 'LoadPercentage',
                     'logs\\hosted-like.pid', 'running for over', 'tail -n', 'ps -W', 'WATCHDOG_EXTRA'):
            self.assertIn(word, script)
        self.assertNotIn('taskkill', script)

    def test_the_log_poll_names_the_phases_log_and_the_stage_has_none(self):
        self.assertIn('/e/gwz-tests/a-20261009/logs/ordinary.log', gate.poll_script('a-20261009', 'ordinary'))
        self.assertNotIn('.log', gate.poll_script('a-20261009', 'stage'))


class WatchdogAbort(unittest.TestCase):
    def go(self, runner, *argv, label='wd-20261030-a'):
        self.addCleanup(runner.released.set)
        with tempfile.TemporaryDirectory() as tmp:
            host = watched_host(runner)
            code = gate.main(['--receipt-dir', tmp, '--label', label, *argv], host=host, core=Path('/nonexistent-core'))
            path = Path(tmp) / label / 'receipt.json'
            receipt = json.loads(path.read_text(encoding='utf-8')) if path.exists() else None
            files = {p.name: p.read_text(encoding='utf-8') for p in (Path(tmp) / label).glob('*')} if path.exists() else {}
            return code, receipt, files, host

    def test_the_wall_deadline_aborts_the_phase_and_fails_with_its_own_exit_code(self):
        runner = HangingRun(sizes=(100, 200, 300, 400, 500, 600, 700))
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300',
                                             '--watchdog-poll', '100')
        self.assertEqual(code, gate.EXIT_TIMEOUT)
        self.assertEqual((receipt['result'], receipt['passed']), ('TIMEOUT', False))
        abort = receipt['abort']
        self.assertEqual((abort['phase'], abort['reason'], abort['limit_seconds']), ('hosted-like', 'deadline', 300))
        self.assertGreaterEqual(abort['elapsed_seconds'], 300)
        self.assertLess(abort['elapsed_seconds'], 500)
        self.assertEqual(len(runner.kills), 1)

    def test_no_progress_aborts_when_the_log_stops_growing_long_before_the_wall_deadline(self):
        runner = HangingRun(sizes=(100, 200, 300, 300))
        code, receipt, files, host = self.go(runner, '--hosted-like', '--no-progress', 'hosted-like=250',
                                             '--watchdog-poll', '100')
        self.assertEqual(code, gate.EXIT_TIMEOUT)
        abort = receipt['abort']
        self.assertEqual((abort['phase'], abort['reason'], abort['limit_seconds']), ('hosted-like', 'no-progress', 250))
        self.assertLess(abort['elapsed_seconds'], gate.DEFAULT_DEADLINES['hosted-like'])
        # the log last grew at the poll that saw 300 (t=300); 250 s later is t=550
        self.assertGreaterEqual(abort['elapsed_seconds'], 550)
        self.assertLess(abort['elapsed_seconds'], 700)

    def test_a_log_that_keeps_growing_is_not_a_no_progress_abort(self):
        runner = HangingRun(sizes=range(100, 100000, 100))
        code, receipt, files, host = self.go(runner, '--hosted-like', '--no-progress', 'hosted-like=250',
                                             '--deadline', 'hosted-like=1000', '--watchdog-poll', '100')
        self.assertEqual(receipt['abort']['reason'], 'deadline')

    def test_an_earlier_phase_that_passed_stays_in_the_receipt_and_later_phases_do_not_run(self):
        runner = HangingRun(hang='transport')
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'transport=200',
                                             '--watchdog-poll', '100')
        self.assertEqual(code, gate.EXIT_TIMEOUT)
        self.assertEqual([(s['shape'], s['exit']) for s in receipt['shapes'][:1]], [('ordinary', 0)])
        self.assertEqual(receipt['shapes'][-1]['shape'], 'transport')
        self.assertFalse(receipt['shapes'][-1]['passed'])
        self.assertTrue(receipt['shapes'][-1]['aborted'])
        self.assertEqual(receipt['abort']['phase'], 'transport')
        self.assertFalse(any('run_tests.py' in s for s in runner.ssh_scripts()))

    def test_the_receipt_summarizes_the_evidence_and_the_files_are_saved_with_it(self):
        runner = HangingRun()
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        abort = receipt['abort']
        self.assertEqual(abort['stalled_tests'], ['a::hung_one', 'b::also_hung', 'doc::src/lib.rs - thing (line 12)'])
        self.assertEqual(sorted(abort['evidence']), ['host', 'local', 'log_tail'])
        self.assertEqual(abort['evidence']['host'], '/e/gwz-tests/wd-20261030-a/logs/watchdog-hosted-like.json')
        self.assertIn('watchdog-hosted-like.json', files)
        self.assertIn('watchdog-hosted-like-log-tail.txt', files)
        self.assertIn('last line two', files['watchdog-hosted-like-log-tail.txt'])
        evidence = json.loads(files['watchdog-hosted-like.json'])
        self.assertEqual((evidence['phase'], evidence['reason']), ('hosted-like', 'deadline'))
        self.assertEqual(evidence['log']['tail'][-1], 'last line two')
        self.assertEqual(evidence['host_load']['cpu_percent'], 97)
        self.assertEqual([p['pid'] for p in evidence['process_tree']], [4242, 4300, 4311])
        self.assertEqual(sorted(evidence['process_tree'][1]),
                         ['cmd', 'cpu_seconds', 'image', 'pid', 'ppid', 'started', 'working_set'])
        self.assertEqual(evidence['stalled_tests'], abort['stalled_tests'])
        self.assertTrue(any('watchdog-hosted-like.json' in s for s in runner.stored))
        # the receipt on the host is written as on the normal path
        self.assertTrue(any('receipt.json' in s for s in runner.ssh_scripts()))

    def test_evidence_is_captured_before_anything_is_killed(self):
        runner = HangingRun()
        self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        order = [k['input'].split('\n', 1)[0] for a, k in runner.calls if 'input' in k and isinstance(k['input'], str)]
        self.assertLess(order.index(EVIDENCE_MARK), order.index(KILL_MARK))

    def test_only_the_recorded_pid_tree_is_killed_and_never_by_image_name(self):
        runner = HangingRun()
        self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        self.assertEqual(len(runner.kills), 1)
        for script in runner.ssh_scripts():
            for word in ('/IM', '-Name', 'pkill', 'killall', 'wmic', 'Stop-Process'):
                self.assertNotIn(word, script)
        self.assertIn(STARTED, runner.kills[0])

    def test_after_the_kill_the_recorded_pids_are_checked_and_the_receipt_says_they_ended(self):
        runner = HangingRun()
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        self.assertEqual(len(runner.verifies), 1)
        for pid in ('4242', '4300', '4311'):
            self.assertIn(pid, runner.verifies[0])
        self.assertIs(receipt['abort']['remote_processes_ended'], True)
        evidence = json.loads(files['watchdog-hosted-like.json'])
        self.assertEqual((evidence['kill']['survivors'], evidence['kill']['recorded_pids']), ([], [4242, 4300, 4311]))

    def test_a_survivor_is_killed_by_its_recorded_pid_and_one_that_stays_is_reported(self):
        runner = HangingRun(survivors=([4311], []))
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        self.assertIs(receipt['abort']['remote_processes_ended'], True)
        self.assertIn('taskkill.exe /F /PID', runner.verifies[1])
        self.assertIn('4311', runner.verifies[1])
        stubborn = HangingRun(survivors=([4311], [4311], [4311], [4311]))
        code, receipt, files, host = self.go(stubborn, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100',
                                             label='wd-20261030-b')
        self.assertEqual(code, gate.EXIT_TIMEOUT)
        self.assertIs(receipt['abort']['remote_processes_ended'], False)
        self.assertEqual(json.loads(files['watchdog-hosted-like.json'])['kill']['survivors'][0]['pid'], 4311)

    def test_a_pid_that_is_not_the_recorded_root_is_never_killed(self):
        runner = HangingRun(evidence=evidence_output(valid=False))
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        self.assertEqual(runner.kills, [])
        self.assertEqual(code, gate.EXIT_TIMEOUT)
        self.assertIsNone(receipt['abort']['remote_processes_ended'])
        self.assertIn('not the one', json.loads(files['watchdog-hosted-like.json'])['kill']['refused'])

    def test_a_build_phase_is_watched_too_and_the_stage_has_a_wall_deadline_only(self):
        runner = HangingRun(hang='ordinary')
        code, receipt, files, host = self.go(runner, '--no-progress', 'ordinary=250', '--watchdog-poll', '100')
        self.assertEqual((receipt['abort']['phase'], receipt['abort']['reason']), ('ordinary', 'no-progress'))
        self.assertEqual(receipt['shapes'][-1]['log'], '/e/gwz-tests/wd-20261030-a/logs/ordinary.log')

    def test_the_phase_script_has_ended_after_the_kill_and_a_refused_kill_leaves_it_running_and_says_so(self):
        runner = HangingRun()
        code, receipt, files, host = self.go(runner, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100')
        self.assertTrue(json.loads(files['watchdog-hosted-like.json'])['phase_script_ended'])
        self.assertIn('C:\\x\\HL1234~1', json.loads(files['watchdog-hosted-like.json'])['phase_output_so_far'])
        refused = HangingRun(evidence=evidence_output(valid=False))
        code, receipt, files, host = self.go(refused, '--hosted-like', '--deadline', 'hosted-like=300', '--watchdog-poll', '100',
                                             label='wd-20261030-c')
        self.assertFalse(json.loads(files['watchdog-hosted-like.json'])['phase_script_ended'])


class WatchdogLeavesNormalRunsAlone(unittest.TestCase):
    def go(self, runner, *argv):
        with tempfile.TemporaryDirectory() as tmp:
            host = gate.Host(run=runner, clock=Clock())
            code = gate.main(['--receipt-dir', tmp, '--label', 'wd-20261030-a', *argv], host=host,
                             core=Path('/nonexistent-core'))
            return code, json.loads((Path(tmp) / 'wd-20261030-a' / 'receipt.json').read_text(encoding='utf-8'))

    def test_a_passing_run_has_the_same_receipt_keys_and_no_abort_fields(self):
        code, receipt = self.go(FakeRun(), '--hosted-like')
        self.assertEqual(code, 0)
        self.assertEqual(list(receipt), ['label', 'host_dir', 'source', 'commit', 'cache_from', 'with_tests', 'toolchain',
                                         'started', 'stage_seconds', 'shapes', 'total_seconds', 'passed'])
        self.assertNotIn('abort', receipt)
        self.assertNotIn('result', receipt)
        self.assertEqual([sorted(s) for s in receipt['shapes'][:2]],
                         [['attempts', 'exit', 'log', 'rustflags', 'seconds', 'shape', 'warnings']] * 2)
        self.assertNotIn('aborted', receipt['shapes'][-1])

    def test_a_failing_run_still_exits_1_and_runs_no_evidence_script(self):
        runner = FakeRun(shape_exits=(0, 101))
        code, receipt = self.go(runner)
        self.assertEqual((code, receipt['passed']), (1, False))
        self.assertFalse(any(s.startswith(('# watchdog-evidence', '# watchdog-kill')) for s in runner.ssh_scripts()))

    def test_the_phase_runs_in_one_ssh_call_as_before_and_polling_is_only_for_the_log(self):
        runner = FakeRun()
        self.go(runner, '--hosted-like')
        phases = [s for s in runner.ssh_scripts() if 'cargo check' in s or 'run_tests.py' in s]
        self.assertEqual(len(phases), 3)


if __name__ == '__main__':
    unittest.main()
