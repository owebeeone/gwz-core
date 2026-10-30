"""Tests for the shrink-only candidate-shape clippy gate (operator decision D-U, 2026-10-31).

They feed synthetic `cargo clippy --message-format=json` lines to the comparison; no test runs clippy.
"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('candidate_clippy', Path(__file__).with_name('check_candidate_clippy.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

SOURCE = '''use std::io;
use std::{
    fmt,
    sync,
};

pub struct Rig {
    unused: u8,
}

impl Rig {
    pub fn first(&self) -> u8 {
        let x = 1;
        x
    }

    fn second(&self) {
        fn helper() {
            let y = 2;
        }
        let z = 3;
    }
}

impl<T: Clone> Holder<T>
where
    T: Send,
{
    fn third(&self) {
        let w = 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row() {
        let v = 5;
    }
}

pub const LIMIT: u32 = 3;
'''


def line_of(text):
    """The 1-based line of the first source line containing `text`."""
    for number, line in enumerate(SOURCE.splitlines(), start=1):
        if text in line:
            return number
    raise AssertionError(text)


def message(lint, file_name='src/rig.rs', line=1, level='warning', unit=('gwz_core', 'lib'), expansion=None,
            is_primary=True, spans=None):
    span = {'file_name': file_name, 'line_start': line, 'line_end': line, 'is_primary': is_primary,
            'expansion': expansion}
    return json.dumps({
        'reason': 'compiler-message', 'package_id': 'path+file:///x#gwz-core@1.0.0',
        'target': {'kind': [unit[1]], 'name': unit[0]},
        'message': {'code': {'code': lint} if lint else None, 'level': level,
                    'message': f'{lint} fired', 'spans': [span] if spans is None else spans}})


def counts(lines, source=SOURCE):
    with tempfile.TemporaryDirectory() as root:
        (Path(root) / 'src').mkdir()
        (Path(root) / 'src' / 'rig.rs').write_text(source, encoding='utf-8')
        return gate.count_keys(lines, gate.SourceCache(root))


class EnclosingItem(unittest.TestCase):
    lines = SOURCE.splitlines()

    def item(self, text):
        return gate.enclosing_item(self.lines, line_of(text))

    def test_a_method_is_inside_its_impl(self):
        self.assertEqual(self.item('let x = 1'), 'impl Rig > fn first')

    def test_the_header_line_names_the_item_it_declares(self):
        self.assertEqual(self.item('pub fn first'), 'impl Rig > fn first')
        self.assertEqual(self.item('unused: u8'), 'struct Rig')

    def test_a_sibling_that_ended_earlier_does_not_enclose(self):
        self.assertEqual(self.item('let z = 3'), 'impl Rig > fn second')

    def test_an_inner_fn_encloses_only_its_own_body(self):
        self.assertEqual(self.item('let y = 2'), 'impl Rig > fn second > fn helper')

    def test_a_multi_line_impl_header_keeps_its_bounds(self):
        self.assertEqual(self.item('let w = 4'), 'impl<T: Clone> Holder<T> > fn third')

    def test_test_modules_are_part_of_the_chain(self):
        self.assertEqual(self.item('let v = 5'), 'mod tests > fn a_row')
        self.assertEqual(self.item('use super::*'), 'mod tests')

    def test_file_level_lines_have_no_item(self):
        self.assertEqual(self.item('use std::io'), '')
        self.assertEqual(self.item('fmt,'), '')

    def test_a_const_is_an_item(self):
        self.assertEqual(self.item('pub const LIMIT'), 'const LIMIT')

    def test_a_line_outside_the_file_has_no_item(self):
        self.assertEqual(gate.enclosing_item(self.lines, 0), '')
        self.assertEqual(gate.enclosing_item(self.lines, 10_000), '')
        self.assertEqual(gate.enclosing_item([], 3), '')


class Keys(unittest.TestCase):
    def test_a_key_is_lint_file_item_and_unit_and_counts_repeats(self):
        found = counts([message('clippy::collapsible_if', line=line_of('let x = 1')),
                        message('clippy::collapsible_if', line=line_of('let x = 1')),
                        message('dead_code', line=line_of('unused: u8'))])
        self.assertEqual(found, {
            ('clippy::collapsible_if', 'src/rig.rs', 'impl Rig > fn first', 'gwz_core:lib'): 2,
            ('dead_code', 'src/rig.rs', 'struct Rig', 'gwz_core:lib'): 1})

    def test_a_key_does_not_move_with_the_line_number(self):
        first = counts([message('clippy::ptr_arg', line=line_of('let x = 1'))])
        moved = counts([message('clippy::ptr_arg', line=line_of('let x = 1') + 40)], '\n' * 40 + SOURCE)
        self.assertEqual(list(first.values()), [1])
        self.assertEqual(first, moved)

    def test_the_unit_is_the_cargo_target(self):
        found = counts([message('clippy::ptr_arg', unit=('protocol', 'test')),
                        message('clippy::ptr_arg', unit=('gwz_core', 'rlib')),
                        message('clippy::ptr_arg', unit=('build-script-build', 'custom-build'))])
        self.assertEqual(sorted(unit for *_, unit in found),
                         ['build-script-build:custom-build', 'gwz_core:lib', 'protocol:test'])

    def test_errors_and_warnings_count_and_notes_and_summaries_do_not(self):
        found = counts([message('clippy::ptr_arg', level='error'), message('clippy::ptr_arg', level='warning'),
                        message('clippy::ptr_arg', level='note'), message('clippy::ptr_arg', level='help'),
                        message(None, level='error', spans=[]),
                        json.dumps({'reason': 'compiler-artifact'}), 'not json', ''])
        self.assertEqual(sum(found.values()), 2)

    def test_a_code_with_no_place_is_still_a_key(self):
        found = counts([message('E0425', spans=[])])
        self.assertEqual(list(found), [('E0425', '', '', 'gwz_core:lib')])

    def test_a_diagnostic_without_a_code_keys_on_its_message(self):
        found = counts([message(None, level='error', line=3)])
        self.assertEqual([key[0] for key in found], ['error: None fired'])

    def test_a_macro_expansion_resolves_to_the_call_site_in_the_repository(self):
        site = {'file_name': 'src/rig.rs', 'line_start': line_of('let w = 4'), 'is_primary': True, 'expansion': None}
        found = counts([message('clippy::ptr_arg', file_name='/rustc/abc/library/std/src/macros.rs', line=7,
                                expansion={'span': site, 'macro_decl_name': 'vec!'})])
        self.assertEqual(list(found), [('clippy::ptr_arg', 'src/rig.rs', 'impl<T: Clone> Holder<T> > fn third',
                                        'gwz_core:lib')])

    def test_a_secondary_span_is_not_the_place(self):
        secondary = {'file_name': 'src/other.rs', 'line_start': 1, 'is_primary': False, 'expansion': None}
        primary = {'file_name': 'src/rig.rs', 'line_start': line_of('let z = 3'), 'is_primary': True,
                   'expansion': None}
        found = counts([message('clippy::ptr_arg', spans=[secondary, primary])])
        self.assertEqual([key[1] for key in found], ['src/rig.rs'])

    def test_windows_separators_and_dot_prefixes_normalise(self):
        found = counts([message('clippy::ptr_arg', file_name='.\\src\\rig.rs', line=line_of('let x = 1'))])
        # the source file is src/rig.rs on every host; the key is spelt with forward slashes
        self.assertEqual([key[1] for key in found], ['src/rig.rs'])

    def test_a_missing_source_file_leaves_the_item_empty(self):
        found = counts([message('clippy::ptr_arg', file_name='src/gone.rs', line=5)])
        self.assertEqual(list(found), [('clippy::ptr_arg', 'src/gone.rs', '', 'gwz_core:lib')])


class Comparison(unittest.TestCase):
    A = ('clippy::a', 'src/a.rs', 'fn a', 'gwz_core:lib')
    B = ('clippy::b', 'src/b.rs', '', 'gwz_core:lib')
    C = ('dead_code', 'src/c.rs', 'fn c', 'gwz_core:lib')

    def test_equal_is_clean(self):
        self.assertEqual(gate.compare({self.A: 2, self.B: 1}, {self.A: 2, self.B: 1}), ([], [], [], []))

    def test_a_count_that_rose_grew(self):
        grew, new, shrank, gone = gate.compare({self.A: 3}, {self.A: 2})
        self.assertEqual((grew, new, shrank, gone), ([(self.A, 2, 3)], [], [], []))

    def test_a_key_not_in_the_list_is_new(self):
        grew, new, shrank, gone = gate.compare({self.A: 1, self.C: 1}, {self.A: 1})
        self.assertEqual((grew, new, shrank, gone), ([], [(self.C, 1)], [], []))

    def test_a_count_that_fell_or_a_key_that_vanished_is_reported_not_failed(self):
        grew, new, shrank, gone = gate.compare({self.A: 1}, {self.A: 3, self.B: 2})
        self.assertEqual((grew, new, shrank, gone), ([], [], [(self.A, 3, 1)], [(self.B, 2)]))

    def test_one_fix_does_not_buy_a_new_error_elsewhere(self):
        # B fell by one, C is new: the fall is not credited against the new key.
        grew, new, _, _ = gate.compare({self.B: 1, self.C: 1}, {self.B: 2})
        self.assertEqual(new, [(self.C, 1)])


class Allowlist(unittest.TestCase):
    def write(self, root, entries, shape='macos'):
        path = Path(root) / 'allow.json'
        path.write_text(json.dumps({'shapes': {shape: {'entries': entries}}}), encoding='utf-8')
        return path

    def entry(self, **changes):
        base = {'lint': 'clippy::a', 'file': 'src/a.rs', 'item': 'fn a', 'unit': 'gwz_core:lib', 'count': 1,
                'owner': '6.1', 'class': 'style'}
        base.update(changes)
        return base

    def test_a_good_list_loads(self):
        with tempfile.TemporaryDirectory() as root:
            data, errors = gate.load_allowlist(self.write(root, [self.entry(), self.entry(item='fn b')]))
        self.assertEqual(errors, [])
        self.assertEqual(gate.allowed_counts(data, 'macos'),
                         {('clippy::a', 'src/a.rs', 'fn a', 'gwz_core:lib'): 1,
                          ('clippy::a', 'src/a.rs', 'fn b', 'gwz_core:lib'): 1})

    def test_an_entry_needs_an_owner_a_class_and_a_positive_count(self):
        for change in ({'owner': ''}, {'class': ''}, {'count': 0}, {'count': True}, {'count': '2'}, {'lint': None}):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as root:
                data, errors = gate.load_allowlist(self.write(root, [self.entry(**change)]))
                self.assertIsNone(data)
                self.assertTrue(errors)

    def test_a_duplicate_key_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            data, errors = gate.load_allowlist(self.write(root, [self.entry(), self.entry()]))
        self.assertIsNone(data)
        self.assertIn('duplicate', errors[0])

    def test_a_list_without_shapes_or_unreadable_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            bare = Path(root) / 'bare.json'
            bare.write_text(json.dumps({'rule': 'x'}), encoding='utf-8')
            self.assertIn("'shapes' object", gate.load_allowlist(bare)[1][0])
            self.assertTrue(gate.load_allowlist(Path(root) / 'missing.json')[1])

    def test_the_checked_in_list_has_a_shape_per_os(self):
        data, errors = gate.load_allowlist(gate.DEFAULT_ALLOWLIST)
        self.assertEqual(errors, [])
        self.assertTrue({'macos', 'linux'} <= set(data['shapes']))
        self.assertNotIn('host', data['shapes'])

    def test_the_default_shape_follows_the_platform(self):
        for platform, shape in (('darwin', 'macos'), ('linux', 'linux'), ('win32', 'windows')):
            with self.subTest(platform=platform), mock.patch.object(gate.sys, 'platform', platform):
                self.assertEqual(gate.platform_shape(), shape)
                self.assertEqual(gate.parse_arguments([]).shape, shape)
        with mock.patch.object(gate.sys, 'platform', 'linux'):
            self.assertEqual(gate.parse_arguments(['--shape', 'macos']).shape, 'macos')

    def test_the_checked_in_list_is_well_formed_and_owned_by_step_6_1(self):
        data, errors = gate.load_allowlist(gate.DEFAULT_ALLOWLIST)
        self.assertEqual(errors, [])
        for shape, body in data['shapes'].items():
            for entry in body['entries']:
                self.assertEqual(entry['owner'], '6.1', (shape, entry))
                self.assertIn(entry['class'], {'style', 'dead-code', 'design', 'concurrency'}, (shape, entry))
                self.assertNotIn('line', entry)

    def test_a_baseline_keeps_owner_and_class_of_entries_that_remain(self):
        key = ('clippy::a', 'src/a.rs', 'fn a', 'gwz_core:lib')
        fresh = gate.entries_for({key: 4, ('dead_code', 'src/z.rs', '', 'gwz_core:lib'): 1},
                                 [self.entry(owner='6.2', **{'class': 'design'})])
        self.assertEqual([(e['lint'], e['count'], e['owner'], e['class']) for e in fresh],
                         [('clippy::a', 4, '6.2', 'design'), ('dead_code', 1, '6.1', 'dead-code')])


class Gate(unittest.TestCase):
    """main() over saved JSON lines and a temporary tree: the exit codes the workflow sees."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.root = Path(self.dir.name)
        (self.root / 'src').mkdir()
        (self.root / 'src' / 'rig.rs').write_text(SOURCE, encoding='utf-8')
        self.allow = self.root / 'allow.json'
        self.json = self.root / 'clippy.json'

    def run_gate(self, lines, entries, *extra, platform_default=False):
        # The list is written under 'macos'; pass that shape unless the test names one or wants the platform's.
        if '--shape' not in extra and not platform_default:
            extra = ('--shape', 'macos', *extra)
        self.allow.write_text(json.dumps({'shapes': {'macos': {'entries': entries}}}), encoding='utf-8')
        self.json.write_text('\n'.join(lines), encoding='utf-8')
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = gate.main(['--allowlist', str(self.allow), '--json', str(self.json),
                              '--source-root', str(self.root), *extra])
        return code, out.getvalue(), err.getvalue()

    def entry(self, lint, line_text, count, item):
        return {'lint': lint, 'file': 'src/rig.rs', 'item': item, 'unit': 'gwz_core:lib', 'count': count,
                'owner': '6.1', 'class': 'style'}

    def lines(self, lint, text, times=1):
        return [message(lint, line=line_of(text))] * times

    def test_the_baseline_is_green(self):
        code, out, err = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1', 2),
                                       [self.entry('clippy::ptr_arg', 'let x = 1', 2, 'impl Rig > fn first')])
        self.assertEqual((code, err), (0, ''))
        self.assertIn('nothing new', out)

    def test_a_new_error_is_red_and_named(self):
        code, _, err = self.run_gate(
            self.lines('clippy::ptr_arg', 'let x = 1') + self.lines('clippy::collapsible_if', 'let z = 3'),
            [self.entry('clippy::ptr_arg', 'let x = 1', 1, 'impl Rig > fn first')])
        self.assertEqual(code, 1)
        self.assertIn('NEW', err)
        self.assertIn('clippy::collapsible_if', err)
        self.assertIn('fn second', err)

    def test_a_count_that_rose_is_red(self):
        code, _, err = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1', 3),
                                     [self.entry('clippy::ptr_arg', 'let x = 1', 2, 'impl Rig > fn first')])
        self.assertEqual(code, 1)
        self.assertIn('GREW', err)
        self.assertIn('2 -> 3', err)

    def test_a_count_that_fell_is_green_and_reported(self):
        code, out, _ = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1'),
                                     [self.entry('clippy::ptr_arg', 'let x = 1', 3, 'impl Rig > fn first')])
        self.assertEqual(code, 0)
        self.assertIn('shrank', out)
        self.assertIn('--update', out)

    def test_update_lowers_the_list_and_refuses_to_raise_it(self):
        entries = [self.entry('clippy::ptr_arg', 'let x = 1', 3, 'impl Rig > fn first'),
                   self.entry('dead_code', 'unused: u8', 1, 'struct Rig')]
        code, _, _ = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1'), entries, '--update')
        self.assertEqual(code, 0)
        rewritten = json.loads(self.allow.read_text(encoding='utf-8'))['shapes']['macos']['entries']
        self.assertEqual([(e['lint'], e['count'], e['owner']) for e in rewritten], [('clippy::ptr_arg', 1, '6.1')])
        code, _, err = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1', 9), rewritten, '--update')
        self.assertEqual(code, 1)
        self.assertIn('GREW', err)
        self.assertEqual(json.loads(self.allow.read_text(encoding='utf-8'))['shapes']['macos']['entries'], rewritten)

    def test_the_default_shape_selects_the_running_platforms_entries(self):
        lines = self.lines('clippy::ptr_arg', 'let x = 1', 2)
        entries = [self.entry('clippy::ptr_arg', 'let x = 1', 2, 'impl Rig > fn first')]
        for platform, shape in (('darwin', 'macos'), ('linux', 'linux')):
            other = 'linux' if shape == 'macos' else 'macos'
            self.allow.write_text(json.dumps({'shapes': {shape: {'entries': entries}, other: {'entries': []}}}),
                                  encoding='utf-8')
            self.json.write_text('\n'.join(lines), encoding='utf-8')
            with self.subTest(platform=platform), mock.patch.object(gate.sys, 'platform', platform):
                out, err = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                    code = gate.main(['--allowlist', str(self.allow), '--json', str(self.json),
                                      '--source-root', str(self.root)])
                self.assertEqual((code, err.getvalue()), (0, ''))
                self.assertIn(f'(shape {shape})', out.getvalue())

    def test_a_missing_shape_is_refused_and_named(self):
        # run_gate writes only a macos shape.
        with mock.patch.object(gate.sys, 'platform', 'linux'):
            code, out, err = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1'),
                                           [self.entry('clippy::ptr_arg', 'let x = 1', 1, 'impl Rig > fn first')],
                                           platform_default=True)
        self.assertEqual((code, out), (1, ''))
        self.assertIn("no shape 'linux'", err)
        self.assertIn('macos', err)
        code, _, err = self.run_gate(self.lines('clippy::ptr_arg', 'let x = 1'),
                                     [self.entry('clippy::ptr_arg', 'let x = 1', 1, 'impl Rig > fn first')],
                                     '--shape', 'windows')
        self.assertEqual(code, 1)
        self.assertIn("no shape 'windows'", err)

    def test_baseline_prints_entries_with_owner_and_class(self):
        code, out, _ = self.run_gate(self.lines('dead_code', 'unused: u8', 2), [], '--baseline')
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out), [{'lint': 'dead_code', 'file': 'src/rig.rs', 'item': 'struct Rig',
                                            'unit': 'gwz_core:lib', 'count': 2, 'owner': '6.1',
                                            'class': 'dead-code'}])

    def test_an_unknown_shape_is_an_error_not_a_pass(self):
        code, _, err = self.run_gate(self.lines('dead_code', 'unused: u8'), [], '--shape', 'windows')
        self.assertEqual(code, 1)
        self.assertIn("no shape 'windows'", err)

    def test_empty_output_checks_nothing_and_is_not_green(self):
        code, _, err = self.run_gate([], [])
        self.assertEqual(code, 2)
        self.assertIn('nothing was checked', err)

    def test_a_bad_allowlist_is_red(self):
        code, _, err = self.run_gate(self.lines('dead_code', 'unused: u8'),
                                     [{'lint': 'dead_code', 'file': 'src/rig.rs', 'item': '', 'unit': 'u', 'count': 1}])
        self.assertEqual(code, 1)
        self.assertIn('owner', err)


class Running(unittest.TestCase):
    def options(self, *extra):
        return gate.parse_arguments(['--manifest-path', '/cand/Cargo.toml', *extra])

    def test_the_command_lints_every_unit_and_denies_nothing(self):
        command = gate.clippy_command(self.options('--target-dir', '/t'))
        self.assertEqual(command[:3], ['cargo', '+1.95.0', 'clippy'])
        for flag in ('--no-deps', '--all-targets', '--all-features', '--locked', '--keep-going',
                     '--message-format=json'):
            self.assertIn(flag, command)
        self.assertNotIn('-D', command)
        self.assertNotIn('warnings', command)
        self.assertEqual(command[command.index('--target-dir') + 1], '/t')

    def test_the_library_shape_swaps_all_targets_for_lib(self):
        command = gate.clippy_command(self.options('--lib', '--target', 'x86_64-pc-windows-msvc'))
        self.assertIn('--lib', command)
        self.assertNotIn('--all-targets', command)
        self.assertEqual(command[command.index('--target') + 1], 'x86_64-pc-windows-msvc')

    def test_the_environment_carries_the_candidate_cfg_once_and_the_lint_conf(self):
        options = self.options()
        env = gate.clippy_environment(options, {'RUSTFLAGS': '-Dwarnings'})
        self.assertEqual(env['RUSTFLAGS'], '-Dwarnings --cfg gwz_transport_candidate')
        self.assertEqual(gate.clippy_environment(options, env)['RUSTFLAGS'], env['RUSTFLAGS'])
        self.assertEqual(gate.clippy_environment(options, {})['RUSTFLAGS'], '--cfg gwz_transport_candidate')
        self.assertEqual(Path(env['CLIPPY_CONF_DIR']), gate.DEFAULT_CONF_DIR.resolve())

    def test_cargo_failing_without_a_diagnostic_is_not_green(self):
        err = io.StringIO()
        with mock.patch.object(gate, 'run_clippy', return_value=(['{"reason":"build-finished","success":false}'],
                                                                101, 'error: failed to run custom build command')):
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                code = gate.main(['--manifest-path', '/cand/Cargo.toml'])
        self.assertEqual(code, 2)
        self.assertIn('did not complete', err.getvalue())
        self.assertIn('failed to run custom build command', err.getvalue())


if __name__ == '__main__':
    unittest.main()
