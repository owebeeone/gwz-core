import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('filesystem_boundary', Path(__file__).with_name('check_filesystem_boundary.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

class BoundaryTests(unittest.TestCase):
    def test_context_scope_rejects_ambient_factories_and_aliases(self):
        for source in ['make_filesystem();', 'crate::git::make_repository();',
                       'use crate::filesystem::make_filesystem as ambient; ambient();',
                       'let factory = crate::git::make_repository;',
                       'OperationContext::existing();']:
            with self.subTest(source=source):
                self.assertTrue(checker.context_violations(source))
        self.assertFalse(checker.context_violations('context.filesystem().read(path);'))
        self.assertFalse(checker.context_violations('// make_repository()\nlet label = "make_filesystem";'))

    def test_imports_and_qualified_calls(self):
        for source in ['use std::fs;', 'use std::{fs as disk, path::Path};',
                       'use cap_std::fs::Dir;', 'std::fs::read(p);',
                       'use std as system; system::fs::read(p);',
                       'use tempfile::tempdir;', 'use libc::open;', 'std::process::Command::new("touch");']:
            with self.subTest(source=source):
                self.assertTrue(checker.violations(source))

    def test_path_observations_and_suppressions(self):
        for source in ['path.canonicalize();', 'path.exists();',
                       '#[allow(clippy::disallowed_methods)] fn f() {}']:
            self.assertTrue(checker.violations(source))

    def test_pure_paths_comments_and_strings(self):
        self.assertFalse(checker.violations('''
use std::{path::Path, io::Cursor};
let p = root.join("std::fs::read(p)");
// use std::fs;
/* path.exists(); /* nested */ */
let message = r###"path.canonicalize();"###;
'''))

    def test_line_numbers(self):
        self.assertEqual(checker.violations('\n\nstd::fs::read(p);')[0][0], 3)

    def test_production_scan_excludes_only_items_that_require_test(self):
        source = '''
#[cfg(test)] mod tests { fn fixture() { std::fs::read(p); } }
#[cfg(all(windows, test))] fn native_fixture() { std::fs::write(p, b"x"); }
#[cfg(not(test))] fn production() { std::fs::read(p); }
#[cfg(any(windows, test))] fn windows_production() { std::fs::read(p); }
'''
        failures = checker.violations(checker.production_source(source))
        self.assertEqual([line for line, _ in failures], [4, 5])


import json
import tempfile


PRODUCERS = ['open_repo', 'repo_head', 'find_remote', 'git_error']


class GitBoundaryTests(unittest.TestCase):
    """Step 5.5: the migrated area of the gwz-git move imports no git2 directly."""

    def uses(self, source):
        return checker.git2_uses(source)

    def test_every_spelling_of_a_direct_use_counts(self):
        for source in ['use git2::Repository;', 'use git2::{Oid, Repository};', 'use git2 as native;',
                       'extern crate git2;', 'let r = git2::Repository::open(p);',
                       'fn f(error: git2::Error) {}', 'pub use git2::Oid;',
                       'x::<git2::Oid>();', 'let r = ::git2::Repository::open(p);', '#[derive(Debug)] struct S { id: git2::Oid }']:
            with self.subTest(source=source):
                self.assertEqual(len(self.uses(source)), 1)

    def test_comments_strings_members_and_test_items_do_not(self):
        for source in ['// use git2::Repository;', '/* git2::Oid */', 'let s = "git2::Oid";',
                       'let s = r#"use git2::Oid;"#;', 'let x = value.git2;', 'let x = other::git2::Oid;',
                       '#[cfg(test)] mod tests { use git2::Repository; }',
                       '#[cfg(test)] fn probe() -> git2::Oid { todo!() }',
                       'let name = "git2"; let gwz_git2 = 1; let git2_like = 2;']:
            with self.subTest(source=source):
                self.assertEqual(self.uses(source), [])

    def test_platform_production_branches_still_count(self):
        self.assertEqual(self.uses('#[cfg(windows)] use git2::Oid;'), [1])
        self.assertEqual(self.uses('#[cfg(not(test))] use git2::Oid;'), [1])

    def test_line_numbers(self):
        self.assertEqual(self.uses('fn a() {}\n\nuse git2::Oid;\nfn b(e: git2::Error) {}'), [3, 4])

    def tree(self, files, config):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        return checker.git2_boundary_failures(root, {'native_producers': PRODUCERS, **config})

    def entry(self, path, count, producers=0, **extra):
        return {'path': path, 'git2': count, 'producers': producers, 'rows': ['G3'], 'responsibility': 'branch functions',
                'exit': 'the G3 step', 'dated': '2026-10-25', **extra}

    def test_a_zero_tolerance_file_with_a_use_fails_and_is_named(self):
        failures, scanned = self.tree({'src/reads.rs': 'use git2::Repository;\n', 'src/ok.rs': 'fn f() {}'},
                                      {'zero': ['src/reads.rs', 'src/gone.rs'], 'exceptions': []})
        self.assertEqual(len(failures), 1)
        self.assertTrue(failures[0].startswith('src/reads.rs:1:'), failures)
        self.assertEqual(scanned, (1, 0))

    def test_a_zero_tolerance_directory_covers_every_production_file_but_not_tests(self):
        files = {'src/old/a.rs': 'fn a() {}', 'src/old/b.rs': 'fn b(e: git2::Error) {}',
                 'src/old/tests/t.rs': 'use git2::Oid;', 'src/old/b_tests.rs': 'use git2::Oid;'}
        failures, scanned = self.tree(files, {'zero': ['src/old'], 'exceptions': []})
        self.assertEqual([f.split(':')[0] for f in failures], ['src/old/b.rs'])
        self.assertEqual(scanned, (2, 0))

    def test_an_exception_holds_its_count_exactly(self):
        config = {'zero': [], 'exceptions': [self.entry('src/refs.rs', 2)]}
        two = 'use git2::Oid;\nfn f(e: git2::Error) {}\n'
        self.assertEqual(self.tree({'src/refs.rs': two}, config)[0], [])
        more = self.tree({'src/refs.rs': two + 'fn g(e: git2::Error) {}\n'}, config)[0]
        self.assertEqual(len(more), 1)
        self.assertIn('3 direct git2 uses, 2 recorded', more[0])
        self.assertIn('lines 1, 2, 3', more[0])
        fewer = self.tree({'src/refs.rs': 'use git2::Oid;\n'}, config)[0]
        self.assertEqual(len(fewer), 1)
        self.assertIn('lower the entry', fewer[0])
        gone = self.tree({'src/refs.rs': 'fn f() {}\n'}, config)[0]
        self.assertTrue(any('delete the entry' in f for f in gone), gone)

    def test_an_exception_for_a_missing_file_or_without_its_reasons_fails(self):
        self.assertIn('file that is gone', self.tree({}, {'zero': [], 'exceptions': [self.entry('src/x.rs', 1)]})[0][0])
        bad = self.entry('src/x.rs', 1)
        del bad['exit']
        self.assertIn('lacks exit', self.tree({'src/x.rs': 'use git2::Oid;'}, {'zero': [], 'exceptions': [bad]})[0][0])
        undated = self.entry('src/x.rs', 1, dated='October')
        self.assertTrue(any('YYYY-MM-DD' in f for f in
                            self.tree({'src/x.rs': 'use git2::Oid;'}, {'zero': [], 'exceptions': [undated]})[0]))
        empty = self.entry('src/x.rs', 1, exit=' ')
        self.assertTrue(any('must say something' in f for f in
                            self.tree({'src/x.rs': 'use git2::Oid;'}, {'zero': [], 'exceptions': [empty]})[0]))

    def test_a_path_is_zero_or_excepted_not_both_and_not_listed_twice(self):
        config = {'zero': ['src'], 'exceptions': [self.entry('src/a.rs', 1)]}
        self.assertTrue(any('zero-tolerance and also' in f for f in self.tree({'src/a.rs': 'use git2::Oid;'}, config)[0]))
        twice = {'zero': [], 'exceptions': [self.entry('src/a.rs', 1), self.entry('src/a.rs', 1)]}
        self.assertTrue(any('listed twice' in f for f in self.tree({'src/a.rs': 'use git2::Oid;'}, twice)[0]))

    def test_a_native_handle_reached_through_a_core_producer_is_a_use_too(self):
        sample = 'use super::*;\nfn f(p: &Path) { let repo = open_repo(p)?; repo.merge_base(a, b); }\n'
        self.assertEqual(checker.git2_uses(sample), [])
        self.assertEqual(checker.producer_uses(sample, set(PRODUCERS)), [2])
        failures, _ = self.tree({'src/reads.rs': sample}, {'zero': ['src/reads.rs'], 'exceptions': []})
        self.assertEqual(len(failures), 1)
        self.assertTrue(failures[0].startswith('src/reads.rs:2:') and 'native-handle producer' in failures[0], failures)

    def test_every_producer_counts_and_a_definition_or_member_does_not(self):
        for name in PRODUCERS:
            with self.subTest(name=name):
                self.assertEqual(checker.producer_uses(f'let x = {name}(p)?;', set(PRODUCERS)), [1])
                self.assertEqual(checker.producer_uses(f'.map_err({name})', set(PRODUCERS)), [1])
                self.assertEqual(checker.producer_uses(f'fn {name}(p: &Path) {{}}', set(PRODUCERS)), [])
                self.assertEqual(checker.producer_uses(f'x.{name}(p);', set(PRODUCERS)), [])
        self.assertEqual(checker.producer_uses('// open_repo(p)\nlet s = "open_repo"; #[cfg(test)] fn t() { open_repo(p); }',
                                               set(PRODUCERS)), [])

    def test_an_exception_holds_its_producer_count_exactly(self):
        config = {'zero': [], 'exceptions': [self.entry('src/cmp.rs', 0, 2)]}
        two = 'fn f(p: &Path) { open_repo(p)?; open_repo(p)?; }\n'
        self.assertEqual(self.tree({'src/cmp.rs': two}, config)[0], [])
        more = self.tree({'src/cmp.rs': two + 'fn g(p: &Path) { repo_head(p); }\n'}, config)[0]
        self.assertEqual(len(more), 1)
        self.assertIn('3 native-handle producer uses, 2 recorded', more[0])
        fewer = self.tree({'src/cmp.rs': 'fn f(p: &Path) { open_repo(p)?; }\n'}, config)[0]
        self.assertIn('lower the entry', fewer[0])
        gone = self.tree({'src/cmp.rs': 'fn f() {}\n'}, config)[0]
        self.assertTrue(any('delete the entry' in f for f in gone), gone)

    def test_comparison_is_an_exception_with_the_g8_row_not_a_zero_file(self):
        config = json.loads((checker.ROOT / checker.GIT_BOUNDARY).read_text(encoding='utf-8'))
        self.assertNotIn('src/git/gitbackend/comparison.rs', config['zero'])
        (entry,) = [e for e in config['exceptions'] if e['path'] == 'src/git/gitbackend/comparison.rs']
        self.assertEqual(entry['rows'], ['G8'])
        self.assertGreater(entry['producers'], 0)

    def test_the_real_tree_is_green_under_the_real_config(self):
        config = json.loads((checker.ROOT / checker.GIT_BOUNDARY).read_text(encoding='utf-8'))
        self.assertEqual(checker.git2_boundary_failures(checker.ROOT, config)[0], [])
        for entry in config['exceptions']:
            self.assertEqual(sorted(entry), sorted(checker.GIT_BOUNDARY_FIELDS))


if __name__ == '__main__':
    unittest.main()
