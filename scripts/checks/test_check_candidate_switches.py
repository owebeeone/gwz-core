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

T, S = 'gwz_transport_candidate', 'gwz_session_candidate'
WORKFLOW = checker.ROOT / '.github' / 'workflows' / 'transport-candidate.yml'
DECLARATIONS = ''.join(f'println!("cargo:rustc-check-cfg=cfg({switch})");\n' for switch in (T, S))


class Sites(unittest.TestCase):
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


class Inventory(unittest.TestCase):
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
    """Rule (a): until S7.1 (1.1.0) the candidate job keeps two shapes green, the transport switch alone and both
    switches. S7.1 (1.1.0) removes gwz_transport_candidate and updates this test with the job."""

    def test_the_candidate_job_has_both_legs(self):
        lines = WORKFLOW.read_text(encoding='utf-8').splitlines()
        start = lines.index('  candidate:')
        end = next((i for i in range(start + 1, len(lines)) if re.match(r'  [\w-]+:', lines[i])), len(lines))
        job = '\n'.join(lines[start:end])
        self.assertEqual(re.findall(r'^ +(?:- )?rustflags: *(.+)$', job, re.M), [f'--cfg {T}', f'--cfg {T} --cfg {S}'])
        # The suites build with the leg's flags, through the job's only RUSTFLAGS; one leg's failure cancels neither.
        self.assertEqual(re.findall(r'^ +RUSTFLAGS: *(.+)$', job, re.M), ['${{ matrix.rustflags }}'])
        self.assertIn('fail-fast: false', job)


if __name__ == '__main__':
    unittest.main()
