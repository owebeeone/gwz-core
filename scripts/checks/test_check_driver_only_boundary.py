import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'driver_only_boundary', Path(__file__).with_name('check_driver_only_boundary.py')
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

CLEAN = '''//! The driver-only builder.
use super::session::Session;
use crate::session_host::EnvironmentSnapshot;
pub struct DriverOnlyBuilder { io_timeout_ms: u64, connect_timeout_ms: u64 }
impl DriverOnlyBuilder {
    pub fn build(self, environment: &EnvironmentSnapshot) -> ModelResult<(DriverOnly, TransportPort)> {
        let (session, port) = Session::driver(self.io_timeout_ms, self.connect_timeout_ms, None)?;
        Ok((DriverOnly::new(session), port))
    }
}
'''


def tree(source=CLEAN, mod_rs='mod driver_only;\n'):
    root = Path(tempfile.mkdtemp())
    for rel, text in ((checker.MODULE, source), (checker.PARENT, mod_rs)):
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')
    return root


class DriverOnlyBoundaryTests(unittest.TestCase):
    def test_a_clean_module_passes(self):
        self.assertEqual(checker.violations(tree()), [])

    def test_the_real_module_passes(self):
        self.assertEqual(checker.violations(checker.ROOT), [])

    def test_a_missing_module_fails(self):
        root = Path(tempfile.mkdtemp())
        (root / checker.PARENT).parent.mkdir(parents=True)
        (root / checker.PARENT).write_text('mod driver_only;\n', encoding='utf-8')
        self.assertTrue(any('missing' in line for line in checker.violations(root)))

    def test_a_module_the_parent_does_not_declare_fails(self):
        self.assertTrue(checker.violations(tree(mod_rs='mod other;\n')))

    def test_the_types_that_make_a_local_endpoint_fail(self):
        for name in ['EndpointSettings', 'SshEndpointConfig', 'SshSettings', 'HttpsEndpointConfig', 'Handoff',
                     'AgentSource', 'HelperSlots', 'NativeCaller', 'NativeHost', 'ProcessSafety', 'ProcessGate',
                     'process_safety', 'CliEndpoint', 'CoreFactory', 'ConnectionHost']:
            with self.subTest(name=name):
                root = tree(CLEAN + f'fn leak(x: {name}) {{}}\n')
                self.assertTrue(any(name in line for line in checker.violations(root)), name)

    def test_the_modules_that_hold_a_credential_fail(self):
        for path in ['endpoint_environment', 'agent_auth', 'ssh_pool', 'ssh_key_snapshot', 'ssh_handoff',
                     'ssh_home', 'agent_source', 'connection_factory']:
            with self.subTest(path=path):
                root = tree(CLEAN + f'use crate::git::endpoint::{path}::x;\n')
                self.assertTrue(checker.violations(root), path)

    def test_a_local_endpoint_constructor_fails(self):
        for call in ['Session::endpoint(c)', 'Session::endpoint_with_https(c, h, k)',
                     'Session::endpoint_with_https_native(c, h, k, n)']:
            with self.subTest(call=call):
                root = tree(CLEAN + f'fn leak() {{ let _ = {call}; }}\n')
                self.assertTrue(checker.violations(root), call)

    def test_a_driver_session_given_a_handoff_fails(self):
        for call in ['Session::driver(a, b, Some(h))', 'Session::driver_with_ssh_budgets(a, b, None, c, d)']:
            with self.subTest(call=call):
                root = tree(CLEAN + f'fn leak() {{ let _ = {call}; }}\n')
                self.assertTrue(checker.violations(root), call)

    def test_environment_reads_fail(self):
        for source in ['let v = std::env::var("X");', 'let v = env::var_os("X");', 'let v = env!("X");',
                       'let v = "SSH_AUTH_SOCK";']:
            with self.subTest(source=source):
                root = tree(CLEAN + f'fn leak() {{ {source} }}\n')
                self.assertTrue(checker.violations(root), source)

    def test_glob_imports_fail(self):
        for source in ['use super::*;\n', 'use super::{session::Session, *};\n', 'use crate::git::endpoint::*;\n']:
            with self.subTest(source=source):
                self.assertTrue(checker.violations(tree(CLEAN + source)), source)

    def test_conditional_compilation_fails(self):
        for source in ['#[cfg(test)]\nmod tests {}\n', 'cfg_if::cfg_if! { if #[cfg(windows)] { fn f() {} } }\n']:
            with self.subTest(source=source):
                self.assertTrue(checker.violations(tree(CLEAN + source)), source)

    def test_a_name_in_a_comment_or_a_string_passes(self):
        source = CLEAN + '// EndpointSettings and Handoff are not accepted here.\nconst WORDS: &str = "Handoff";\n'
        self.assertEqual(checker.violations(tree(source)), [])

    def test_a_longer_identifier_is_not_the_name(self):
        self.assertEqual(checker.violations(tree(CLEAN + 'struct HandoffNote;\n')), [])

    def test_a_seeded_handoff_import_fails(self):
        root = tree(CLEAN + 'use super::Handoff;\n')
        self.assertTrue(any('Handoff' in line for line in checker.violations(root)))

    def test_the_local_transport_entry_points_fail(self):
        # Code P3-2, v1: `with_cancellable_local_transport` builds an SSH and HTTPS `TransportRuntime` from the
        # snapshot, with agent and helpers, from inputs the module already holds.
        for source in [
            'fn leak(e: &EnvironmentSnapshot, t: &CancellationToken, m: RequestMeta) {\n'
            '    let _ = super::with_cancellable_local_transport(m, "fetch".into(), e, t, |_b| ());\n}\n',
            'fn leak() { let _ = super::with_local_transport(a, b); }\n',
            'fn leak() { let _ = super::cancellable::run(a); }\n',
            'fn leak(x: super::TransportRuntime) {}\n',
            'fn leak() { let _ = local_command(a); }\n',
            'use super::local_command;\n',
        ]:
            with self.subTest(source=source):
                self.assertTrue(checker.violations(tree(CLEAN + source)), source)

    def test_an_alias_of_an_import_fails(self):
        # Code P3-2, v2: `Session as Host` carries `Session::endpoint` past the spelling scan.
        for source in [
            'use super::session::Session as Host;\nfn leak() { let _ = Host::endpoint(c); }\n',
            'use super::{session::Session as Host, invalid};\n',
            'use std::sync::Arc as Shared;\n',
        ]:
            with self.subTest(source=source):
                self.assertTrue(any(' as ' in line or 'alias' in line for line in checker.violations(tree(CLEAN + source))), source)

    def test_an_import_outside_the_allow_list_fails(self):
        for source in ['use crate::git::endpoint::ssh_remote::OpenStream;\n', 'use super::DriverSomethingNew;\n',
                       'use std::collections::HashMap;\n']:
            with self.subTest(source=source):
                self.assertTrue(any('allow-list' in line for line in checker.violations(tree(CLEAN + source))), source)

    def test_a_driver_session_constructor_that_is_not_a_call_fails(self):
        # State axis: a function-pointer alias of `Session::driver` evades the argument scan.
        for source in ['fn leak() { let make = Session::driver; let _ = make(a, b, Some(h)); }\n',
                       'fn leak() { let make: fn(u64, u64, Option<Handoff>) = Session::driver; }\n']:
            with self.subTest(source=source):
                self.assertTrue(checker.violations(tree(CLEAN + source)), source)

    def test_the_gates_run_the_check_and_its_unit_test(self):
        # Code P3-1: the type-level row has to be a gate. `run_boundary_job.py` replays the workflow's own lines.
        workflow = (checker.ROOT / '.github/workflows/checked-artifact-boundary.yml').read_text(encoding='utf-8')
        self.assertIn('python scripts/checks/check_driver_only_boundary.py', workflow)
        self.assertIn('scripts/checks/test_check_driver_only_boundary.py', workflow)
        runner = (checker.ROOT / 'scripts/run_tests.py').read_text(encoding='utf-8')
        self.assertIn('check_driver_only_boundary.py', runner)


if __name__ == '__main__':
    unittest.main()
