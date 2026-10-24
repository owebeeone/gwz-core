"""Tests for the candidate switch inventory check.

TR2.12, rule (a) of dev-docs/GwzTransportReleasePlanAmendment-2.md §3.13.
"""
import contextlib
import importlib.util
import io
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('candidate_switches',
                                              Path(__file__).with_name('check_candidate_switches.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

T = 'gwz_transport_candidate'
RETIRED = 'gwz_session_candidate'
# The checker's mechanics are exercised with a second, synthetic switch: `gwz_session_candidate` is retired (plan step
# 2.2 of dev-docs/GwzTransportReleaseConvergencePlan.md), so the real checker knows one switch.
S = 'gwz_synthetic_candidate'


class TwoSwitches(unittest.TestCase):
    def setUp(self):
        for name, value in (('SWITCHES', (T, S)), ('RETIRED', ())):
            patcher = patch.object(checker, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
WORKFLOW = checker.ROOT / '.github' / 'workflows' / 'transport-candidate.yml'
DECLARATIONS = ''.join(f'println!("cargo:rustc-check-cfg=cfg({switch})");\n' for switch in (T, S))


class Sites(TwoSwitches):
    def test_a_site_is_a_cfg_or_cfg_attr_attribute_or_cfg_macro_that_names_a_switch(self):
        source = f'''#![cfg(any(unix, {S}))]
#[cfg({T})]
mod a {{}}
#[cfg_attr(all(unix, {S}), path = "b_unix.rs")]
mod b;
fn c() -> bool {{ cfg!(any({T}, {S})) }}
#[allow(clippy::needless_update, reason = "{T} adds fields")]
fn d() {{ let s = "#[cfg({S})]"; }} // #[cfg({S})]
/* #[cfg({T})] */
#[cfg(unix)]
mod e {{ #![cfg(not({S}))] }}
'''
        self.assertEqual(checker.sites(source), [(S, '(file)'), (T, 'mod a'), (S, 'mod b'), (T, 'fn c'), (S, 'fn c'),
                                                 (S, 'mod e')])

    def test_the_symbol_is_the_function_a_site_sits_in_or_else_the_first_item_it_gates(self):
        source = f'''cfg_if::cfg_if! {{
    if #[cfg({T})] {{
        pub use gwz_transport::cbor;
    }} else {{
        pub mod cbor;
    }}
}}
cfg_if::cfg_if! {{
    if #[cfg(all(unix, {T}))] {{
        use std::sync::{{Arc, Mutex}};
        #[derive(Clone)]
        pub(crate) struct Runtime(Arc<Mutex<u8>>);
        fn helper() {{}}
    }} else if #[cfg({S})] {{
        impl Default for Runtime {{ fn default() -> Self {{ todo!() }} }}
    }}
}}
pub struct Meta {{
    #[cfg({S})]
    pub transport_message: Option<String>,
}}
pub(crate) const SESSION: bool = !(cfg!({S}) || false);
#[cfg({S})]
unsafe extern "C" {{ fn probe(); }}
impl Backend {{
    cfg_if::cfg_if! {{
        if #[cfg({S})] {{
            pub(crate) fn with_host_context(&self) {{}}
        }}
    }}
    fn with_transport(&self) {{
        cfg_if::cfg_if! {{
            if #[cfg({T})] {{
                let selected = 1;
                fn nested() {{}}
            }}
        }}
        let run = || {{ #[cfg({S})] {{ println!(); }} }};
    }}
}}
'''
        self.assertEqual(checker.sites(source), [
            (T, 'use gwz_transport::cbor'), (T, 'struct Runtime'), (S, 'impl Default for Runtime'),
            (S, 'struct Meta'), (S, 'const SESSION'), (S, 'extern "C"'), (S, 'fn with_host_context'),
            (T, 'fn with_transport'), (S, 'fn with_transport'),
        ])


class Inventory(TwoSwitches):
    LIB = f'#[cfg({T})]\nmod a {{}}\nfn b() {{ cfg_if::cfg_if! {{ if #[cfg({S})] {{ let x = 1; }} }} }}\n'
    ROWS = [f'{S}  src/lib.rs  fn b', f'{T}  src/lib.rs  mod a']

    def repo(self, directory, rows, build=DECLARATIONS, header='# What this is, and which test reads it.\n\n'):
        root = Path(directory)
        for path, text in (('build.rs', build), ('src/lib.rs', self.LIB),
                           ('scripts/candidate_switch_inventory.txt', header + ''.join(f'{r}\n' for r in rows))):
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text, encoding='utf-8')
        return root

    def test_the_sites_must_equal_the_inventory_in_order(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(checker.check(self.repo(directory, self.ROWS)), [])
            # Any run of spaces or tabs separates the three fields.
            self.assertEqual(checker.check(self.repo(directory, [f'{S}\tsrc/lib.rs   fn b ', self.ROWS[1]])), [])
            self.assertEqual(checker.check(self.repo(directory, self.ROWS[:1] + [f'{T}  src/lib.rs  mod z'])),
                             [f'NEW    {T}  src/lib.rs  mod a', f'STALE  {T}  src/lib.rs  mod z'])
            errors = checker.check(self.repo(directory, self.ROWS[::-1]))
            self.assertEqual(len(errors), 1, errors)
            self.assertTrue(errors[0].startswith('ORDER'), errors)

    def test_each_switch_must_be_declared_in_build_rs_or_the_manifest_check_cfg(self):
        with tempfile.TemporaryDirectory() as directory:
            only = f'fn main() {{ println!("cargo:rustc-check-cfg=cfg({T})"); }}\n'
            self.assertEqual(checker.check(self.repo(directory, self.ROWS, build=only)),
                             [f'UNDECLARED {S}: neither build.rs nor Cargo.toml declares it in check-cfg'])
            (Path(directory) / 'Cargo.toml').write_text(
                f"[lints.rust]\nunexpected_cfgs = {{ level = \"warn\", check-cfg = ['cfg({S})'] }}\n", encoding='utf-8')
            self.assertEqual(checker.check(self.repo(directory, self.ROWS, build=only)), [])

    def test_list_prints_the_sites_and_a_mismatch_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.repo(directory, self.ROWS[:1])
            stdout, stderr = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                self.assertEqual(checker.main(['--repo', str(root), '--list']), 0)
                self.assertEqual(checker.main(['--repo', str(root)]), 1)
            self.assertEqual(stdout.getvalue(), ''.join(f'{r}\n' for r in self.ROWS))
            self.assertIn(f'NEW    {T}  src/lib.rs  mod a', stderr.getvalue())


class Retired(unittest.TestCase):
    """`gwz_session_candidate` is retired: no site may name it, and nothing declares it."""

    def test_a_site_that_names_a_retired_switch_is_an_error_whatever_the_inventory_says(self):
        lib = f'#[cfg({T})]\nmod a {{}}\n#[cfg({RETIRED})]\nmod b {{}}\n'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path, text in (('build.rs', f'println!("cargo:rustc-check-cfg=cfg({T})");\n'), ('src/lib.rs', lib),
                               ('scripts/candidate_switch_inventory.txt', f'# h\n{T}  src/lib.rs  mod a\n')):
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text(text, encoding='utf-8')
            self.assertEqual(checker.check(root), [f'RETIRED {RETIRED}  src/lib.rs  mod b'])

    def test_the_retired_switch_is_not_a_switch(self):
        self.assertEqual(checker.SWITCHES, (T,))
        self.assertEqual(checker.RETIRED, (RETIRED,))


class Repository(unittest.TestCase):
    def test_core_switch_sites_equal_its_inventory(self):
        errors = checker.check(checker.ROOT)
        self.assertEqual(errors, [], '\n'.join(errors))

    def test_the_test_runner_runs_the_check(self):
        spec = importlib.util.spec_from_file_location('runner', checker.ROOT / 'scripts' / 'run_tests.py')
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        with patch.object(runner.subprocess, 'run') as run, contextlib.redirect_stdout(io.StringIO()):
            run.return_value.returncode = 0
            with self.assertRaises(SystemExit):
                runner.main(['--skip-transport-globals', '--skip-cfg-siblings'])
        commands = [[str(part) for part in call.args[0]] for call in run.call_args_list]
        self.assertIn([sys.executable, str(checker.ROOT / 'scripts/checks/check_candidate_switches.py')], commands)

    def test_the_boundary_job_runs_the_check_and_this_test(self):
        text = (checker.ROOT / '.github' / 'workflows' / 'checked-artifact-boundary.yml').read_text(encoding='utf-8')
        self.assertRegex(text, r'\n +python scripts/checks/check_candidate_switches\.py\n')
        self.assertRegex(text, r'\n +python -m unittest [^\n]*scripts/checks/test_check_candidate_switches\.py')


class CandidateWorkflow(unittest.TestCase):
    """Rule (a): until S7.1 removes gwz_transport_candidate the candidate job keeps the transport shape green; plan
    step 2.2 retired the session leg. S7.1 updates this test with the job."""

    def test_the_candidate_job_has_one_leg(self):
        lines = WORKFLOW.read_text(encoding='utf-8').splitlines()
        start = lines.index('  candidate:')
        end = next((i for i in range(start + 1, len(lines)) if re.match(r'  [\w-]+:', lines[i])), len(lines))
        job = '\n'.join(lines[start:end])
        self.assertEqual(re.findall(r'^ +(?:- )?rustflags: *(.+)$', job, re.M), [])
        # The suites build with the transport switch, through the job's only RUSTFLAGS.
        self.assertEqual(re.findall(r'^ +RUSTFLAGS: *(.+)$', job, re.M), [f'--cfg {T}'])
        self.assertNotIn('matrix', job)
        self.assertNotIn(f'--cfg {RETIRED}', WORKFLOW.read_text(encoding='utf-8'))


class WindowsCandidateJob(unittest.TestCase):
    """TR4.6, second part (GwzTransportWindowsParityPlan.md, step 5.4): after step 5.1 retired the Windows-only switch,
    the Windows job builds the same candidate as the Linux legs and runs every candidate test that needs no fixture, and
    the SSH rows that run against a portable sshd.exe. These rows read the job's text, so that a change which drops a
    shape, a target or the sshd fails here, on Linux, before a hosted run can show it."""

    @staticmethod
    def job():
        lines = WORKFLOW.read_text(encoding='utf-8').splitlines()
        start = lines.index('  candidate-windows:')
        end = next((i for i in range(start + 1, len(lines)) if re.match(r'  [\w-]+:', lines[i])), len(lines))
        return '\n'.join(lines[start:end])

    @staticmethod
    def step(job, name):
        """The text of the step called `name`: from its `- name:` line to the next step."""
        parts = re.split(r'(?m)^      - ', job)
        found = [p for p in parts if re.match(rf'(?:id: \w+\n +)?name: {re.escape(name)}\n', p)]
        assert len(found) == 1, f'expected one step named {name!r}, found {len(found)}'
        return found[0]

    def test_the_job_is_the_pinned_image_under_bash(self):
        job = self.job()
        self.assertIn('name: Candidate build on Windows', job)
        self.assertIn('runs-on: windows-2022', job)
        self.assertIn('shell: bash', job)
        self.assertIn('working-directory: gwz-core', job)

    def test_it_builds_the_ordinary_shape_and_the_candidate_shape_and_no_third(self):
        # Step 5.1 retired the Windows-only qualification switch; the shapes are what the Linux legs' first leg is.
        job = self.job()
        self.assertEqual(sorted(set(re.findall(r'^ +RUSTFLAGS: *(.*)$', job, re.M))), sorted(['""', f'--cfg {T}']))
        self.assertFalse('gwz_windows' in job, 'the retired Windows-only switch is named in the job')

    def test_it_runs_the_library_suites_through_the_source_checks_and_the_sshd_rows(self):
        job = self.job()
        suites = self.step(job, "Run the candidate build's library suites")
        self.assertIn(f'RUSTFLAGS: --cfg {T}', suites)
        self.assertRegex(suites, r'python scripts/run_tests\.py [^\n]*--lib')
        self.assertIn('GWZ_TEST_SSHD', self.step(job, 'Install the OpenSSH server the SSH fixtures run'))

    def test_the_integration_step_names_every_integration_target_and_excludes_none(self):
        """Cargo discovers one target per tests/*.rs, and prepare.py adds the placement guide. A target that is not
        named here never runs on Windows, and a hand-kept list is how that happens, so the list is the discovered set."""
        job = self.job()
        step = self.step(job, 'Integration targets')
        named = re.findall(r'--test +([\w-]+)', step)
        discovered = sorted(path.stem for path in (checker.ROOT / 'tests').glob('*.rs')) + ['transport_placement_guide']
        self.assertEqual(sorted(named), sorted(discovered))
        self.assertIn('publish_workflow', named)
        self.assertIn('GWZ_TEST_GIT: real', step)
        self.assertIn(f'RUSTFLAGS: --cfg {T}', step)
        # No exclusion is left to explain: the text that gave publish_workflow's reason is gone.
        self.assertNotIn('Not publish_workflow', job)
        self.assertNotIn('collapsed before the lookup', job)


if __name__ == '__main__':
    unittest.main()
