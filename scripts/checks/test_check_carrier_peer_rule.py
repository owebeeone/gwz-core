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
    'local/peer.rs': 'pub struct PeerPolicy { own: u8 }\nimpl PeerPolicy { pub fn verify(&self) {} fn new() -> Self { PeerPolicy { own: 1 } } }\nfn s() { sandbox::of_self(); }\n',
    'local/sandbox.rs': 'cfg_if::cfg_if! { if #[cfg(any(target_os = "linux", target_os = "android"))] { mod linux; } else if #[cfg(target_vendor = "apple")] { mod macos; } }\n',
    'local/sandbox/linux.rs': 'fn probe() { std::fs::read_to_string("/proc/self/status"); }\n',
    'local/unix.rs': 'fn a() { p.verify(x, Relation::ListenerChecksClient); }\nfn b() { p.verify(y, Relation::ClientChecksListener); }\n'
                     'fn check_peer() { let hold = Hold::take(fd)?; sandbox::of_peer(&hold, fd); policy.verify(&peer, r)?; hold.confirm(fd)?; }\n'
                     'impl Hold { fn check(&self) {} fn confirm(self) { self.check() } }\n',
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

    def test_assigning_the_policys_own_field_fails(self):
        for source in ['fn f(mut policy: PeerPolicy, peer: X) { policy.own = peer; }\n',
                       'fn f(policy: &mut PeerPolicy, peer: X) { policy.own.principal = peer; }\n',
                       'fn f(policy: &PeerPolicy) { let _ = &policy.own; }\n']:
            with self.subTest(source=source):
                root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + source})
                self.assertTrue(any('own field' in line for line in checker.violations(root)), source)

    def test_the_own_accessor_call_is_allowed(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + 'fn f(policy: &PeerPolicy) { let _ = policy.own().clone(); }\n'})
        self.assertEqual(checker.violations(root), [])

    def test_the_test_build_constructor_named_outside_peer_rs_fails(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + 'fn f() { let p = PeerPolicy::from_identity(x); }\n'})
        self.assertTrue(any('constructor outside peer.rs' in line for line in checker.violations(root)))

    def test_peer_rs_may_define_its_test_constructor_under_cfg_test(self):
        peer = CLEAN['local/peer.rs'] + 'cfg_if::cfg_if! { if #[cfg(test)] { impl PeerPolicy { pub fn from_identity(own: u8) -> Self { Self { own } } } } }\n'
        self.assertEqual(checker.violations(tree(**{'local/peer.rs': peer})), [])

    def test_a_second_verify_fails(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + 'fn verify() {}\n'})
        self.assertTrue(any('second verify' in line for line in checker.violations(root)))

    def test_a_platform_module_must_check_both_directions(self):
        root = tree(**{'local/windows.rs': 'fn a() { p.verify(x, Relation::ListenerChecksClient); }\n'})
        self.assertTrue(any('must each check' in line for line in checker.violations(root)))

    def test_the_sandbox_probes_may_not_read_the_environment_or_name_a_bypass(self):
        for source in ['let v = std::env::var("PROC");', 'let v = env::var_os("X");', 'fn accept_any_sandbox() {}', 'fn insecure_probe() {}']:
            with self.subTest(source=source):
                root = tree(**{'local/sandbox/linux.rs': CLEAN['local/sandbox/linux.rs'] + source + '\n'})
                self.assertTrue(checker.violations(root), source)

    def test_a_probe_may_not_add_a_platform_switch_and_sandbox_rs_may_only_list_its_two(self):
        for rel in ['local/sandbox/linux.rs', 'local/sandbox.rs']:
            for source in ['#[cfg(test)]\nfn weaker() {}\n', '#[cfg(unix)]\nfn weaker() {}\n', '#[cfg(debug_assertions)]\nfn weaker() {}\n']:
                with self.subTest(rel=rel, source=source):
                    root = tree(**{rel: CLEAN[rel] + source})
                    self.assertTrue(any('switch' in line for line in checker.violations(root)))

    def test_the_cfg_expression_macro_is_a_switch_too(self):
        # `cfg!(..)` is an expression, not an attribute: a probe that returns early under it is a build variant with a different rule.
        for rel in ['local/sandbox/linux.rs', 'local/unix.rs', 'local/sandbox.rs']:
            for source in ['fn weaker() { if cfg!(debug_assertions) { return; } }\n', 'fn weaker() -> bool { cfg!(not(test)) }\n',
                           'fn weaker() -> bool { cfg ! ( feature = "lax" ) }\n']:
                with self.subTest(rel=rel, source=source):
                    root = tree(**{rel: CLEAN[rel] + source})
                    self.assertTrue(any('switch' in line for line in checker.violations(root)), source)

    def test_the_cfg_expression_macro_is_allowed_nowhere_not_even_for_the_files_own_conditions(self):
        # The attribute list names `test` for peer.rs, windows.rs and mod.rs and the platform conditions for unix.rs, but
        # `if !cfg!(test) { return Ok(()); }` is strict in a test build and lax in production, where no row can see it.
        cases = [('local/peer.rs', 'fn verify_loosely() -> Result<(), ()> { if !cfg!(test) { return Ok(()); } Err(()) }\n'),
                 ('local/windows.rs', 'fn weaker() -> bool { !cfg!(test) }\n'),
                 ('local/windows.rs', 'fn weaker() -> bool { cfg!(test) }\n'),
                 ('mod.rs', 'fn weaker() -> bool { !cfg!(test) }\n'),
                 ('local/mod.rs', 'fn weaker() -> bool { cfg!(unix) }\n'),
                 ('local/unix.rs', 'fn check_loosely(peer: P) -> R { if cfg!(target_os = "android") { return Ok(peer); } todo() }\n'),
                 ('local/unix.rs', 'fn host() -> bool { cfg!(target_vendor = "apple") }\n'),
                 ('local/sandbox.rs', 'fn host() -> bool { cfg!(any(target_os = "linux", target_os = "android")) }\n'),
                 ('local/unix.rs', 'fn host() -> bool { core::cfg!(target_os = "android") }\n')]
        for rel, source in cases:
            with self.subTest(rel=rel, source=source):
                root = tree(**{rel: CLEAN[rel] + source})
                self.assertTrue(any('switch' in line for line in checker.violations(root)), source)

    def test_a_platform_condition_is_allowed_as_a_cfg_if_or_attribute_condition(self):
        source = ('cfg_if::cfg_if! { if #[cfg(any(target_os = "linux", target_os = "android"))] { cfg_if::cfg_if! { if #[cfg(target_os = "android")] { '
                  'const A: i32 = 77; } else { const A: i32 = 1; } } } else { fn other() {} } }\n')
        self.assertEqual(checker.violations(tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + source})), [])

    def test_the_hold_must_be_confirmed_after_the_probe(self):
        # Md2: the non-consuming test of the hold before the probe, the confirmation dropped.
        unix = CLEAN['local/unix.rs'].replace(' hold.confirm(fd)?;', '').replace('sandbox::of_peer(&hold, fd);', 'hold.check(fd)?; sandbox::of_peer(&hold, fd);')
        found = checker.violations(tree(**{'local/unix.rs': unix}))
        self.assertTrue(any('never calls hold.confirm(' in line for line in found), found)
        self.assertTrue(any('check_peer calls .check(' in line for line in found), found)

    def test_a_check_call_in_check_peer_fails_even_beside_the_confirmation(self):
        for call in ['hold.check(fd)?;', 'Hold::check(&hold, fd)?;', 'hold . check ( fd )?;']:
            with self.subTest(call=call):
                unix = CLEAN['local/unix.rs'].replace('sandbox::of_peer(&hold, fd);', f'{call} sandbox::of_peer(&hold, fd);')
                found = checker.violations(tree(**{'local/unix.rs': unix}))
                self.assertTrue(any('check_peer calls .check(' in line for line in found), found)

    def test_dropping_the_confirmation_alone_fails(self):
        unix = CLEAN['local/unix.rs'].replace(' hold.confirm(fd)?;', '')
        self.assertTrue(any('never calls hold.confirm(' in line for line in checker.violations(tree(**{'local/unix.rs': unix}))))

    def test_check_outside_check_peer_is_the_holds_own_business(self):
        # The hold's own `check` (and a platform probe's closure over it) are not in check_peer.
        unix = CLEAN['local/unix.rs'] + 'fn probe(hold: &Hold) { let _ = || hold.check(fd); }\n'
        self.assertEqual(checker.violations(tree(**{'local/unix.rs': unix})), [])

    def test_a_unix_module_without_check_peer_fails(self):
        unix = CLEAN['local/unix.rs'].replace('fn check_peer()', 'fn renamed()')
        self.assertTrue(any('no check_peer function' in line for line in checker.violations(tree(**{'local/unix.rs': unix}))))

    def test_dropping_a_probe_call_fails(self):
        for rel, text in [('local/unix.rs', CLEAN['local/unix.rs'].replace('sandbox::of_peer(&hold, fd)', 'nothing()')),
                          ('local/peer.rs', CLEAN['local/peer.rs'].replace('sandbox::of_self()', 'nothing()'))]:
            with self.subTest(rel=rel):
                self.assertTrue(any('sandbox probes must run' in line for line in checker.violations(tree(**{rel: text}))))

    def test_the_checked_constructor_is_named_like_the_other_test_constructor(self):
        root = tree(**{'local/unix.rs': CLEAN['local/unix.rs'] + 'fn f() { let p = PeerPolicy::try_from_identity(x); }\n'})
        self.assertTrue(any('constructor outside peer.rs' in line for line in checker.violations(root)))

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
