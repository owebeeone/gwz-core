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

    def errors(self):
        return checker.check(self.root, self.inventory)[0]


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
                 entry('unix', 'mod c', state='platform', reason='AF_UNIX')]
        self.assertEqual(self.compare(added[:1], added), [])

    def test_a_count_raise_fails(self):
        self.assertTrue(self.compare([entry('unix', 'mod a')], [entry('unix', 'mod a', count=2)])[0].startswith('RAISED'))

    def test_a_narrower_scope_or_a_reopened_step_fails(self):
        self.assertTrue(self.compare([], [], now_roots=ROOTS[:1])[0].startswith('NARROWED'))
        self.assertTrue(self.compare([], [], base_done=['0.1', '0.2'], now_done=['0.1'])[0].startswith('REOPENED'))

    def test_a_missing_base_passes(self):
        with Tree() as tree:
            self.assertEqual(checker.shrink(tree.inventory, tree.root / 'none.json')[0], [])


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
