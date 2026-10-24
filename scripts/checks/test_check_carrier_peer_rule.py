import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('carrier_peer_rule', Path(__file__).with_name('check_carrier_peer_rule.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

CLEAN = {
    'mod.rs': 'cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }\n',
    'local/mod.rs': 'cfg_if::cfg_if! { if #[cfg(unix)] { mod unix; } else if #[cfg(windows)] { mod windows; } }\n'
                    'cfg_if::cfg_if! { if #[cfg(all(test, unix))] { pub use unix::x; } else if #[cfg(all(test, windows))] { pub use windows::y; } }\n',
    'local/peer.rs': 'pub struct PeerPolicy { own: u8 }\nimpl PeerPolicy { pub fn verify(&self) {} fn new() -> Self { PeerPolicy { own: 1 } } }\n',
    'local/unix.rs': 'fn a() { p.verify(x, Relation::ListenerChecksClient); }\nfn b() { p.verify(y, Relation::ClientChecksListener); }\n',
    'local/windows.rs': 'fn a() { p.verify(x, Relation::ListenerChecksClient); }\nfn b() { p.verify(y, Relation::ClientChecksListener); }\n',
}


def tree(**changes):
    files = {**CLEAN, **changes}
    root = Path(tempfile.mkdtemp())
    for rel, text in files.items():
        path = root / checker.MODULE / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')
    return root


class PeerRuleTests(unittest.TestCase):
    def test_a_clean_module_passes(self):
        self.assertEqual(checker.violations(tree()), [])

    def test_the_real_module_passes(self):
        self.assertEqual(checker.violations(checker.ROOT), [])

    def test_environment_reads_fail(self):
        for source in ['let v = std::env::var("X");', 'let v = env::var_os("X");', 'let v = option_env!("X");']:
            with self.subTest(source=source):
                root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + source + '\n'})
                self.assertTrue(checker.violations(root))

    def test_bypass_names_fail(self):
        for name in ['skip_peer_check', 'insecure_mode', 'accept_any', 'for_tests', 'test_only', 'peer_override_x']:
            with self.subTest(name=name):
                root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + f'fn {name}() {{}}\n'})
                self.assertTrue(checker.violations(root), name)

    def test_a_test_switch_in_production_code_fails(self):
        for source in ['#[cfg(test)]\nfn weaker() {}\n', '#[cfg(feature = "lax")]\nfn weaker() {}\n',
                       '#[cfg(debug_assertions)]\nfn weaker() {}\n']:
            with self.subTest(source=source):
                root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + source})
                self.assertTrue(checker.violations(root))

    def test_a_policy_built_outside_peer_rs_fails(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + 'fn f() { let p = PeerPolicy { own: 1 }; }\n'})
        self.assertTrue(any('outside peer.rs' in line for line in checker.violations(root)))

    def test_a_second_verify_fails(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + 'fn verify() {}\n'})
        self.assertTrue(any('second verify' in line for line in checker.violations(root)))

    def test_a_platform_module_must_check_both_directions(self):
        root = tree(**{'local/windows.rs': 'fn a() { p.verify(x, Relation::ListenerChecksClient); }\n'})
        self.assertTrue(any('must each check' in line for line in checker.violations(root)))

    def test_rows_may_do_what_production_may_not(self):
        root = tree()
        tests = root / checker.MODULE / 'tests'
        tests.mkdir()
        (tests / 'rows.rs').write_text('fn skip_peer() { std::env::var("X"); PeerPolicy { own: 1 }; }\n', encoding='utf-8')
        self.assertEqual(checker.violations(root), [])

    def test_comments_and_strings_are_not_scanned(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + '// insecure bypass\nlet s = "std::env::var";\n/* skip_peer */\n'})
        self.assertEqual(checker.violations(root), [])


if __name__ == '__main__':
    unittest.main()
