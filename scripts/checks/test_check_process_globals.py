"""Tests for the process-global state ratchet (GwzCoreSessionDesign O9)."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('process_globals', Path(__file__).with_name('check_process_globals.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def found(source, features=frozenset()):
    return sorted((o.kind, o.name) for o in checker.analyze(source, features).occurrences)


class Detection(unittest.TestCase):
    def test_flags_shared_statics_thread_locals_and_ambient_reads(self):
        source = '''
static LOCK: std::sync::Mutex<u8> = std::sync::Mutex::new(0);
static mut RAW: u8 = 0;
static COUNTER: AtomicU64 = AtomicU64::new(0);
static LAZY: OnceLock<String> = OnceLock::new();
static CUSTOM: Registry = Registry::new();
thread_local! { static SLOT: Cell<u8> = const { Cell::new(0) }; }
fn f() {
    let home = std::env::var_os("HOME");
    let cwd = env::current_dir();
    git2::opts::set_server_timeout_in_milliseconds(1);
    opts::set_verify_owner_validation(false);
    std::panic::set_hook(hook);
    let h = dirs::home_dir();
    let p = unsafe { libc::getenv(name) };
}
use std::env::{self, var};
'''
        self.assertEqual(found(source), sorted([
            ('static', 'LOCK'), ('static', 'RAW'), ('static', 'COUNTER'), ('static', 'LAZY'),
            ('static', 'CUSTOM'), ('thread_local', 'SLOT'), ('env', 'env::var_os'),
            ('env', 'env::current_dir'), ('libgit2', 'git2::opts::set_server_timeout_in_milliseconds'),
            ('libgit2', 'git2::opts::set_verify_owner_validation'), ('hook', 'panic::set_hook'),
            ('env', 'dirs::home_dir'), ('env', 'libc::getenv'), ('env', 'env::var'),
        ]))

    def test_ignores_immutable_data_lifetimes_comments_and_literals(self):
        self.assertEqual(found('''
static NAMES: &[&str] = &["a", "b"];
static LIMIT: u32 = 3;
static TABLE: [u8; 4] = [0; 4];
const SHARED: AtomicU64 = AtomicU64::new(0);
fn f(x: &'static str) -> Box<dyn Fn() + Send + 'static> { todo!() }
// static HIDDEN: Mutex<u8> = Mutex::new(0);
/* std::env::var("X") /* nested */ thread_local! {} */
fn g() {
    let text = "static QUOTED: Mutex<u8>; std::env::var(\\"X\\")";
    let raw = r#"thread_local! { static R: Cell<u8> }"#;
    let brace = '{';
    let name = env!("CARGO_PKG_NAME");
    let optional = option_env!("GWZ_X");
    let value = self.env.var("HOME");
}
'''), [])

    def test_exempts_only_code_that_requires_test(self):
        source = '''
#[cfg(test)] static A: Mutex<u8> = Mutex::new(0);
#[cfg(all(test, unix))] fn b() { static B: AtomicU64 = AtomicU64::new(0); }
#[cfg(test)]
#[allow(dead_code)]
mod tests { fn c() { std::env::var("C"); } }
cfg_if::cfg_if! {
    if #[cfg(test)] { static D: Mutex<u8> = Mutex::new(0); }
    else { static E: Mutex<u8> = Mutex::new(0); }
}
cfg_if::cfg_if! {
    if #[cfg(not(test))] { static F: Mutex<u8> = Mutex::new(0); }
    else if #[cfg(unix)] { static G: Mutex<u8> = Mutex::new(0); }
    else { static H: Mutex<u8> = Mutex::new(0); }
}
#[cfg(not(test))] static I: Mutex<u8> = Mutex::new(0);
#[cfg(any(test, windows))] static J: Mutex<u8> = Mutex::new(0);
enum Mode { #[cfg(test)] Probe, Real }
static K: Mutex<u8> = Mutex::new(0);
#[cfg(any(test, feature = "contract-tests"))] static L: Mutex<u8> = Mutex::new(0);
'''
        self.assertEqual(found(source), [('static', name) for name in 'EFIJKL'])
        self.assertEqual(found(source, frozenset({'contract-tests'})), [('static', name) for name in 'EFIJK'])

    def test_inner_test_attribute_exempts_the_whole_scope(self):
        self.assertEqual(found('#![cfg(test)]\nstatic A: Mutex<u8> = Mutex::new(0);'), [])
        self.assertEqual(found('mod m { #![cfg(test)] static A: Mutex<u8> = Mutex::new(0); }\n'
                               'static B: Mutex<u8> = Mutex::new(0);'), [('static', 'B')])

    def test_line_numbers(self):
        occurrence = checker.analyze('\n\nfn f() {\n    std::env::var("X");\n}').occurrences[0]
        self.assertEqual(occurrence.line, 4)


def write(root, files):
    for name, text in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')


GLOBAL = 'static G: Mutex<u8> = Mutex::new(0);\n'


class ModuleTree(unittest.TestCase):
    def test_follows_mod_and_path_declarations_and_exempts_test_modules(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'src/lib.rs': 'mod a;\n#[cfg(test)]\nmod b;\n#[path = "x/c_impl.rs"]\nmod c;\n'
                              'mod d { pub(crate) mod e; }\n',
                'src/a.rs': GLOBAL + 'mod nested;\n',
                'src/a/nested.rs': GLOBAL,
                'src/b.rs': GLOBAL + 'mod inner;\n',
                'src/b/inner.rs': GLOBAL,
                'src/x/c_impl.rs': 'mod sibling;\n',
                'src/x/sibling.rs': GLOBAL,
                'src/d/e.rs': GLOBAL,
                'src/orphan.rs': GLOBAL,
            })
            result = checker.scan(root, ['src/lib.rs'])
            self.assertEqual(sorted(path for path, _, _ in result.occurrences),
                             ['src/a.rs', 'src/a/nested.rs', 'src/d/e.rs', 'src/orphan.rs', 'src/x/sibling.rs'])
            self.assertEqual(result.unreached, ['src/orphan.rs'])


class Ratchet(unittest.TestCase):
    def run_check(self, entries, files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, files)
            allowlist = root / 'allowlist.json'
            allowlist.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': entries}), encoding='utf-8')
            return checker.check(root, allowlist)[0]

    def entry(self, **overrides):
        entry = {'path': 'src/lib.rs', 'kind': 'static', 'name': 'G', 'disposition': 'debt', 'reason': 'r'}
        entry.update(overrides)
        return entry

    def test_listed_state_passes_and_new_state_fails(self):
        self.assertEqual(self.run_check([self.entry()], {'src/lib.rs': GLOBAL}), [])
        errors = self.run_check([self.entry()], {'src/lib.rs': GLOBAL + 'fn f() { std::env::var("X"); }\n'})
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('NEW   env env::var at src/lib.rs:2'), errors)

    def test_removed_state_must_leave_the_list(self):
        errors = self.run_check([self.entry()], {'src/lib.rs': 'fn f() {}\n'})
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('STALE static G'), errors)

    def test_counts_must_match_exactly(self):
        source = 'fn f() { std::env::var("A"); std::env::var("B"); }\n'
        listed = self.entry(kind='env', name='env::var', count=1)
        self.assertTrue(self.run_check([listed], {'src/lib.rs': source})[0].startswith('COUNT'))
        listed['count'] = 2
        self.assertEqual(self.run_check([listed], {'src/lib.rs': source}), [])

    def test_entries_need_a_disposition_and_reason(self):
        errors = self.run_check([self.entry(disposition='later', reason='')], {'src/lib.rs': GLOBAL})
        self.assertEqual(len(errors), 2)


class Repository(unittest.TestCase):
    def test_core_adds_no_process_global_state(self):
        errors, _, _ = checker.check(checker.ROOT, checker.DEFAULT_ALLOWLIST)
        self.assertEqual(errors, [], '\n'.join(errors))


if __name__ == '__main__':
    unittest.main()
