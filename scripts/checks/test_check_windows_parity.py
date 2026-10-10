"""Tests for the Windows-parity inventory and its ratchet (GwzTransportWindowsParityPlan.md, step 0.3)."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('windows_parity', Path(__file__).with_name('check_windows_parity.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

STEPS = ['0.1', '0.2', '0.5', '1.2', '3.4']
ROOTS = ['src/git/endpoint', 'src/transport_host']
FILE = 'src/git/endpoint/x.rs'
RUN_TESTS = checker.ROOT / 'scripts' / 'run_tests.py'
LANE_GATE = checker.ROOT / 'scripts' / 'checks' / 'check_lane_commits.sh'


def entry(gate, target, kind='gate', state='unported', owner=('1.2',), path=FILE, **more):
    return {'path': path, 'kind': kind, 'gate': gate, 'target': target, 'state': state, 'owner': list(owner), **more}


def found(source):
    return [(o.kind, o.gate, o.target, o.side, o.windows_arm) for o in checker.analyze(source)]


class Tree(tempfile.TemporaryDirectory):
    """A scratch gwz-core with one source file and an inventory."""

    def __init__(self, source='', entries=(), done=('0.1',), roots=ROOTS):
        super().__init__()
        self.root = Path(self.name)
        self.write(FILE, source)
        self.write('src/transport_host/mod.rs', '')
        self.inventory = self.root / 'inventory.json'
        self.save(entries, done, roots)

    def __enter__(self):
        return self

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')

    def save(self, entries, done=('0.1',), roots=ROOTS, path=None):
        (path or self.inventory).write_text(json.dumps(
            {'roots': list(roots), 'steps': STEPS, 'done_steps': list(done), 'entries': list(entries)}), encoding='utf-8')

    def save_dir(self, files, done=('0.1',), roots=ROOTS, scans=None, path=None):
        """An inventory directory: meta.json and one `<step>.json` per entry of `files` (a step: its entries or its file's content)."""
        path = path or self.root / 'inventory'
        path.mkdir(parents=True, exist_ok=True)
        meta = {'roots': list(roots), 'steps': STEPS, 'done_steps': list(done)}
        if scans is not None:
            meta['scans'] = list(scans)
        (path / 'meta.json').write_text(json.dumps(meta), encoding='utf-8')
        for step, content in files.items():
            body = content if isinstance(content, dict) else {'entries': list(content)}
            (path / f'{step}.json').write_text(json.dumps(body), encoding='utf-8')
        return path

    def errors(self, inventory=None):
        return checker.check(self.root, inventory or self.inventory)[0]


UNIX_IMPORT = 'cfg_if::cfg_if! { if #[cfg(unix)] { use std::os::unix::ffi::OsStrExt; } }\n'
UNIX_IMPORT_ENTRIES = [entry('unix', 'arm: use std::os::unix::ffi::OsStrExt'),
                       entry('os::unix::ffi::OsStrExt', '', kind='os')]


class Predicates(unittest.TestCase):
    def gates(self, predicate):
        return [g for _, g, *_ in found(f'cfg_if::cfg_if! {{ if #[cfg({predicate})] {{ mod m; }} }}')]

    def test_unix_only_predicates_are_gates(self):
        for predicate in ('unix', 'all(test, unix)', 'not(windows)', 'target_os = "linux"', 'target_family = "unix"',
                          'not(any(windows, target_vendor = "apple"))', 'all(unix, gwz_transport_candidate)',
                          'any(all(test, unix), not(any(windows, target_vendor = "apple")))'):
            self.assertEqual(self.gates(predicate), [checker.cfg.render(checker.cfg.lex(predicate))], predicate)

    def test_windows_capable_predicates_are_not_gates(self):
        for predicate in ('windows', 'not(unix)', 'test', 'all(windows, gwz_transport_candidate)', 'target_os = "windows"',
                          'any(unix, windows)', 'not(target_os = "macos")', 'feature = "x"', 'gwz_transport_candidate'):
            self.assertEqual(self.gates(predicate), [], predicate)

    def test_an_else_arm_is_a_gate_only_when_the_arms_before_it_cover_windows(self):
        self.assertEqual(found('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } else { mod b; } }'),
                         [('gate', 'unix', 'mod a', '', True)])
        self.assertEqual(found('cfg_if::cfg_if! { if #[cfg(windows)] { mod a; } else { mod b; } }'),
                         [('gate', 'else', 'mod b', '', True)])
        self.assertEqual(found('cfg_if::cfg_if! { if #[cfg(windows)] { mod a; } else if #[cfg(unix)] { mod b; } }'),
                         [('gate', 'else if unix', 'mod b', '', True)])

    def test_a_module_list_names_each_module_and_other_items_share_one_target(self):
        self.assertEqual(
            found('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; #[path = "b.rs"] mod b; fn f() {} fn g() {} } }'),
            [('gate', 'unix', 'mod a', '', False), ('gate', 'unix', 'mod b', '', False),
             ('gate', 'unix', 'arm: fn f', '', False)])

    def test_attribute_and_inner_attribute_gates(self):
        self.assertEqual(found('#[cfg(unix)]\nfn f() {}\n'), [('gate', 'unix', 'fn f', '', False)])
        self.assertEqual(found('#[cfg(unix)]\nuse a::b;\n#[cfg(windows)]\nuse c::d;\n'),
                         [('gate', 'unix', 'use a::b', '', True)])
        self.assertEqual(found('#![cfg(unix)]\nfn f() {}\n'), [('gate', 'unix', 'file', '', False)])
        self.assertEqual(found('#[cfg(unix)]\nfn f() {}\n#[cfg(not(unix))]\nfn f() {}\n'),
                         [('gate', 'unix', 'fn f', '', True)])

    def test_keys_ignore_layout_and_line_numbers(self):
        compact = found('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }')
        spread = found('\n\n// comment\ncfg_if::cfg_if! {\n    if #[cfg(unix)] {\n        mod a;\n    }\n}\n')
        self.assertEqual(compact, spread)


class OsUses(unittest.TestCase):
    def test_libc_and_unix_paths_are_found_once_each(self):
        self.assertEqual(
            sorted(g for k, g, *_ in found('use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};\n'
                                           'fn f() { unsafe { libc::poll(a, 1, 0) }; let _ = libc::POLLIN; }\n'
                                           'use std::os::fd::AsRawFd;\n') if k == 'os'),
            ['libc::POLLIN', 'libc::poll', 'os::fd::AsRawFd', 'os::unix::fs::{OpenOptionsExt, PermissionsExt}'])

    def test_windows_paths_and_other_libc_named_things_are_not_found(self):
        self.assertEqual(found('use std::os::windows::ffi::OsStrExt;\nuse crate::libc::x;\nfn f() { let libc = 1; }\n'), [])

    def test_a_use_in_a_windows_arm_is_on_the_windows_side(self):
        source = ('cfg_if::cfg_if! { if #[cfg(unix)] { fn f() { libc::kill(1, 9); } } else { fn f() { libc::malloc(8); } } }\n'
                  '#[cfg(windows)]\nfn g() { libc::free(p); }\n')
        self.assertEqual([(g, side) for k, g, _, side, _ in found(source) if k == 'os'],
                         [('libc::kill', ''), ('libc::malloc', 'windows'), ('libc::free', 'windows')])


class Ratchet(unittest.TestCase):
    def test_a_listed_gate_passes(self):
        with Tree(UNIX_IMPORT, UNIX_IMPORT_ENTRIES) as tree:
            self.assertEqual(tree.errors(), [])

    def test_a_new_unix_import_with_no_owner_fails(self):
        with Tree(UNIX_IMPORT, UNIX_IMPORT_ENTRIES) as tree:
            tree.write(FILE, UNIX_IMPORT + 'cfg_if::cfg_if! { if #[cfg(unix)] { use std::os::unix::fs::symlink; } }\n')
            errors = tree.errors()
        self.assertEqual(len(errors), 2, errors)
        self.assertTrue(all(error.startswith('NEW') for error in errors), errors)

    def test_a_cfg_all_test_unix_module_missing_from_the_inventory_fails(self):
        with Tree('cfg_if::cfg_if! { if #[cfg(all(test, unix))] { mod a_tests; } }\n') as tree:
            errors = tree.errors()
        self.assertEqual(len(errors), 1)
        self.assertIn('NEW   gate all(test, unix) mod a_tests', errors[0])

    def test_a_removed_gate_with_a_stale_entry_fails(self):
        with Tree('fn f() {}\n', UNIX_IMPORT_ENTRIES) as tree:
            errors = tree.errors()
        self.assertEqual([error.split()[0] for error in errors], ['STALE', 'STALE'])

    def test_more_gates_than_the_count_lists_fail(self):
        with Tree(UNIX_IMPORT * 2, UNIX_IMPORT_ENTRIES) as tree:
            self.assertEqual({error.split()[0] for error in tree.errors()}, {'COUNT'})
            tree.save([dict(e, count=2) for e in UNIX_IMPORT_ENTRIES])
            self.assertEqual(tree.errors(), [])
            tree.write(FILE, UNIX_IMPORT)
            self.assertEqual({error.split()[0] for error in tree.errors()}, {'STALE'})

    def test_a_libc_use_in_a_windows_arm_fails_and_passes_with_a_proof(self):
        source = 'cfg_if::cfg_if! { if #[cfg(windows)] { fn f() { libc::malloc(8); } } }\n'
        listed = entry('libc::malloc', '', kind='os', side='windows', owner=['3.4'])
        with Tree(source, [listed]) as tree:
            errors = tree.errors()
            self.assertEqual([error.split()[0] for error in errors], ['PROOF'])
            tree.save([dict(listed, proof='2026-10-09-crt-heap-proof')])
            self.assertEqual(tree.errors(), [])

    def test_a_windows_use_with_no_entry_fails_as_new(self):
        with Tree('#[cfg(windows)]\nfn f() { unsafe { libc::malloc(8) }; }\n') as tree:
            self.assertEqual([error.split()[0] for error in tree.errors()], ['NEW'])

    def test_a_paired_entry_that_loses_its_windows_arm_fails(self):
        paired = [entry('unix', 'mod a', state='paired')]
        with Tree('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } else { mod a_windows; } }\n', paired) as tree:
            self.assertEqual(tree.errors(), [])
            tree.write(FILE, 'cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }\n')
            self.assertEqual([error.split()[0] for error in tree.errors()], ['PAIRED'])

    def test_an_os_call_in_a_unix_arm_with_a_windows_arm_beside_it_can_be_paired(self):
        both = ('cfg_if::cfg_if! { if #[cfg(unix)] { fn k() { libc::kill(1, 9); } } else { fn k() {} } }\n')
        listed = entry('libc::kill', '', kind='os', state='paired', owner=['0.5'])
        with Tree(both, [entry('unix', 'arm: fn k', state='paired', owner=['0.5']), listed]) as tree:
            self.assertEqual(tree.errors(), [])
            tree.write(FILE, both.replace(' else { fn k() {} }', ''))
            self.assertEqual(sorted(error.split()[0] for error in tree.errors()), ['PAIRED', 'PAIRED'])

    def test_a_stub_arm_does_not_make_an_unported_entry_paired(self):
        with Tree('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } else { mod stub; } }\n', [entry('unix', 'mod a')]) as tree:
            self.assertEqual(tree.errors(), [])

    def test_an_unported_entry_whose_owner_is_done_fails(self):
        with Tree('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }\n',
                  [entry('unix', 'mod a', owner=['1.2', '3.4'])], done=['0.1', '1.2']) as tree:
            self.assertEqual(tree.errors(), [])
            tree.save([entry('unix', 'mod a', owner=['1.2', '3.4'])], done=['0.1', '1.2', '3.4'])
            self.assertEqual([error.split()[0] for error in tree.errors()], ['DONE'])
            tree.save([entry('unix', 'mod a', owner=['1.2'], state='platform', reason='AF_UNIX')],
                      done=['0.1', '1.2', '3.4'])
            self.assertEqual(tree.errors(), [])

    def test_inventory_errors(self):
        source = 'cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }\n'
        cases = {
            'unknown owner': ([entry('unix', 'mod a', owner=['9.9'])], 'owner must be a list of steps'),
            'no owner': ([entry('unix', 'mod a', owner=[])], 'owner must be a list of steps'),
            'platform without a reason': ([entry('unix', 'mod a', state='platform')], 'needs a reason'),
            'unknown state': ([entry('unix', 'mod a', state='done')], 'state must be one of'),
            'duplicate': ([entry('unix', 'mod a')] * 2, 'duplicate inventory entry'),
            'bad count': ([entry('unix', 'mod a', count=0)], 'count must be a positive integer'),
        }
        for name, (entries, message) in cases.items():
            with self.subTest(name), Tree(source, entries) as tree:
                self.assertTrue(any(message in error for error in tree.errors()), tree.errors())

    def test_a_missing_scope_root_fails(self):
        with Tree('', [], roots=ROOTS + ['src/gone']) as tree:
            self.assertTrue(any(error.startswith('MISSING') for error in tree.errors()))

    def test_a_prefix_root_scans_the_matching_files_and_directories(self):
        with Tree('', [], roots=['src/git/gitbackend/transport_*']) as tree:
            tree.write('src/git/gitbackend/transport_a.rs', UNIX_IMPORT)
            tree.write('src/git/gitbackend/transport_dir/b.rs', UNIX_IMPORT)
            tree.write('src/git/gitbackend/other.rs', UNIX_IMPORT)
            self.assertEqual(sorted({error.split(' at ')[-1].split(':')[0] for error in tree.errors()}),
                             ['src/git/gitbackend/transport_a.rs', 'src/git/gitbackend/transport_dir/b.rs'])

    def test_files_outside_the_scope_are_not_scanned(self):
        with Tree('', []) as tree:
            tree.write('src/other/y.rs', UNIX_IMPORT)
            self.assertEqual(tree.errors(), [])


class Shrink(unittest.TestCase):
    def compare(self, base_entries, now_entries, base_done=('0.1',), now_done=('0.1',), base_roots=ROOTS, now_roots=ROOTS):
        with Tree() as tree:
            base = tree.root / 'base.json'
            tree.save(base_entries, base_done, base_roots, base)
            tree.save(now_entries, now_done, now_roots)
            return checker.shrink(tree.inventory, base)[0]

    def test_a_smaller_or_equal_unported_count_passes(self):
        two = [entry('unix', 'mod a'), entry('unix', 'mod b')]
        self.assertEqual(self.compare(two, two[:1]), [])
        self.assertEqual(self.compare(two, two), [])

    def test_a_new_unported_entry_fails_unless_another_goes(self):
        two = [entry('unix', 'mod a'), entry('unix', 'mod b')]
        self.assertTrue(self.compare(two[:1], two)[0].startswith('RAISED'))
        self.assertEqual(self.compare(two[:1], [two[1]]), [])

    def test_a_new_paired_or_platform_entry_does_not_raise_the_count(self):
        added = [entry('unix', 'mod a'), entry('unix', 'mod b', state='paired'),
                 entry('unix', 'mod c', state='platform', reason='AF_UNIX', approved_platform='operator 2026-10-10')]
        self.assertEqual(self.compare(added[:1], added), [])

    def test_a_count_raise_fails(self):
        self.assertTrue(self.compare([entry('unix', 'mod a')], [entry('unix', 'mod a', count=2)])[0].startswith('RAISED'))

    def test_a_narrower_scope_or_a_reopened_step_fails(self):
        self.assertTrue(self.compare([], [], now_roots=ROOTS[:1])[0].startswith('NARROWED'))
        self.assertTrue(self.compare([], [], base_done=['0.1', '0.2'], now_done=['0.1'])[0].startswith('REOPENED'))

    def test_a_missing_base_passes(self):
        with Tree() as tree:
            self.assertEqual(checker.shrink(tree.inventory, tree.root / 'none.json')[0], [])


SPLIT_SOURCE = 'fn f() -> bool { cfg!(windows) }\n'


class Directory(unittest.TestCase):
    """The inventory as a directory: meta.json plus one file per owning step, loaded as one inventory."""

    def test_a_directory_is_one_inventory(self):
        two = UNIX_IMPORT + 'cfg_if::cfg_if! { if #[cfg(unix)] { mod b; } }\n'
        listed = [entry('unix', 'arm: use std::os::unix::ffi::OsStrExt', owner=['1.2']),
                  entry('os::unix::ffi::OsStrExt', '', kind='os', owner=['1.2'])]
        with Tree(two, listed) as tree:
            directory = tree.save_dir({'1.2': listed, '3.4': [entry('unix', 'mod b', owner=['3.4'])]})
            self.assertEqual(tree.errors(directory), [])
            data, errors = checker.load_inventory(directory)
            self.assertEqual((errors, len(data['entries']), checker.unported(data)), ([], 3, 3))
            tree.save_dir({'1.2': listed}, path=directory)
            (directory / '3.4.json').unlink()
            self.assertEqual([error.split()[0] for error in tree.errors(directory)], ['NEW'])

    def test_an_entry_lives_in_the_file_of_its_first_owner(self):
        with Tree('cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }\n') as tree:
            directory = tree.save_dir({'3.4': [entry('unix', 'mod a', owner=['1.2', '3.4'])]})
            errors = checker.load_inventory(directory)[1]
            self.assertTrue(any('FILED' in e and '1.2' in e and '3.4.json' in e for e in errors), errors)
            tree.save_dir({'1.2': [entry('unix', 'mod a', owner=['1.2', '3.4'])]}, path=directory)
            (directory / '3.4.json').unlink()
            self.assertEqual(checker.load_inventory(directory)[1], [])

    def test_a_file_named_for_an_unknown_step_or_with_no_meta_is_an_error(self):
        with Tree() as tree:
            directory = tree.save_dir({'9.9': []})
            self.assertTrue(any('9.9.json' in e and 'not a step' in e for e in checker.load_inventory(directory)[1]))
            (directory / 'meta.json').unlink()
            self.assertTrue(any('meta.json' in e for e in checker.load_inventory(directory)[1]))

    def test_the_same_key_in_two_files_is_a_duplicate(self):
        with Tree() as tree:
            directory = tree.save_dir({'1.2': [entry('unix', 'mod a')], '3.4': [entry('unix', 'mod a', owner=['3.4'])]})
            self.assertTrue(any('duplicate' in e for e in checker.load_inventory(directory)[1]))

    def test_a_step_file_can_record_its_step_done(self):
        source = 'cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }\n'
        with Tree(source) as tree:
            directory = tree.save_dir({'1.2': [entry('unix', 'mod a')]})
            self.assertEqual(tree.errors(directory), [])
            tree.save_dir({'1.2': {'done': True, 'entries': [entry('unix', 'mod a')]}}, path=directory)
            self.assertEqual([error.split()[0] for error in tree.errors(directory)], ['DONE'])
            data, _ = checker.load_inventory(directory)
            self.assertEqual(data['done_steps'], ['0.1', '1.2'])

    def test_shrink_compares_a_directory_with_a_single_file_in_either_direction(self):
        two, one = [entry('unix', 'mod a'), entry('unix', 'mod b')], [entry('unix', 'mod a')]
        with Tree() as tree:
            directory = tree.save_dir({'1.2': two})
            tree.save(one)
            self.assertTrue(checker.shrink(directory, tree.inventory)[0][0].startswith('RAISED'))
            self.assertEqual(checker.shrink(tree.inventory, directory)[0], [])
            tree.save(two)
            self.assertEqual(checker.shrink(directory, tree.inventory)[0], [])

    def test_the_command_takes_a_directory_for_both_the_inventory_and_the_base(self):
        source = 'cfg_if::cfg_if! { if #[cfg(unix)] { mod a; } }\n'
        with Tree(source) as tree:
            directory = tree.save_dir({'1.2': [entry('unix', 'mod a')]})
            base = tree.save_dir({'1.2': [entry('unix', 'mod a')]}, path=tree.root / 'base')
            run = Command().run_main
            self.assertEqual(run('--root', str(tree.root), '--inventory', str(directory))[0], 0)
            self.assertEqual(run('--inventory', str(directory), '--shrink-from', str(base))[0], 0)


class RuntimeSplits(unittest.TestCase):
    """cfg!(), cfg_attr and the OS extension modules by name are seen, so a runtime split is not invisible."""

    def test_cfg_macros_and_cfg_attr_with_a_platform_predicate_are_splits(self):
        self.assertEqual(found('fn f() -> bool { cfg!(windows) }\nfn g() -> bool { !cfg!(unix) }\n'),
                         [('split', 'cfg!(windows)', '', '', True), ('split', 'cfg!(unix)', '', '', True)])
        self.assertEqual(found('fn f() -> bool { cfg!(all(windows, test)) || cfg!(target_os = "macos") }\n'),
                         [('split', 'cfg!(all(windows, test))', '', '', True),
                          ('split', 'cfg!(target_os = "macos")', '', '', True)])
        self.assertEqual(found('#[cfg_attr(unix, allow(unused))]\nfn f() {}\n'),
                         [('split', 'cfg_attr(unix)', 'fn f', '', True)])
        self.assertEqual(found('#![cfg_attr(not(unix), allow(dead_code))]\nfn f() {}\n'),
                         [('split', 'cfg_attr(not(unix))', 'file', '', True)])

    def test_cfg_macros_without_a_platform_predicate_or_with_the_qualification_switch_are_not_splits(self):
        for source in ('fn f() -> bool { cfg!(test) }\n', 'fn f() -> bool { cfg!(feature = "x") }\n',
                       'fn f() -> bool { cfg!(debug_assertions) }\n',
                       'fn f() -> bool { cfg!(all(windows, gwz_transport_candidate, gwz_windows_https_qualification)) }\n',
                       '#[cfg_attr(test, derive(Debug))]\nstruct S;\n'):
            self.assertEqual(found(source), [], source)

    def test_the_named_os_extension_modules_are_os_uses(self):
        self.assertEqual(sorted(g for k, g, *_ in found('use std::os::linux::fs::MetadataExt;\nuse std::os::macos::fs::MetadataExt as M;\n')),
                         ['os::linux::fs::MetadataExt', 'os::macos::fs::MetadataExt'])

    def test_a_split_without_an_entry_fails_and_one_with_an_entry_passes(self):
        with Tree(SPLIT_SOURCE) as tree:
            self.assertTrue(tree.errors()[0].startswith('NEW   split cfg!(windows)'), tree.errors())
            tree.save([entry('cfg!(windows)', '', kind='split', state='paired')])
            self.assertEqual(tree.errors(), [])
            tree.write(FILE, 'fn f() {}\n')
            self.assertEqual([error.split()[0] for error in tree.errors()], ['STALE'])

    def test_an_unported_split_counts_in_the_ratchet(self):
        with Tree(SPLIT_SOURCE, [entry('cfg!(windows)', '', kind='split', state='unported')]) as tree:
            self.assertEqual(tree.errors(), [])
            data, _ = checker.load_inventory(tree.inventory)
            self.assertEqual(checker.unported(data), 1)


class ScannedKinds(unittest.TestCase):
    """A kind the base inventory does not scan yet is compared only once the base scans it."""

    def test_a_single_file_scans_gate_and_os(self):
        with Tree() as tree:
            self.assertEqual(checker.load_inventory(tree.inventory)[0]['scans'], ['gate', 'os'])

    def test_a_directory_must_list_the_kinds_the_checker_scans(self):
        with Tree() as tree:
            directory = tree.save_dir({}, scans=['gate', 'os'])
            self.assertTrue(any('scans' in e for e in checker.load_inventory(directory)[1]))
            directory = tree.save_dir({}, scans=list(checker.SCANS))
            self.assertEqual(checker.load_inventory(directory)[1], [])

    def test_a_kind_the_base_does_not_scan_is_left_out_of_the_comparison(self):
        old_base = [entry('unix', 'mod a')]
        now = [entry('unix', 'mod a'), entry('cfg!(windows)', '', kind='split', state='unported'),
               entry('cfg!(unix)', '', kind='split', state='unported')]
        with Tree() as tree:
            tree.save(old_base)
            directory = tree.save_dir({'1.2': now}, scans=list(checker.SCANS))
            errors, summary = checker.shrink(directory, tree.inventory)
            self.assertEqual(errors, [])
            self.assertIn('split', summary)
            base_with_splits = tree.save_dir({'1.2': old_base}, scans=list(checker.SCANS), path=tree.root / 'base')
            self.assertTrue(checker.shrink(directory, base_with_splits)[0][0].startswith('RAISED'))

    def test_an_old_kind_still_ratchets_against_an_old_base(self):
        with Tree() as tree:
            tree.save([entry('unix', 'mod a')])
            directory = tree.save_dir({'1.2': [entry('unix', 'mod a'), entry('unix', 'mod b')]}, scans=list(checker.SCANS))
            self.assertTrue(checker.shrink(directory, tree.inventory)[0][0].startswith('RAISED'))


class NewPlatformRows(unittest.TestCase):
    """P3-2: a row that is newly `platform` against the base is listed, and needs a reason and a named approval."""

    def platform(self, **more):
        return entry('unix', 'mod a', state='platform', reason='AF_UNIX only', **more)

    def compare(self, base, now):
        with Tree() as tree:
            tree.save(base, path=tree.root / 'base.json')
            tree.save(now)
            return checker.shrink(tree.inventory, tree.root / 'base.json')

    def test_relabelling_unported_as_platform_without_an_approval_fails_and_is_listed(self):
        errors, summary = self.compare([entry('unix', 'mod a')], [self.platform()])
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('PLATFORM'), errors)
        self.assertIn('approved_platform', errors[0])
        self.assertIn('mod a', errors[0])
        self.assertIn('AF_UNIX only', errors[0])

    def test_a_named_approval_passes_and_the_row_is_still_listed(self):
        errors, summary = self.compare([entry('unix', 'mod a')], [self.platform(approved_platform='operator 2026-10-10')])
        self.assertEqual(errors, [])
        self.assertIn('newly platform', summary)
        self.assertIn('mod a', summary)
        self.assertIn('operator 2026-10-10', summary)

    def test_approved_platform_must_name_the_decision(self):
        for bad in (True, '', '  ', None, 3):
            errors, _ = self.compare([entry('unix', 'mod a')], [self.platform(approved_platform=bad)])
            self.assertEqual([e.split()[0] for e in errors], ['PLATFORM'], bad)

    def test_a_new_row_that_is_platform_from_the_start_is_listed_too(self):
        errors, _ = self.compare([], [self.platform()])
        self.assertEqual([e.split()[0] for e in errors], ['PLATFORM'])

    def test_a_platform_row_the_base_already_had_is_not_listed(self):
        errors, summary = self.compare([self.platform()], [self.platform()])
        self.assertEqual(errors, [])
        self.assertNotIn('newly platform', summary)

    def test_a_raised_platform_count_is_listed(self):
        errors, _ = self.compare([self.platform()], [dict(self.platform(), count=2)])
        self.assertEqual([e.split()[0] for e in errors], ['PLATFORM'])

    def test_a_platform_row_is_not_a_count_raise_once_approved(self):
        errors, _ = self.compare([entry('unix', 'mod a')], [self.platform(approved_platform='OD14')])
        self.assertEqual(errors, [])


class Command(unittest.TestCase):
    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = checker.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_exit_codes_and_messages(self):
        with Tree(UNIX_IMPORT, UNIX_IMPORT_ENTRIES) as tree:
            code, out, _ = self.run_main('--root', str(tree.root), '--inventory', str(tree.inventory))
            self.assertEqual((code, 'nothing new' in out), (0, True))
            tree.write(FILE, UNIX_IMPORT + 'cfg_if::cfg_if! { if #[cfg(unix)] { mod b; } }\n')
            code, _, err = self.run_main('--root', str(tree.root), '--inventory', str(tree.inventory))
            self.assertEqual((code, 'NEW' in err, 'owner step' in err), (1, True, True))

    def test_list_prints_inventory_lines(self):
        with Tree(UNIX_IMPORT, UNIX_IMPORT_ENTRIES) as tree:
            code, out, _ = self.run_main('--root', str(tree.root), '--inventory', str(tree.inventory), '--list')
        rows = [json.loads(line) for line in out.splitlines()]
        self.assertEqual((code, [(r['kind'], r['gate']) for r in rows]),
                         (0, [('gate', 'unix'), ('os', 'os::unix::ffi::OsStrExt')]))


class Seeded(unittest.TestCase):
    """The committed inventory against the committed tree."""

    def test_the_seeded_inventory_passes_at_this_tree(self):
        errors, files, entries = checker.check(checker.ROOT, checker.DEFAULT_INVENTORY)
        self.assertEqual(errors, [])
        self.assertGreater(files, 100)

    def test_every_entry_has_an_owner_step_and_the_plan_names_every_step(self):
        data, errors = checker.load_inventory(checker.DEFAULT_INVENTORY)
        self.assertEqual(errors, [])
        plan = (checker.ROOT / 'dev-docs' / 'GwzTransportWindowsParityPlan.md').read_text(encoding='utf-8')
        for step in data['steps']:
            if f'### Step {step}' not in plan.replace('#### Step', '### Step'):
                # A step the lane added after the plan's revision carries its reason in the inventory.
                self.assertTrue(data.get('step_notes', {}).get(step), f'step {step} is neither in the plan nor noted')
        self.assertTrue(all(e['owner'] and e.get('appendix') for e in data['entries']))

    def test_the_inventory_is_wired_into_the_runner_and_the_lane_gate(self):
        self.assertIn('check_windows_parity.py', RUN_TESTS.read_text(encoding='utf-8'))
        lane_gate = LANE_GATE.read_text(encoding='utf-8')
        self.assertIn('check_windows_parity.py', lane_gate)
        self.assertIn('--shrink-from', lane_gate)


if __name__ == '__main__':
    unittest.main()
