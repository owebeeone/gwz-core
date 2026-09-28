"""Tests for the conditional-compilation boundary ratchet (root AGENTS.md, explicit scope; CS1.7)."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('cfg_boundaries', Path(__file__).with_name('check_cfg_boundaries.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

runner_spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(runner_spec)
runner_spec.loader.exec_module(runner)

WINDOWS_IMPORT = ('#[cfg(windows)]', 'use std::os::windows::ffi::OsStrExt')
BOUNDARY_WORKFLOW = checker.ROOT / '.github' / 'workflows' / 'checked-artifact-boundary.yml'


def found(source):
    return [(o.attrs, o.item) for o in checker.analyze(source)]


def key(text):
    """The key of a whole-text target: its tokens, rendered."""
    return checker.render(checker.lex(text))


class Detection(unittest.TestCase):
    def test_bare_cfg_import_fails_in_a_disabled_arm(self):
        # No host compiles all three: the scan inspects every arm without compiling any.
        self.assertEqual(found('#[cfg(windows)]\nuse std::os::windows::ffi::OsStrExt;\n'), [WINDOWS_IMPORT])
        self.assertEqual(found('''
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;
    } else {
        #[cfg(windows)] use std::os::windows::ffi::OsStrExt;
    }
}
#[cfg(windows)]
mod imp {
    #[cfg(windows)]
    use std::os::windows::ffi::OsStrExt;
}
'''), [WINDOWS_IMPORT] * 2)

    def test_the_same_import_inside_cfg_if_passes(self):
        self.assertEqual(found('''
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
    } else if #[cfg(unix)] {
        pub(crate) use std::os::unix::ffi::OsStrExt;
    }
}
#[cfg(windows)]
mod imp { use std::os::windows::ffi::OsStrExt; }
'''), [])

    def test_flags_every_unbraced_declaration(self):
        self.assertEqual(found('''
#[cfg(unix)] pub use a::B;
#[cfg(unix)] pub(crate) use a::{c, d as e};
#[cfg(unix)] extern crate alloc;
#[cfg(test)] mod tests;
#[cfg(unix)] type Alias = u8;
#[cfg(unix)] const LIMIT: u8 = { 3 };
#[cfg(unix)] static mut RAW: Mode = Mode { raw: 0 };
#[cfg(unix)] struct Unit;
#[cfg(unix)] pub(super) struct Pair<T>(T, T) where T: Copy;
#[cfg(unix)] fn helper() { #[cfg(unix)] use a::F; }
trait Api {
    #[cfg(unix)] type Handle;
    #[cfg(unix)] const ID: u8;
    #[cfg(unix)] fn raw(&self) -> u8;
}
impl Api for X {
    #[cfg(unix)] type Handle = u8;
    #[cfg(unix)] const ID: u8 = 1;
    #[cfg(test)] delegate!(probe(repo: &Path) -> u8 => fixture::probe);
}
unsafe extern "C" { #[cfg(unix)] safe fn getpid() -> i32; #[cfg(unix)] static environ: *const u8; }
#[cfg(unix)] macro_rules! noop ( () => () );
cfg_if::cfg_if! { if #[cfg(unix)] {} else { #[cfg(test)] probe![windows]; } }
'''), [('#[cfg(unix)]', 'use a::B'), ('#[cfg(unix)]', 'use a::{c, d as e}'),
       ('#[cfg(unix)]', 'extern crate alloc'), ('#[cfg(test)]', 'mod tests'),
       ('#[cfg(unix)]', 'type Alias'), ('#[cfg(unix)]', 'const LIMIT'), ('#[cfg(unix)]', 'static RAW'),
       ('#[cfg(unix)]', 'struct Unit'), ('#[cfg(unix)]', 'struct Pair'), ('#[cfg(unix)]', 'use a::F'),
       ('#[cfg(unix)]', 'type Handle'), ('#[cfg(unix)]', 'const ID'), ('#[cfg(unix)]', 'fn raw'),
       ('#[cfg(unix)]', 'type Handle'), ('#[cfg(unix)]', 'const ID'),
       ('#[cfg(test)]', key('delegate!(probe(repo: &Path) -> u8 => fixture::probe)')),
       ('#[cfg(unix)]', 'fn getpid'), ('#[cfg(unix)]', 'static environ'),
       ('#[cfg(unix)]', key('macro_rules! noop ( () => () )')), ('#[cfg(test)]', key('probe![windows]'))])

    def test_braced_items_pass(self):
        self.assertEqual(found('''
#[cfg(unix)] fn f() {}
#[cfg(unix)] pub(crate) const unsafe fn g() -> [u8; 2] { [0; 2] }
#[cfg(unix)] mod m {}
#[cfg(unix)] impl X {}
#[cfg(unix)] unsafe impl Send for X {}
#[cfg(unix)] struct S { a: u8 }
#[cfg(unix)] struct W<T> where T: Copy { t: T }
#[cfg(unix)] enum E { A }
#[cfg(unix)] union U { a: u8 }
#[cfg(unix)] trait T { fn provided(&self) {} }
#[cfg(unix)] extern "C" {}
#[cfg(unix)] cfg_if::cfg_if! { if #[cfg(test)] { use a::B; } }
#[cfg(test)] thread_local! { static SLOT: u8 = 0; }
#[cfg(unix)] macro_rules! m { () => {} }
'''), [])

    def test_unbraced_statements_are_flagged(self):
        # Consistency's snippet (C-P2-1): three statements, three occurrences.
        self.assertEqual(found('''
fn f() {
    #[cfg(not(unix))]
    let mode = 0;
    #[cfg(unix)]
    apply(mode);
    #[cfg(unix)]
    assert_eq!(mode, 0);
}
'''), [('#[cfg(not(unix))]', 'let mode'), ('#[cfg(unix)]', 'apply(mode)'), ('#[cfg(unix)]', 'assert_eq!(mode, 0)')])
        self.assertEqual(found('''
fn g() -> u8 {
    let run = || { #[cfg(test)] hit(1); };
    #[cfg(windows)] super::fault::fault(Fault::Rename);
    #[cfg(unix)] return 1;
    #[cfg_attr(test, allow(unused_mut))] let mut cases = vec![1];
    0
}
'''), [('#[cfg(test)]', 'hit(1)'), ('#[cfg(windows)]', 'super::fault::fault(Fault::Rename)'),
       ('#[cfg(unix)]', 'return 1'), ('#[cfg_attr(test, allow(unused_mut))]', 'let mut cases')])

    def test_let_statements_are_keyed_by_their_pattern(self):
        self.assertEqual(found('''
fn f() {
    #[cfg(unix)] let _ = metadata;
    #[cfg(unix)] let (a, b): (u8, u8) = pair;
    #[cfg(unix)] let mut total;
    #[cfg(unix)] let Some(x) = opt else { return; };
    #[cfg(unix)] let Point { x, y } = p;
    #[cfg(unix)] let run = || { 1 };
}
'''), [('#[cfg(unix)]', 'let _'), ('#[cfg(unix)]', 'let (a, b)'), ('#[cfg(unix)]', 'let mut total'),
       ('#[cfg(unix)]', 'let Some(x)'), ('#[cfg(unix)]', 'let ' + key('Point { x, y }')), ('#[cfg(unix)]', 'let run')])

    def test_replays_of_a_deleted_statement_flag_before_and_after(self):
        # Safety's replay pairs (S-P2-1): the attribute left behind is found on its new statement.
        before = 'fn f() {\n    #[cfg(unix)]\n    let mode = 0o755;\n    let path = build(mode);\n    use_it(path);\n}\n'
        after = 'fn f() {\n    #[cfg(unix)]\n    let path = build(0);\n    use_it(path);\n}\n'
        self.assertEqual(found(before), [('#[cfg(unix)]', 'let mode')])
        self.assertEqual(found(after), [('#[cfg(unix)]', 'let path')])
        before = 'fn f(path: &Path, mode: u32) { #[cfg(unix)] let mode = mode | 0o111; apply(path, mode); }'
        after = 'fn f(path: &Path, mode: u32) { #[cfg(unix)] apply(path, mode); }'
        self.assertEqual(found(before), [('#[cfg(unix)]', 'let mode')])
        self.assertEqual(found(after), [('#[cfg(unix)]', 'apply(path, mode)')])

    def test_braced_statements_and_statement_position_cfg_if_pass(self):
        self.assertEqual(found('''
fn f() {
    #[cfg(unix)] { let m = 1; }
    #[cfg(unix)] if x { y(); }
    #[cfg(unix)] match x { _ => {} }
    #[cfg(unix)] for i in 0..3 { y(i); }
    #[cfg(unix)] while x { y(); }
    #[cfg(unix)] loop { break; }
    #[cfg(unix)] unsafe { y(); }
    #[cfg(unix)] 'outer: loop { break 'outer; }
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            let mode = 0;
            apply(mode);
            assert_eq!(mode, 0);
        }
    }
    #[cfg(unix)] tail()
}
'''), [])
        # A bare cfg inside a statement-position arm is still directly on its statement.
        self.assertEqual(found('fn f() { cfg_if::cfg_if! { if #[cfg(unix)] { #[cfg(test)] let m = 1; } } }'),
                         [('#[cfg(test)]', 'let m')])

    def test_a_statement_after_an_inner_attribute_is_scanned(self):
        # C-P3-5 and S-P3-9: the start rule also accepts the `]` that closes an inner attribute.
        self.assertEqual(found('fn f() { #![allow(unused)] #[cfg(unix)] let a = 1; #[cfg(unix)] g(a); }'),
                         [('#[cfg(unix)]', 'let a'), ('#[cfg(unix)]', 'g(a)')])
        self.assertEqual(found('fn f() {\n    #![allow(unused)]\n    #[cfg(unix)]\n    let x = 1;\n    go(x);\n}\n'),
                         [('#[cfg(unix)]', 'let x')])
        self.assertEqual(found('mod m { #![cfg(windows)] use a::I; }'), [])

    def test_a_statement_is_braced_only_by_its_first_token(self):
        # S-P3-8: braces inside a statement's expression are not a boundary around the statement.
        statements = ['state = State { mode }', 'total = if a { 1 } else { 2 }', 'total = match a { 0 => 1, _ => 2 }',
                      'return S { a: 1 }', 'x = { compute() }', '*slot = S { a: 1 }', 'handler = move |x| { go(x) }',
                      'n += { 1 }']
        for statement in statements:
            self.assertEqual(found(f'fn f() {{ #[cfg(unix)] {statement}; next(); }}'),
                             [('#[cfg(unix)]', key(statement))], statement)
        self.assertEqual(found('fn f() { #[cfg(unix)] let s = S { a: 1 }; }'), [('#[cfg(unix)]', 'let s')])
        self.assertEqual(found('''
fn f() {
    #[cfg(unix)] 'block: { break 'block; }
    #[cfg(unix)] cfg_if::cfg_if! { if #[cfg(test)] { go(); } }
    #[cfg(unix)] probe! { x };
}
'''), [])

    def test_an_async_block_statement_is_not_braced(self):
        # An async block is an expression without a block, like a closure: its statement needs `;`.
        self.assertEqual(found('fn f() { #[cfg(unix)] async { go().await }.await; }'),
                         [('#[cfg(unix)]', 'async{go().await}.await')])
        self.assertEqual(found('fn f() { #[cfg(unix)] async move { go().await }.await; }'),
                         [('#[cfg(unix)]', 'async move{go().await}.await')])
        self.assertEqual(found('fn f() { #[cfg(unix)] async fn g() {} }'), [])

    def test_impl_and_extern_blocks_in_a_body_are_braced_items(self):
        # Not statements: an impl or extern block's body is its boundary, and the scan must not run past it.
        for block in ('impl Trait for X {}', 'unsafe impl Send for X {}', 'extern "C" { fn getpid() -> i32; }',
                      'unsafe extern "C" { safe fn getpid() -> i32; }'):
            self.assertEqual(found(f'fn f() {{ #[cfg(unix)] {block} go(); }}'), [], block)
            self.assertEqual(found(f'fn f() {{ #[cfg(unix)] {block} #[cfg(unix)] go(); }}'),
                             [('#[cfg(unix)]', 'go()')], block)

    def test_match_arms_after_a_block_bodied_arm_are_not_statements(self):
        # S-P3-8's controls: an arm after a block-bodied arm starts where a statement may, but no `;` ends it
        # before the match body closes. Shapes of the two real sites, then the sites themselves.
        self.assertEqual(found('''
fn clone_regular_file(self) -> Outcome {
    match self {
        Self::Native => platform::clone_regular_file(source, temporary),
        #[cfg(test)]
        Self::ScriptedUnsupported => {
            Outcome::Unsupported("scripted".to_owned())
        }
        #[cfg(test)]
        Self::ScriptedFailure(category) => {
            Outcome::Failed(category, "scripted native failure".to_owned())
        }
    }
}
fn late(change: LateImageChange) -> PathBuf {
    let path = match change {
        LateImageChange::Untracked => {
            PathBuf::from("late.txt")
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        LateImageChange::RawNonUtf8 => {
            let path = PathBuf::from("late-raw");
            path
        }
    };
    path
}
'''), [])
        for path, arm in (('crates/refcopy/src/native/attempt.rs', 'Self::ScriptedFailure(category) =>'),
                          ('src/git/tests/g15/root_preservation/stash.rs', 'LateImageChange::RawNonUtf8 =>')):
            source = (checker.ROOT / path).read_text(encoding='utf-8')
            self.assertIn(arm, source)
            self.assertEqual([o for o in checker.analyze(source) if o.item.startswith(arm.split('(')[0].split(' ')[0])],
                             [], path)

    def test_statement_keys_survive_reformatting(self):
        self.assertEqual(found('fn f() {\n    #[cfg(unix)]\n    assert_eq!(\n        mode,\n        0,\n    );\n}\n'),
                         found('fn f() { #[cfg(unix)] assert_eq!(mode, 0); }'))

    def test_fields_variants_arms_and_parameters_are_out_of_scope(self):
        # Neither items nor statements, and Rust gives them no enclosing boundary (S-P3-1).
        self.assertEqual(found('''
fn f(#[cfg(unix)] mode: u32) {
    match mode { #[cfg(unix)] 0 => {} _ => {} }
    match mode { #[cfg(unix)] 1 => a(), _ => b(), }
    let s = S { #[cfg(unix)] mode, other: 1 };
    let g = |#[cfg(unix)] p: u8| p;
}
struct S { #[cfg(unix)] pub mode: u32, #[cfg(unix)] r#type: u8 }
enum E { #[cfg(unix)] Unix, #[cfg(windows)] Windows(u8) }
'''), [])

    def test_const_generic_blocks_in_a_header_are_not_bodies(self):
        # Consistency's four snippets (C-P2-2) and its two controls.
        self.assertEqual(found('''
#[cfg(unix)] struct D<const N: usize = { 2 }>;
trait T { #[cfg(unix)] fn f() -> Foo<{ N }>; }
#[cfg(unix)] struct W<T>(T) where T: Trait<{ N }>;
impl Foo<{ N }> for Bar { #[cfg(test)] delegate!(x => y); }
#[cfg(unix)] const X: u8 = { 3 };
#[cfg(unix)] fn g() -> Foo<N> { body() }
'''), [('#[cfg(unix)]', 'struct D'), ('#[cfg(unix)]', 'fn f'), ('#[cfg(unix)]', 'struct W'),
       ('#[cfg(test)]', key('delegate!(x => y)')), ('#[cfg(unix)]', 'const X')])

    def test_a_where_clauses_trailing_comma_is_not_a_generic_argument(self):
        # rustfmt ends a vertical where clause with a comma just before the body.
        self.assertEqual(found('''
#[cfg(test)]
fn f<T>()
where
    T: Clone,
{
    body();
}
use a::B;
#[cfg(test)]
struct S<T>
where
    T: Clone,
{
    a: T,
}
impl<T> X for Y<T>
where
    T: Clone,
{
    #[cfg(test)]
    delegate!(z);
}
'''), [('#[cfg(test)]', 'delegate!(z)')])

    def test_a_macro_path_may_start_with_colons(self):
        self.assertEqual(found('#[cfg(test)] ::probe::fixture!(x);'), [('#[cfg(test)]', '::probe::fixture!(x)')])

    def test_non_ascii_names_are_identifiers(self):
        self.assertEqual(found('#[cfg(unix)] mod über;\n#[cfg(unix)] const ünit: u8 = 0;\n#[cfg(unix)] type Ärger = u8;\n'),
                         [('#[cfg(unix)]', 'mod über'), ('#[cfg(unix)]', 'const ünit'), ('#[cfg(unix)]', 'type Ärger')])

    def test_macro_items_are_keyed_by_their_whole_invocation(self):
        listed = 'impl X { #[cfg(test)] delegate!(a() -> u8 => f::a); #[cfg(test)] delegate!(b() -> u8 => f::b); }'
        swapped = listed.replace('b() -> u8 => f::b', 'zzz_new(repo: &Path) -> Vec<u8> => other::zzz')
        self.assertEqual(len(set(found(listed))), 2)
        self.assertNotEqual(found(listed), found(swapped))

    def test_stacked_attributes_and_comments_still_attach(self):
        self.assertEqual(found('''
#[cfg(test)]
#[allow(unused_imports)]
use a::B;
#[allow(dead_code)]
// the probe
/// Docs.
#[cfg(unix)]
/* note */ #[doc = "x"]
const PROBE: u8 = 0;
#[cfg(unix)]
#[cfg(target_os = "linux")]
#[path = "linux_impl.rs"]
mod linux;
'''), [('#[cfg(test)]', 'use a::B'), ('#[cfg(unix)]', 'const PROBE'),
       ('#[cfg(unix)] #[cfg(target_os = "linux")]', 'mod linux')])

    def test_cfg_attr_is_conditional_when_its_effect_depends_on_the_configuration(self):
        self.assertEqual(found('''
#[cfg_attr(test, allow(unused_imports))] use a::A;
#[cfg_attr(windows, path = "win.rs")] mod imp;
#[cfg_attr(all(), cfg(unix))] use a::B;
#[cfg_attr(all(), allow(unused_imports))] use a::C;
#[cfg_attr(any(), cfg(unix))] use a::D;
#[cfg_attr(not(any()), cfg_attr(unix, allow(dead_code)))] use a::E;
#[cfg_attr(unix,)] use a::F;
#[cfg_attr(all(unix, any()), cfg(test))] use a::G;
'''), [('#[cfg_attr(test, allow(unused_imports))]', 'use a::A'),
       ('#[cfg_attr(windows, path = "win.rs")]', 'mod imp'),
       ('#[cfg_attr(all(), cfg(unix))]', 'use a::B'),
       ('#[cfg_attr(not(any()), cfg_attr(unix, allow(dead_code)))]', 'use a::E')])

    def test_ignores_comments_literals_and_inner_attributes(self):
        self.assertEqual(found('''
#![cfg(unix)]
// #[cfg(unix)] use a::A;
/* #[cfg(unix)] /* nested */ use a::B; */
/// #[cfg(unix)]
//! #[cfg(unix)] use a::C;
/** #[cfg(unix)] use a::D; */
const TEXT: &str = "#[cfg(unix)] use a::E; \\" #[cfg(unix)] use a::F;";
const RAW: &str = r##"#[cfg(unix)] use a::"#G;"##;
const BYTES: &[u8] = br#"#[cfg(unix)] use a::H;"#;
const HASH: char = '#';
const BRACKET: u8 = b'[';
fn f<'a>(x: &'a str) { #![cfg(unix)] }
mod m { #![cfg(windows)] use a::I; }
'''), [])

    def test_line_numbers_point_at_the_first_conditional_attribute(self):
        [occurrence] = checker.analyze('\n\n#[allow(unused)]\n#[cfg(test)]\nuse a::B;\n')
        self.assertEqual(occurrence.line, 4)


def write(root, files):
    for name, text in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(text, bytes):
            path.write_bytes(text)
        else:
            path.write_text(text, encoding='utf-8')


def run_main(argv):
    stdout, stderr = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        status = checker.main(argv)
    return status, stdout.getvalue(), stderr.getvalue()


IMPORT = '#[cfg(windows)]\nuse std::os::windows::ffi::OsStrExt;\n'
REPOS = {'core': {'path': '.', 'roots': ['.']}, 'cli': {'path': '../cli', 'roots': ['src']}}


class Ratchet(unittest.TestCase):
    def run_check(self, entries, files, skip=(), repos=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'core'
            write(Path(directory), files)
            allowlist = Path(directory) / 'allowlist.json'
            allowlist.write_text(json.dumps({'repos': repos or REPOS, 'entries': entries}), encoding='utf-8')
            return checker.check(root, allowlist, skip)[0]

    def entry(self, **overrides):
        entry = {'repo': 'core', 'path': 'src/lib.rs', 'attrs': WINDOWS_IMPORT[0], 'item': WINDOWS_IMPORT[1]}
        entry.update(overrides)
        return entry

    def test_listed_occurrence_passes_and_a_new_one_fails(self):
        files = {'core/src/lib.rs': IMPORT, 'cli/src/main.rs': ''}
        self.assertEqual(self.run_check([self.entry()], files), [])
        files['cli/src/main.rs'] = 'fn main() {}\n' + IMPORT
        errors = self.run_check([self.entry()], files)
        self.assertEqual(errors, ['NEW   #[cfg(windows)] use std::os::windows::ffi::OsStrExt at cli/src/main.rs:2'])

    def test_a_listed_occurrence_that_disappears_fails(self):
        files = {'core/src/lib.rs': 'cfg_if::cfg_if! { if #[cfg(windows)] { use std::os::windows::ffi::OsStrExt; } }\n',
                 'cli/src/main.rs': ''}
        errors = self.run_check([self.entry()], files)
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('STALE #[cfg(windows)] use std::os::windows::ffi::OsStrExt in core/src/lib.rs'))
        self.assertIn('remove it from the allowlist', errors[0])

    def test_keys_survive_line_drift_and_reformatting(self):
        drifted = '//! Moved down.\n\nfn f() {}\n\n#[cfg( windows )]\n\n  use std::os::windows::ffi ::OsStrExt ;\n'
        self.assertEqual(self.run_check([self.entry()], {'core/src/lib.rs': drifted, 'cli/src/main.rs': ''}), [])
        # rustfmt adds a trailing comma when it wraps a list and drops it when it joins one.
        self.assertEqual(found('#[cfg(any(\n    test,\n))]\nuse a::{\n    b,\n    c,\n};\n'),
                         found('#[cfg(any(test))] use a::{b, c};'))

    def test_a_modified_occurrence_is_new(self):
        errors = self.run_check([self.entry()], {'core/src/lib.rs': IMPORT.replace('windows)', 'not(unix))'),
                                                 'cli/src/main.rs': ''})
        self.assertEqual([error[:5] for error in errors], ['NEW  ', 'STALE'])

    def test_a_swapped_macro_body_is_new_and_stale(self):
        listed = 'impl X {\n    #[cfg(test)]\n    delegate!(a() -> u8 => f::a);\n}\n'
        entry = self.entry(attrs='#[cfg(test)]', item=key('delegate!(a() -> u8 => f::a)'))
        self.assertEqual(self.run_check([entry], {'core/src/lib.rs': listed, 'cli/src/main.rs': ''}), [])
        swapped = listed.replace('a() -> u8 => f::a', 'zzz_new(repo: &Path) -> Vec<u8> => other::zzz')
        errors = self.run_check([entry], {'core/src/lib.rs': swapped, 'cli/src/main.rs': ''})
        self.assertEqual([error[:5] for error in errors], ['NEW  ', 'STALE'])

    def test_counts_must_match_exactly(self):
        files = {'core/src/lib.rs': IMPORT + 'mod m {\n' + IMPORT + '}\n', 'cli/src/main.rs': ''}
        self.assertTrue(self.run_check([self.entry()], files)[0].startswith('COUNT'))
        self.assertEqual(self.run_check([self.entry(count=2)], files), [])

    def test_entries_are_validated(self):
        errors = self.run_check([self.entry(), self.entry(), self.entry(repo='elsewhere'), self.entry(item=''),
                                 self.entry(path='src/other.rs', count=0)],
                                {'core/src/lib.rs': IMPORT, 'cli/src/main.rs': ''})
        self.assertEqual(len(errors), 4, errors)

    def test_build_output_and_hidden_directories_are_skipped(self):
        files = {'core/Cargo.toml': '', 'core/target/debug/build/gen.rs': IMPORT, 'core/.venv/lib/site.rs': IMPORT,
                 'core/src/target/mod.rs': IMPORT, 'cli/src/main.rs': ''}
        errors = self.run_check([], files)
        self.assertEqual(errors, ['NEW   #[cfg(windows)] use std::os::windows::ffi::OsStrExt at core/src/target/mod.rs:1'])

    def test_a_missing_sibling_fails_closed_and_the_skip_flag_says_so(self):
        files = {'core/src/lib.rs': IMPORT}
        errors = self.run_check([self.entry(), self.entry(repo='cli', path='src/main.rs')], files)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('--skip-repo cli', errors[0])
        self.assertEqual(self.run_check([self.entry(), self.entry(repo='cli', path='src/main.rs')], files, ['cli']), [])
        self.assertTrue(self.run_check([self.entry()], files, ['cli', 'nowhere'])[0].startswith('--skip-repo nowhere'))

    def test_the_checks_own_repository_cannot_be_skipped(self):
        errors = self.run_check([self.entry()], {'core/src/lib.rs': IMPORT, 'cli/src/main.rs': ''}, ['core'])
        self.assertEqual(len(errors), 1, errors)
        self.assertTrue(errors[0].startswith('--skip-repo core: refused'), errors)
        self.assertIn("the check's own repository", errors[0])
        status, _, stderr = run_main(['--skip-repo', 'gwz-core'])
        self.assertEqual(status, 1)
        self.assertIn("--skip-repo gwz-core: refused", stderr)

    def test_a_run_that_scans_no_file_fails(self):
        siblings = {'cli': {'path': '../cli', 'roots': ['src']}}
        errors = self.run_check([], {'cli/src/main.rs': ''}, ['cli'], siblings)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('no Rust file was scanned', errors[0])
        errors = self.run_check([], {'core/docs/a.md': '', 'cli/src/README': ''},
                                repos={'core': {'path': '.', 'roots': ['docs']}})
        self.assertIn('no Rust file was scanned', errors[-1])

    def test_main_prints_skipped_gate_and_fails_on_new_occurrences(self):
        with tempfile.TemporaryDirectory() as directory:
            write(Path(directory), {'core/src/lib.rs': IMPORT})
            allowlist = Path(directory) / 'allowlist.json'
            allowlist.write_text(json.dumps({'repos': REPOS, 'entries': []}), encoding='utf-8')
            for argv, printed in ((['--skip-repo', 'cli'], 'SKIPPED GATE'), ([], 'MISSING cli')):
                status, stdout, stderr = run_main(['--root', str(Path(directory) / 'core'),
                                                   '--allowlist', str(allowlist), *argv])
                self.assertEqual(status, 1)
                self.assertIn(printed, stdout + stderr)
                self.assertIn('NEW   #[cfg(windows)]', stderr)

    def test_a_malformed_allowlist_or_an_undecodable_file_fails_closed(self):
        # Safety §3: each path fails with an error, not an uncaught exception.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'core'
            write(Path(directory), {'core/src/lib.rs': '', 'cli/src/main.rs': ''})
            allowlist = Path(directory) / 'allowlist.json'
            for text in ('{"repos": {', json.dumps({'entries': []}), '[]'):
                allowlist.write_text(text, encoding='utf-8')
                status, _, stderr = run_main(['--root', str(root), '--allowlist', str(allowlist)])
                self.assertEqual(status, 1, text)
                self.assertIn('boundary guard failed', stderr)
            allowlist.write_text(json.dumps({'repos': REPOS, 'entries': []}), encoding='utf-8')
            write(root, {'src/bad.rs': b'fn f() { let s = "\xff"; }\n'})
            status, _, stderr = run_main(['--root', str(root), '--allowlist', str(allowlist)])
            self.assertEqual(status, 1)
            self.assertIn('UNREADABLE core/src/bad.rs', stderr)


class ShrinkFrom(unittest.TestCase):
    """--shrink-from BASE: the allowlist may drop entries or lower counts, never add or raise them (S-P3-3)."""

    def entry(self, item='delegate!(a)', count=None):
        entry = {'repo': 'core', 'path': 'src/lib.rs', 'attrs': '#[cfg(test)]', 'item': item}
        if count is not None:
            entry['count'] = count
        return entry

    def shrink(self, current, base, repos=REPOS, base_repos=REPOS):
        with tempfile.TemporaryDirectory() as directory:
            now, before = Path(directory) / 'now.json', Path(directory) / 'base.json'
            now.write_text(json.dumps({'repos': repos, 'entries': current}), encoding='utf-8')
            if base is not None:
                before.write_text(base if isinstance(base, str) else json.dumps({'repos': base_repos, 'entries': base}),
                                  encoding='utf-8')
            return run_main(['--allowlist', str(now), '--shrink-from', str(before)])

    def test_debt_moved_between_files_passes(self):
        # C-P3-4: totals per repository, attributes and target, summed over files, as a movement-only split keeps them.
        base = [self.entry(item='use x::Y') | {'path': 'src/a.rs'}]
        moved = [self.entry(item='use x::Y') | {'path': 'src/b.rs'}]
        self.assertEqual(self.shrink(moved, base)[0], 0)
        status, _, stderr = self.shrink(base + moved, base)
        self.assertEqual(status, 1)
        self.assertIn('RAISED #[cfg(test)] use x::Y in core/src/a.rs, core/src/b.rs: count 2, base 1', stderr)
        status, _, stderr = self.shrink([self.entry(item='use x::Y', count=2) | {'path': 'src/b.rs'}], base)
        self.assertEqual(status, 1)
        self.assertIn('RAISED #[cfg(test)] use x::Y in core/src/b.rs: count 2, base 1', stderr)

    def test_the_docstring_and_the_rule_state_what_shrinking_means(self):
        # Summing over files is a recorded trade-off: remove-and-recreate elsewhere in a repository passes.
        rule = json.loads(checker.DEFAULT_ALLOWLIST.read_text(encoding='utf-8'))['rule']
        for text in (' '.join(checker.__doc__.split()), rule):
            self.assertIn('a movement-only split cannot be told apart from remove-and-recreate', text)
            self.assertIn('move claim to verify', text)
        self.assertNotIn('fails on an added entry or a raised count', rule)
        self.assertIn('fails on an added occurrence (a count, summed over files, above the base', rule)
        self.assertIn('or a narrowed scope', rule)

    def test_a_narrowed_scope_fails_naming_the_change(self):
        # S-P3-7: the debt only shrinks, and the scanned scope never does.
        real = json.loads(checker.DEFAULT_ALLOWLIST.read_text(encoding='utf-8'))['repos']
        entries = [{'repo': 'gwz-core', 'path': 'src/lib.rs', 'attrs': '#[cfg(test)]', 'item': 'mod tests'}]
        dropped = {name: repo for name, repo in real.items() if name != 'gwz-cli'}
        narrowed = real | {'gwz-core': {'path': '.', 'roots': ['src']}}
        moved = real | {'gwz-cli': {'path': '../gwz-cli-2', 'roots': ['.']}}
        for repos, message in ((dropped, 'REMOVED repository gwz-cli'),
                               (narrowed, 'NARROWED repository gwz-core: base root . is not under the current roots src'),
                               (moved, 'MOVED repository gwz-cli: path ../gwz-cli in the base, ../gwz-cli-2 now')):
            status, _, stderr = self.shrink(entries, entries, repos, real)
            self.assertEqual(status, 1, message)
            self.assertIn(message, stderr)
        added = real | {'gwz-transport': {'path': '../gwz-transport', 'roots': ['src']}}
        rooted = real | {'gwz-py': {'path': '../gwz-py', 'roots': ['native', 'scripts']}}
        widened = {'gwz-core': {'path': './', 'roots': ['.']}}
        for repos, base_repos in ((added, real), (rooted, real), (real, real),
                                  (widened, {'gwz-core': {'path': '.', 'roots': ['src', 'crates/x']}})):
            self.assertEqual(self.shrink(entries, entries, repos, base_repos)[0], 0, repos)

    def test_an_added_key_or_a_raised_count_fails(self):
        base = [self.entry(count=14)]
        status, _, stderr = self.shrink(base + [self.entry(item='use a::New')], base)
        self.assertEqual(status, 1)
        self.assertIn('ADDED  #[cfg(test)] use a::New in core/src/lib.rs', stderr)
        status, _, stderr = self.shrink([self.entry(count=15)], base)
        self.assertEqual(status, 1)
        self.assertIn('RAISED #[cfg(test)] delegate!(a) in core/src/lib.rs: count 15, base 14', stderr)

    def test_a_removed_key_or_a_lowered_count_passes(self):
        base = [self.entry(count=14), self.entry(item='use a::Gone')]
        self.assertEqual(self.shrink([self.entry(count=13)], base)[0], 0)
        self.assertEqual(self.shrink([self.entry(count=14)], base)[0], 0)
        self.assertEqual(self.shrink([], base)[0], 0)

    def test_a_missing_base_passes_with_a_message_and_a_malformed_one_fails(self):
        status, stdout, _ = self.shrink([self.entry()], None)
        self.assertEqual(status, 0)
        self.assertIn('no base allowlist', stdout)
        self.assertEqual(self.shrink([self.entry()], '{"repos"')[0], 1)


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
    return ['\n'.join(step) for step in steps]


class ShrinkWorkflow(unittest.TestCase):
    def test_boundary_job_runs_shrink_from_on_pull_requests_and_pushes(self):
        text = BOUNDARY_WORKFLOW.read_text(encoding='utf-8')
        self.assertRegex(text, r'\non:\n  pull_request:\n  push:\n')
        steps = job_steps(text, 'boundary')
        self.assertIn('fetch-depth: 0', steps[0])
        shrinking = [step for step in steps if '--shrink-from' in step]
        self.assertEqual(len(shrinking), 1)
        step = shrinking[0]
        condition = re.search(r'^        if: (.*)$', step, re.M).group(1)
        self.assertIn("github.event_name == 'pull_request'", condition)
        self.assertIn("github.event_name == 'push' && github.event.before != '0000000000000000000000000000000000000000'",
                      condition)
        base = re.search(r'^          BASE_SHA: (.*)$', step, re.M).group(1)
        self.assertEqual(base, "${{ github.event_name == 'pull_request' && github.event.pull_request.base.sha "
                               "|| github.event.before }}")
        self.assertIn('git rev-parse --verify "$BASE_SHA^{commit}"', step)
        self.assertIn('git show "$BASE_SHA:scripts/checks/cfg_boundaries_allowlist.json" > "$base"', step)
        self.assertIn('python scripts/checks/check_cfg_boundaries.py --shrink-from "$base"', step)
        self.assertLess(steps.index(step), next(i for i, s in enumerate(steps) if 'Install Rust toolchain' in s))


class Runner(unittest.TestCase):
    """scripts/run_tests.py runs the check over all three trees, or says SKIPPED GATE for the siblings."""

    def cfg_arguments(self, argv):
        with patch.object(runner.subprocess, 'run') as run, contextlib.redirect_stdout(io.StringIO()):
            run.return_value.returncode = 0
            with self.assertRaises(SystemExit):
                runner.main(['--skip-transport-globals', *argv])
        commands = [[str(part) for part in call.args[0]] for call in run.call_args_list]
        return [command[2:] for command in commands if command[1].endswith('check_cfg_boundaries.py')]

    def test_checks_every_tree_by_default(self):
        self.assertEqual(self.cfg_arguments([]), [[]])

    def test_skip_flag_skips_only_the_siblings(self):
        self.assertEqual(self.cfg_arguments(['--skip-cfg-siblings']),
                         [['--skip-repo', 'gwz-cli', '--skip-repo', 'gwz-py']])

    def test_an_abbreviated_flag_does_not_skip(self):
        self.assertEqual(self.cfg_arguments(['--skip-cfg']), [[]])


class Repository(unittest.TestCase):
    def test_trees_add_no_unbraced_conditional_declaration(self):
        repos = json.loads(checker.DEFAULT_ALLOWLIST.read_text(encoding='utf-8'))['repos']
        absent = [name for name, repo in repos.items() if not (checker.ROOT / repo['path']).is_dir()]
        errors, _, _ = checker.check(checker.ROOT, checker.DEFAULT_ALLOWLIST, absent)
        self.assertEqual(errors, [], '\n'.join(errors))
        if absent:
            self.skipTest(f'not checked out beside gwz-core, so not checked: {", ".join(absent)}')


if __name__ == '__main__':
    unittest.main()
