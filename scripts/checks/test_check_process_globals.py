"""Tests for the process-global state ratchet (GwzCoreSessionDesign O9)."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import re
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('process_globals', Path(__file__).with_name('check_process_globals.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

TRANSPORT_ALLOWLIST = Path(__file__).with_name('process_globals_allowlist_gwz_transport.json')
BOUNDARY_WORKFLOW = checker.ROOT / '.github' / 'workflows' / 'checked-artifact-boundary.yml'
TRANSPORT_ALLOWLIST_ARG = '--allowlist scripts/checks/process_globals_allowlist_gwz_transport.json'


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
    let child = std::process::Command::new("git").arg("tag").status();
    let helper = tokio::process::Command::new(program).spawn();
}
use std::env::{self, var};
'''
        self.assertEqual(found(source), sorted([
            ('static', 'LOCK'), ('static', 'RAW'), ('static', 'COUNTER'), ('static', 'LAZY'),
            ('static', 'CUSTOM'), ('thread_local', 'SLOT'), ('env', 'env::var_os'),
            ('env', 'env::current_dir'), ('libgit2', 'git2::opts::set_server_timeout_in_milliseconds'),
            ('libgit2', 'git2::opts::set_verify_owner_validation'), ('hook', 'panic::set_hook'),
            ('env', 'dirs::home_dir'), ('env', 'libc::getenv'), ('env', 'env::var'),
            ('process', 'Command::new("git")'), ('process', 'Command::new'),
        ]))

    def test_flags_libgit2_credential_helper_spawn_only(self):
        self.assertEqual(found('''
fn f() {
    let credential = git2::Cred::credential_helper(&config, url, None);
    let agent = git2::Cred::ssh_key_from_agent(user);
}
'''), [('process', 'Cred::credential_helper')])

    def test_flags_the_credential_helper_struct_as_the_same_spawn(self):
        # Safety P3-31 of Verdict-3: git2's public CredentialHelper performs
        # the spawn that Cred::credential_helper wraps, so both spellings are
        # one occurrence and a switch between them never reads as debt paid.
        self.assertEqual(found('''
fn f() {
    let found = git2::CredentialHelper::new(url).config(&config).execute();
    let policy = CredentialHelperPolicy::AllowConfigured;
    let helper: Option<CredentialHelper> = None;
}
'''), [('process', 'Cred::credential_helper')])

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


class GlobalState(unittest.TestCase):
    """GwzCoreSessionCrateMap §1, "No globals": which declarations are global mutable state.

    A `permanent` entry may only be immutable data, a cache of immutable data, or
    state a named dependency imposes. Global mutable state is any counter, flag,
    lock-protected or cell-held state, or thread-local, read from the declaration.
    """

    def states(self, source):
        return {o.name: o.state for o in checker.analyze(source).occurrences}

    def test_counters_flags_locks_cells_and_thread_locals_are_mutable_state(self):
        states = self.states('''
static COUNTER: AtomicU64 = AtomicU64::new(0);
static FLAG: std::sync::atomic::AtomicBool = AtomicBool::new(false);
static INIT: Once = Once::new();
static LOCK: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static ORPHANS: OnceLock<std::sync::Mutex<Vec<Child>>> = OnceLock::new();
static SWAP: ArcSwapOption<String> = ArcSwapOption::const_empty();
static CELL: SyncUnsafeCell<u8> = SyncUnsafeCell::new(0);
static mut RAW: u8 = 0;
static PER_THREAD: ThreadLocal<u8> = ThreadLocal::new();
thread_local! { static SLOT: Cell<bool> = const { Cell::new(false) }; static PLAIN: u8 = 0; }
fn f() {
    git2::opts::set_server_timeout_in_milliseconds(1);
    std::panic::set_hook(hook);
}
''')
        expected = {
            'COUNTER': 'counter or flag (AtomicU64)',
            'FLAG': 'counter or flag (AtomicBool)',
            'INIT': 'counter or flag (Once)',
            'LOCK': 'lock-protected state (Mutex)',
            'SLOTS': 'lock-protected state (Semaphore)',
            'ORPHANS': 'lock-protected state (Mutex)',
            'SWAP': 'cell-held state (ArcSwapOption)',
            'CELL': 'cell-held state (SyncUnsafeCell)',
            'RAW': 'static mut',
            'PER_THREAD': 'thread-local (ThreadLocal)',
            # A thread-local counts as global state whatever it holds.
            'SLOT': 'thread-local',
            'PLAIN': 'thread-local',
            'git2::opts::set_server_timeout_in_milliseconds': 'libgit2',
            'panic::set_hook': 'hook',
        }
        self.assertEqual(set(states), set(expected))
        for name, phrase in expected.items():
            self.assertIn(phrase, states[name] or '', name)

    def test_a_type_the_scan_cannot_see_into_counts_as_mutable(self):
        # Fail closed: a lexical scan cannot see inside a named type, and every
        # such static the three allowlists list today (agent_job's HUB,
        # gwz-py's REGISTRY and STORE) is in fact a mutable registry.
        states = self.states('''
static HUB: OnceLock<Hub> = OnceLock::new();
static CUSTOM: Registry = Registry::new();
static HANDLER: LazyLock<Box<dyn Fn() + Send + Sync>> = LazyLock::new(|| Box::new(|| ()));
''')
        self.assertIn('Hub', states['HUB'] or '')
        self.assertIn('Registry', states['CUSTOM'] or '')
        self.assertIn('Fn', states['HANDLER'] or '')

    def test_lazy_static_statics_are_found_whatever_they_hold(self):
        # `lazy_static!` declares `static ref NAME: T`, a lazily initialised
        # global. Every one is listed, even of a primitive or reference type
        # that a plain static would pass, and its type classifies it as a
        # static's does: mutable state, an unseen type, or a cache.
        found = {o.name: (o.kind, o.state) for o in checker.analyze('''
lazy_static! {
    static ref CACHE: Mutex<HashMap<String, u8>> = Mutex::new(HashMap::new());
    #[doc = "the hub"]
    pub(crate) static ref HUB: Hub = Hub::new();
    pub static ref LIMIT: u32 = 3;
    static ref NAME: &'static str = "gwz";
}
lazy_static::lazy_static! { pub static ref NEXT: AtomicU64 = AtomicU64::new(0); }
''').occurrences}
        self.assertEqual(set(found), {'CACHE', 'HUB', 'LIMIT', 'NAME', 'NEXT'})
        self.assertEqual({kind for kind, _ in found.values()}, {'lazy_static'})
        self.assertIn('lock-protected state (Mutex)', found['CACHE'][1] or '')
        self.assertIn('Hub', found['HUB'][1] or '')
        self.assertIn('counter or flag (AtomicU64)', found['NEXT'][1] or '')
        self.assertEqual((found['LIMIT'][1], found['NAME'][1]), (None, None))

    def test_a_static_a_macro_rules_declares_is_listed_unparsed(self):
        # Safety P3-3 of the steps' review: outside the two known macros too, a
        # `static` whose declaration the scan cannot read is listed, as global
        # mutable state. `static` is a keyword; `'static` is a lifetime token.
        occurrences = checker.analyze('''
macro_rules! counter { ($name:ident) => { static $name: AtomicU64 = AtomicU64::new(0); }; }
counter!(D_MACRO);
fn f(text: &'static str) -> &'static str { text }
''').occurrences
        self.assertEqual([(o.kind, o.name) for o in occurrences], [('static', '<unparsed static>')])
        self.assertTrue(occurrences[0].state)

    def test_a_raw_identifier_named_static_is_not_a_declaration(self):
        self.assertEqual(found('fn f() { let r#static = 1; let _ = r#static; }\n'), [])

    def test_a_static_those_macros_declare_is_never_skipped(self):
        # Fail closed: a generated name the scan cannot read is still listed,
        # as global mutable state, so it fails as NEW until someone looks.
        occurrences = checker.analyze('''
macro_rules! registry { ($name:ident) => { lazy_static! { static ref $name: Mutex<u8> = Mutex::new(0); } }; }
macro_rules! slot { ($name:ident) => { thread_local! { static $name: Cell<u8> = Cell::new(0); } }; }
''').occurrences
        self.assertEqual([(o.kind, o.name) for o in occurrences],
                         [('lazy_static', '<unparsed static>'), ('thread_local', '<unparsed static>')])
        self.assertTrue(all(o.state for o in occurrences))

    def test_immutable_data_and_caches_of_it_are_not_mutable_state(self):
        states = self.states('''
static NAMES: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(HashMap::new);
static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
static GREETING: String = String::new();
static TABLE: [Option<u8>; LEN] = [None; LEN];
static PAIRS: once_cell::sync::Lazy<Vec<(String, u64)>> = once_cell::sync::Lazy::new(Vec::new);
static DOUBLE: fn(u8) -> u8 = double;
fn f() {
    let home = std::env::var_os("HOME");
    let child = std::process::Command::new("git").status();
}
''')
        self.assertEqual(states, dict.fromkeys(
            ['NAMES', 'ROOT', 'GREETING', 'TABLE', 'PAIRS', 'DOUBLE', 'env::var_os', 'Command::new("git")']))


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

    def test_follows_a_literal_include_and_records_one_it_cannot_read(self):
        # Hardening from the same finding: `include!` splices a file into its
        # includer, so a literal path is followed like a `mod`, relative to the
        # including file. Any other production include is recorded with its
        # literal fragments, and fails the check closed (S-3, below).
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'src/lib.rs': 'include!("../extra/included.rs");\n'
                              'include!(concat!(env!("OUT_DIR"), "/generated.rs"));\n'
                              '#[cfg(test)]\nmod tests { include!(concat!(env!("OUT_DIR"), "/t.rs")); '
                              'include!("../extra/test_only.rs"); }\n',
                'extra/included.rs': 'static O_INCLUDED: AtomicU64 = AtomicU64::new(0);\n',
                'extra/test_only.rs': GLOBAL,
            })
            result = checker.scan(root, ['src/lib.rs'])
            self.assertEqual(sorted(result.occurrences), [('extra/included.rs', 'static', 'O_INCLUDED')])
            self.assertEqual(result.unreadable_includes, [('src/lib.rs', 2, ('OUT_DIR', '/generated.rs'))])
            self.assertEqual(result.unreached, [])


class TestsDirectory(unittest.TestCase):
    """Standing rule (operator, 2026-09-29): a crate's `tests/` directory holds no
    production code. Any build that compiles a file outside `cfg(test)`, such as
    the `gwz_transport_candidate` build, makes it production."""

    def test_production_code_reached_under_a_crate_tests_directory_is_listed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'Cargo.toml': '[package]\nname = "fixture"\n',
                'src/lib.rs': 'cfg_if::cfg_if! {\n'
                              '    if #[cfg(gwz_transport_candidate)] {\n'
                              '        #[path = "../tests/candidate/generated.rs"]\n'
                              '        pub mod generated;\n'
                              '    } else {\n'
                              '        #[path = "generated.rs"]\n'
                              '        pub mod generated;\n'
                              '    }\n'
                              '}\n'
                              '#[cfg(unix)]\n#[path = "../tests/support/platform.rs"]\nmod platform;\n'
                              'include!("../tests/spliced.rs");\n',
                'src/generated.rs': '',
                'tests/candidate/generated.rs': '',
                'tests/support/platform.rs': '',
                'tests/spliced.rs': '',
            })
            result = checker.scan(root, ['src/lib.rs'])
            self.assertEqual(result.in_tests, [
                ('tests/candidate/generated.rs', ('src/lib.rs',)),
                ('tests/spliced.rs', ('src/lib.rs',)),
                ('tests/support/platform.rs', ('src/lib.rs',)),
            ])

    def test_test_only_code_may_live_under_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'Cargo.toml': '[package]\nname = "fixture"\n',
                'src/lib.rs': '#[cfg(test)]\n#[path = "../tests/support/a.rs"]\nmod a;\n'
                              'cfg_if::cfg_if! {\n'
                              '    if #[cfg(all(test, unix))] {\n'
                              '        #[path = "../tests/support/b.rs"]\n'
                              '        mod b;\n'
                              '    }\n'
                              '}\n'
                              '#[cfg(test)]\nmod tests { include!("../tests/support/c.rs"); }\n'
                              'mod checks;\n'
                              '#[cfg(feature = "contract-tests")]\n#[path = "../tests/support/f.rs"]\nmod f;\n',
                'src/checks.rs': '#![cfg(test)]\n#[path = "../tests/support/d.rs"]\nmod d;\n',
                'tests/support/a.rs': '',
                'tests/support/b.rs': '',
                'tests/support/c.rs': '',
                'tests/support/d.rs': '',
                'tests/support/f.rs': '',
            })
            self.assertEqual(checker.scan(root, ['src/lib.rs'], frozenset({'contract-tests'})).in_tests, [])
            # A feature is a test build only when the allowlist names it one.
            self.assertEqual(checker.scan(root, ['src/lib.rs']).in_tests,
                             [('tests/support/f.rs', ('src/lib.rs',))])

    def test_a_file_that_production_also_reaches_is_production(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'Cargo.toml': '[package]\nname = "fixture"\n',
                'src/lib.rs': '#[cfg(test)]\nmod checks;\nmod shared;\n',
                'src/checks.rs': '#[path = "../tests/shared.rs"]\nmod shared;\n',
                'src/shared.rs': '#[path = "../tests/shared.rs"]\nmod inner;\n',
                'tests/shared.rs': '',
            })
            self.assertEqual(checker.scan(root, ['src/lib.rs']).in_tests,
                             [('tests/shared.rs', ('src/shared.rs',))])

    def test_a_crate_tests_directory_is_the_one_beside_its_cargo_toml(self):
        # A member crate's own `tests/` counts, whoever reaches into it. A
        # module directory named `tests` inside `src/` is not a crate's.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'Cargo.toml': '[workspace]\nmembers = ["crates/*"]\n',
                'crates/family/Cargo.toml': '[package]\nname = "family"\n',
                'crates/family/src/lib.rs': '#[path = "../tests/fixture.rs"]\nmod fixture;\nmod view;\n',
                'crates/family/src/view.rs': '#[path = "tests/rows.rs"]\nmod rows;\n',
                'crates/family/src/tests/rows.rs': '',
                'crates/family/tests/fixture.rs': '',
                'src/lib.rs': '#[path = "../crates/family/tests/fixture.rs"]\nmod borrowed;\n',
            })
            result = checker.scan(root, ['src/lib.rs', 'crates/*/src/lib.rs'])
            self.assertEqual(result.in_tests,
                             [('crates/family/tests/fixture.rs', ('crates/family/src/lib.rs', 'src/lib.rs'))])

    def test_check_fails_on_production_code_under_tests_and_no_entry_waives_it(self):
        files = {
            'Cargo.toml': '[package]\nname = "fixture"\n',
            'src/lib.rs': '#[cfg(gwz_transport_candidate)]\n#[path = "../tests/generated.rs"]\nmod generated;\n',
            'tests/generated.rs': '',
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, files)
            allowlist = root / 'allowlist.json'
            allowlist.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': []}), encoding='utf-8')
            errors, _, _ = checker.check(root, allowlist)
            self.assertEqual(len(errors), 1, errors)
            self.assertTrue(errors[0].startswith("TESTS tests/generated.rs: production code in a crate's "
                                                 'tests/ directory, loaded by src/lib.rs;'), errors)
            stderr = io.StringIO()
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stderr):
                self.assertEqual(checker.main(['--repo', str(root), '--allowlist', str(allowlist)]), 1)
            self.assertIn('TESTS tests/generated.rs', stderr.getvalue())

    # S-3 of the cross-lane cleanup's Safety review (2026-09-29): the rule failed
    # open on an `include!` the scan cannot read and on a `cfg_attr` path.
    # Fixtures A and B are the reviewer's.

    def check_fixture(self, files, entries=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, files)
            allowlist = root / 'allowlist.json'
            allowlist.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': list(entries)}), encoding='utf-8')
            return checker.check(root, allowlist)[0]

    def test_fixture_a_an_unreadable_include_naming_tests_fails_and_no_entry_waives_it(self):
        files = {
            'Cargo.toml': '[package]\nname = "f"\n',
            'src/lib.rs': 'include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/spliced.rs"));\n',
            'tests/spliced.rs': 'static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n',
        }
        waiver = {'path': 'src/lib.rs', 'kind': 'include', 'name': '<unparsed include>', 'disposition': 'permanent',
                  'reason': 'probe: does a permanent include entry waive the tests/ rule?'}
        tests_error = ("TESTS src/lib.rs:1: an include! the scan cannot read names a tests/ directory "
                       "('CARGO_MANIFEST_DIR', '/tests/spliced.rs'); no entry waives it")
        self.assertEqual(self.check_fixture(files), [tests_error])
        # The entry waives nothing, and is refused itself.
        errors = self.check_fixture(files, [waiver])
        self.assertEqual(len(errors), 2, errors)
        self.assertIn(tests_error, errors)
        self.assertTrue(any(error.startswith('INCL  include <unparsed include> in src/lib.rs: an include! the '
                                             'scan cannot read is never allowlisted') for error in errors), errors)

    def test_an_unreadable_include_anywhere_in_production_fails_closed(self):
        # The scan cannot see what it splices in, globals included, so it fails
        # whatever the path; a literal path is followed instead.
        errors = self.check_fixture({
            'src/lib.rs': 'include!(concat!(env!("OUT_DIR"), "/generated.rs"));\n'
                          '#[cfg(test)]\nmod tests { include!(concat!(env!("OUT_DIR"), "/t.rs")); }\n',
        })
        self.assertEqual(errors, ["INCL  src/lib.rs:1: an include! the scan cannot read ('OUT_DIR', "
                                  "'/generated.rs'); spell its path as a literal, which the scan follows"])

    def test_fixture_b_a_cfg_attr_path_into_tests_is_followed_and_fails(self):
        errors = self.check_fixture({
            'Cargo.toml': '[package]\nname = "f"\n',
            'src/lib.rs': '#[cfg_attr(not(test), path = "../tests/prod.rs")]\nmod prod;\n',
            'tests/prod.rs': 'static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n',
        })
        self.assertIn("TESTS tests/prod.rs: production code in a crate's tests/ directory, loaded by src/lib.rs; "
                      'move it under src/', errors)
        # The file is scanned as production as well: its static is new.
        self.assertTrue(any(error.startswith('NEW   static COUNTER') for error in errors), errors)

    def test_a_cfg_attr_path_is_production_unless_its_predicate_implies_test(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'Cargo.toml': '[package]\nname = "fixture"\n',
                'src/lib.rs': '#[cfg_attr(test, path = "../tests/support/probe.rs")]\nmod probe;\n'
                              '#[cfg_attr(unix, path = "../tests/support/unix.rs")]\nmod platform;\n'
                              '#[cfg_attr(not(test), allow(dead_code), path = "../tests/support/two.rs")]\nmod two;\n'
                              '#[cfg_attr(unix, cfg_attr(not(test), path = "../tests/support/nested.rs"))]\n'
                              'mod nested;\n'
                              '#[cfg_attr(not(test), path = "live.rs")]\nmod swapped;\n',
                # Default files: `probe.rs` loads whenever `test` does not hold,
                # `swapped.rs` only when `not(test)` does not, in test builds.
                'src/probe.rs': 'static PROBE: Mutex<u8> = Mutex::new(0);\n',
                'src/swapped.rs': 'static SWAPPED: Mutex<u8> = Mutex::new(0);\n',
                'src/live.rs': '',
                'tests/support/probe.rs': '',
                'tests/support/unix.rs': '',
                'tests/support/two.rs': '',
                'tests/support/nested.rs': '',
            })
            result = checker.scan(root, ['src/lib.rs'])
            self.assertEqual(result.in_tests, [
                ('tests/support/nested.rs', ('src/lib.rs',)),
                ('tests/support/two.rs', ('src/lib.rs',)),
                ('tests/support/unix.rs', ('src/lib.rs',)),
            ])
            self.assertEqual(sorted(result.occurrences), [('src/probe.rs', 'static', 'PROBE')])
            self.assertEqual(result.unreached, [])


class Ratchet(unittest.TestCase):
    def run_check(self, entries, files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, files)
            allowlist = root / 'allowlist.json'
            allowlist.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': entries}), encoding='utf-8')
            return checker.check(root, allowlist)[0]

    def entry(self, **overrides):
        entry = {'path': 'src/lib.rs', 'kind': 'static', 'name': 'G', 'disposition': 'debt', 'reason': 'r',
                 'owner': 'o'}
        entry.update(overrides)
        return {key: value for key, value in entry.items() if value is not None}

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

    def test_debt_planted_under_either_candidate_switch_is_listed(self):
        # TR2.12: each candidate switch's build is production, so state under either is listed and its debt entry
        # matches, and the module's definition of production names both switches.
        for switch in ('gwz_transport_candidate', 'gwz_session_candidate'):
            source = f'cfg_if::cfg_if! {{\n    if #[cfg(all(unix, {switch}))] {{\n        {GLOBAL}    }}\n}}\n'
            errors = self.run_check([], {'src/lib.rs': source})
            self.assertEqual(len(errors), 1, (switch, errors))
            self.assertRegex(errors[0], r'^NEW   static G \(Mutex < u8 >\) at src/lib\.rs:3')
            self.assertEqual(self.run_check([self.entry()], {'src/lib.rs': source}), [], switch)
        definition = ('Production means any build outside `cfg(test)`, the `gwz_transport_candidate` and '
                      '`gwz_session_candidate` builds included;')
        self.assertTrue(definition in ' '.join(checker.__doc__.split()), f'the docstring lacks: {definition}')

    def test_injected_credential_helper_struct_is_the_listed_spawn(self):
        # Safety P3-31's closure test: the fault injected into the fixture is
        # CredentialHelper::new(url).execute(). Unlisted, it is NEW as
        # `process` `Cred::credential_helper`. Listed as that, a switch from
        # Cred::credential_helper to it keeps the entry matched, not STALE.
        injected = 'fn f() { let found = CredentialHelper::new(url).execute(); }\n'
        errors = self.run_check([], {'src/lib.rs': injected})
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('NEW   process Cred::credential_helper at src/lib.rs:1'), errors)
        listed = self.entry(kind='process', name='Cred::credential_helper')
        before = 'fn f() { let found = git2::Cred::credential_helper(&config, url, None); }\n'
        self.assertEqual(self.run_check([listed], {'src/lib.rs': before}), [])
        self.assertEqual(self.run_check([listed], {'src/lib.rs': injected}), [])

    def test_permanent_global_mutable_state_names_the_dependency_that_imposes_it(self):
        # GwzCoreSessionCrateMap §1: no counter, flag, lock- or cell-held state
        # or thread-local is permanent unless a named dependency imposes it.
        cases = [
            ('static', 'C', 'static C: AtomicU64 = AtomicU64::new(0);\n'),
            ('static', 'H', 'static H: OnceLock<Hub> = OnceLock::new();\n'),
            ('thread_local', 'S', 'thread_local! { static S: Cell<bool> = const { Cell::new(false) }; }\n'),
            ('libgit2', 'git2::opts::set_server_timeout_in_milliseconds',
             'fn f() { git2::opts::set_server_timeout_in_milliseconds(1); }\n'),
            ('hook', 'panic::set_hook', 'fn f() { std::panic::set_hook(hook); }\n'),
            ('lazy_static', 'L', 'lazy_static! { static ref L: Mutex<u8> = Mutex::new(0); }\n'),
        ]
        for kind, name, source in cases:
            listed = self.entry(kind=kind, name=name, disposition='permanent', owner=None)
            errors = self.run_check([listed], {'src/lib.rs': source})
            self.assertEqual(len(errors), 1, (name, errors))
            self.assertTrue(errors[0].startswith(f'PERM  {kind} {name} in src/lib.rs: '), errors)
            self.assertIn('imposed_by', errors[0])
            listed['imposed_by'] = 'libgit2'
            self.assertEqual(self.run_check([listed], {'src/lib.rs': source}), [], name)

    def test_permanent_immutable_data_spawns_and_env_reads_need_no_dependency(self):
        source = ('static NAMES: LazyLock<HashMap<&\'static str, u32>> = LazyLock::new(HashMap::new);\n'
                  'fn f() { std::env::var("X"); Command::new("gh").env_clear(); }\n')
        entries = [self.entry(kind=kind, name=name, disposition='permanent', owner=None)
                   for kind, name in (('static', 'NAMES'), ('env', 'env::var'), ('process', 'Command::new("gh")'))]
        self.assertEqual(self.run_check(entries, {'src/lib.rs': source}), [])

    def test_debt_global_state_names_its_owner(self):
        slot = 'thread_local! { static S: Cell<u8> = const { Cell::new(0) }; }\n'
        lazy = 'lazy_static! { static ref L: u32 = 3; }\n'
        for kind, name, source in (('static', 'G', GLOBAL), ('thread_local', 'S', slot), ('lazy_static', 'L', lazy)):
            errors = self.run_check([self.entry(kind=kind, name=name, owner=None)], {'src/lib.rs': source})
            self.assertEqual(len(errors), 1, errors)
            self.assertIn('owner', errors[0])
            self.assertIn(f"'{kind}', '{name}'", errors[0])
            self.assertEqual(self.run_check([self.entry(kind=kind, name=name, owner='unassigned')],
                                            {'src/lib.rs': source}), [])
        # Spawns and env reads are not global state and keep their treatment.
        source = 'fn f() { std::env::var("X"); std::process::Command::new("git").status(); }\n'
        entries = [self.entry(kind=kind, name=name, owner=None)
                   for kind, name in (('env', 'env::var'), ('process', 'Command::new("git")'))]
        self.assertEqual(self.run_check(entries, {'src/lib.rs': source}), [])

    def test_owner_and_imposed_by_are_non_empty_text(self):
        for field_name, value in (('owner', ''), ('owner', 7), ('imposed_by', '  '), ('imposed_by', ['libgit2'])):
            errors = self.run_check([self.entry(**{field_name: value})], {'src/lib.rs': GLOBAL})
            self.assertEqual(len(errors), 1, (field_name, value, errors))
            self.assertIn(f'{field_name} must be', errors[0])


class ReconciledCommit(unittest.TestCase):
    """An allowlist for another repository records the commit it was reconciled against."""

    def allowlist(self, directory, **fields):
        path = Path(directory) / 'allowlist.json'
        path.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': [], **fields}), encoding='utf-8')
        return path

    def test_only_a_full_lowercase_sha_is_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            loaded, errors = checker.load_allowlist(self.allowlist(directory, reconciled_commit='a' * 40))
            self.assertEqual((loaded.reconciled_commit, errors), ('a' * 40, []))
            loaded, errors = checker.load_allowlist(self.allowlist(directory))
            self.assertEqual((loaded.reconciled_commit, errors), (None, []))
            for bad in ('46e65a9', 'A' * 40, 'g' * 40, 'a' * 41, '', 40):
                loaded, errors = checker.load_allowlist(self.allowlist(directory, reconciled_commit=bad))
                self.assertIsNone(loaded.reconciled_commit, bad)
                self.assertEqual(len(errors), 1, bad)
                self.assertIn('reconciled_commit', errors[0])

    def test_reader_prints_the_commit_or_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            for fields, status, printed in (({'reconciled_commit': 'b' * 40}, 0, 'b' * 40 + '\n'),
                                            ({}, 1, ''),
                                            ({'reconciled_commit': 'b' * 12}, 1, '')):
                stdout, stderr = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    result = checker.main(['--allowlist', str(self.allowlist(directory, **fields)),
                                           '--reconciled-commit'])
                self.assertEqual((result, stdout.getvalue()), (status, printed), fields)
                self.assertEqual(bool(stderr.getvalue()), status != 0, fields)


def job_steps(text, job):
    """The step blocks of one job in a workflow: jobs sit at two spaces, steps at six."""
    lines = text.splitlines()
    start = lines.index(f'  {job}:')
    end = next((i for i in range(start + 1, len(lines)) if re.match(r'  [\w-]+:', lines[i])), len(lines))
    steps = []
    for line in lines[start + 1:end]:
        if line.startswith('      - '):
            steps.append([line])
        elif steps and (line.startswith('        ') or not line.strip()):
            steps[-1].append(line)
    return '\n'.join(lines[start:end]), ['\n'.join(step) for step in steps]


def field(step, key):
    match = re.search(rf'^ +(?:- )?{key}: *(.*)$', step, re.M)
    return match.group(1).strip() if match else None


class TransportPin(unittest.TestCase):
    """B11 of Verdict-3: gwz-core's boundary job checks gwz-transport at the allowlist's commit."""

    def test_reconciled_commit_is_a_full_sha(self):
        loaded, errors = checker.load_allowlist(TRANSPORT_ALLOWLIST)
        self.assertEqual(errors, [])
        self.assertRegex(loaded.reconciled_commit or '', r'^[0-9a-f]{40}$')
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            self.assertEqual(checker.main(['--allowlist', str(TRANSPORT_ALLOWLIST), '--reconciled-commit']), 0)
        self.assertEqual(stdout.getvalue(), loaded.reconciled_commit + '\n')

    def test_boundary_job_checks_out_gwz_transport_at_the_reconciled_commit(self):
        text = BOUNDARY_WORKFLOW.read_text(encoding='utf-8')
        job, steps = job_steps(text, 'boundary')
        checkouts = [step for step in steps if field(step, 'repository') == 'owebeeone/gwz-transport']
        self.assertEqual(len(checkouts), 1, 'one gwz-transport checkout in the boundary job')
        checkout = checkouts[0]
        ref = re.fullmatch(r'\$\{\{ steps\.([\w-]+)\.outputs\.(\w+) \}\}', field(checkout, 'ref') or '')
        self.assertIsNotNone(ref, 'the checkout takes its ref from a step output')
        step_id, output = ref.groups()
        readers = [step for step in steps if field(step, 'id') == step_id]
        self.assertEqual(len(readers), 1)
        reader = readers[0]
        self.assertLess(steps.index(reader), steps.index(checkout))
        self.assertIn('python scripts/checks/check_process_globals.py', reader)
        self.assertIn(TRANSPORT_ALLOWLIST_ARG, reader)
        self.assertIn('--reconciled-commit', reader)
        self.assertIn(f'{output}=', reader)
        self.assertIn('"$GITHUB_OUTPUT"', reader)
        # Beside gwz-core, as a workspace has them, and every command runs in gwz-core.
        self.assertIn('actions/checkout', steps[0])
        self.assertEqual(field(steps[0], 'path'), 'gwz-core')
        self.assertEqual(field(checkout, 'path'), 'gwz-transport')
        self.assertEqual(field(job, 'working-directory'), 'gwz-core')
        # The check runs over that checkout, and the job has no way to skip it.
        runs = [step for step in steps if '--repo ../gwz-transport' in step]
        self.assertEqual(len(runs), 1)
        self.assertLess(steps.index(checkout), steps.index(runs[0]))
        self.assertIn(TRANSPORT_ALLOWLIST_ARG, runs[0])
        self.assertNotIn('--skip-transport-globals', job)
        # The allowlist is the only place the commit is written.
        loaded, _ = checker.load_allowlist(TRANSPORT_ALLOWLIST)
        self.assertNotIn(loaded.reconciled_commit, text)


def listed(path):
    data = json.loads(path.read_text(encoding='utf-8'))
    return {(e['path'], e['kind'], e['name']): e for e in data['entries']}, data['rule']


class Dispositions(unittest.TestCase):
    """GwzCoreSessionCrateMap §6 step 1: the counters and CROSSING become debt, each with its owner;
    steps 2 and 4 then remove the temp-name counters and CROSSING."""

    def test_the_temp_name_counters_are_gone(self):
        # Crate map §6 step 2: gwz-ids replaced the four temp-name counters,
        # so their entries left the list with them.
        core, _ = listed(checker.DEFAULT_ALLOWLIST)
        for path, name in (('crates/family-store/src/publish.rs', 'SEQUENCE'),
                           ('src/artifact/encoding.rs', 'TEMP_SEQ'),
                           ('src/verified_write.rs', 'TEMP_SEQUENCE'),
                           ('src/workspace_ops/merge/v1_lifecycle/store/rewrite.rs', 'TEMP_SEQUENCE')):
            self.assertNotIn((path, 'static', name), core)
        # 26 -> 25 (2026-09-29): the local-import Git fallback's spawn left, below.
        # 25 -> 24 (2026-09-29): the HTTPS helper `SLOTS` semaphore left; its budget
        # is the host's now (CS6.5, pulled forward).
        self.assertEqual(len(core), 24)

    def test_the_local_import_git_fallback_spawn_is_gone(self):
        # GwzNoFallbackPlan.md §4: `fetch_anonymous` runs on the libgit2 fork
        # alone, so its `git fetch` fallback and that spawn's entry are gone.
        core, _ = listed(checker.DEFAULT_ALLOWLIST)
        self.assertNotIn(('src/git/gitbackend/transport.rs', 'process', 'Command::new("git")'), core)

    def test_crossing_is_gone(self):
        # Crate map §6 step 4: the gate moved to gwz-session-host, where
        # non-Clone controls and a per-gate thread record replace the
        # thread-local, so its entry left the list with it.
        core, _ = listed(checker.DEFAULT_ALLOWLIST)
        self.assertNotIn(('src/session_host/gate.rs', 'thread_local', 'CROSSING'), core)
        self.assertEqual([key for key in core if key[0].startswith('src/session_host/')], [])

    def test_the_counters_are_debt_with_their_owners(self):
        core, _ = listed(checker.DEFAULT_ALLOWLIST)
        transport, _ = listed(TRANSPORT_ALLOWLIST)
        extraction = "CS7.1, the candidate crates' extraction (IdSource)"
        expected = [
            (core, 'src/git/endpoint/https_auth.rs', 'static', 'NEXT_ID', extraction),
            (core, 'src/git/endpoint/https_local.rs', 'static', 'NEXT_SESSION', extraction),
            (core, 'src/git/endpoint/ssh_worker.rs', 'static', 'NEXT_WORKER', extraction),
            (core, 'src/transport_host/session.rs', 'static', 'SERIAL', extraction),
            (transport, 'src/pool/machine.rs', 'static', 'NEXT_POOL',
             'CS7.2–CS7.6 (the pool takes its ID from its host)'),
        ]
        for entries, path, kind, name, owner in expected:
            entry = entries[(path, kind, name)]
            self.assertEqual((entry['disposition'], entry.get('owner')), ('debt', owner), name)
            self.assertNotIn('imposed_by', entry, name)

    def test_libgit2_timeout_stays_permanent_naming_libgit2(self):
        core, _ = listed(checker.DEFAULT_ALLOWLIST)
        for kind, name in (('static', 'TIMEOUT_STATE'),
                           ('libgit2', 'git2::opts::set_server_connect_timeout_in_milliseconds'),
                           ('libgit2', 'git2::opts::set_server_timeout_in_milliseconds')):
            entry = core[('src/git/gitbackend/transport_support.rs', kind, name)]
            self.assertEqual((entry['disposition'], entry.get('imposed_by')), ('permanent', 'libgit2'), name)

    def test_the_rule_states_the_definition(self):
        _, rule = listed(checker.DEFAULT_ALLOWLIST)
        for phrase in ('immutable data', 'a cache of immutable data', 'a named dependency imposes',
                       'counter, flag, lock-protected or cell-held state, or thread-local',
                       "'imposed_by'", "'owner'", 'GwzCoreSessionCrateMap §1'):
            self.assertIn(phrase, rule)
        # The gwz-transport copy defers to that rule rather than restating it.
        self.assertIn("gwz-core's scripts/checks/process_globals_allowlist.json", listed(TRANSPORT_ALLOWLIST)[1])


class Repository(unittest.TestCase):
    def test_core_adds_no_process_global_state(self):
        errors, _, _ = checker.check(checker.ROOT, checker.DEFAULT_ALLOWLIST)
        self.assertEqual(errors, [], '\n'.join(errors))


if __name__ == '__main__':
    unittest.main()
