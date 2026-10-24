"""Fixtures for check_gwz_git_adoption.py (step 5.5): each way a gwz-git capability can fail to be adopted."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('adoption', Path(__file__).with_name('check_gwz_git_adoption.py'))
adoption = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adoption)

GIT = {
    'src/lib.rs': 'mod repo;\npub mod ops;\npub use repo::{Repository, Flags};\n',
    'src/repo.rs': '''
pub struct Repository;
pub struct Flags;
impl Repository {
    pub fn open(path: &str) -> Self { Self }
    pub fn head(&self) -> u8 { 0 }
    pub fn status<'a, T>(&self, options: &'a T) -> u8 { 0 }
    pub(crate) fn hidden(&self) {}
    fn private(&self) {}
}
impl<'a> Flags {
    pub const fn make() -> Self { Self }
}
impl std::fmt::Display for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}
struct Internal;
impl Internal { pub fn not_exported(&self) {} }
''',
    'src/ops.rs': '''
pub trait Gate { fn run(&self); }
pub fn commit() {}
pub(crate) fn internal() {}
#[cfg(test)]
mod tests { pub fn probe() {} }
''',
    'src/tests.rs': 'impl super::repo::Repository { pub fn test_only(&self) {} }',
}
CORE = {
    'src/reads.rs': '''
use gwz_git::{Flags, Repository};
fn read(path: &str) { let repo = Repository::open(path); repo.head(); let _ = Flags::make(); repo.status(&1); }
''',
    'src/ops_user.rs': 'use gwz_git::ops; fn go() { ops::commit(); }\nstruct S;\nimpl gwz_git::ops::Gate for S { fn run(&self) {} }\n',
}
GUIDE = '''# guide
Repository, Flags and `ops` are described here. Gate is the trait.
```rust,ignore
impl Repository {
    pub fn open(path: &str) -> Self;
    pub fn head(&self) -> u8;
    pub fn status(&self, options: &T) -> u8;
}
pub mod ops { pub fn commit(); }
```
`Flags::make()` builds flags.
'''
NONE = {'unconsumed': []}


def entry(capability, **extra):
    return {'capability': capability, 'rows': ['G9'], 'reason': 'not consumed yet', 'exit': 'a later row',
            'dated': '2026-10-25', **extra}


class Adoption(unittest.TestCase):
    def world(self, git=None, core=None, guide=GUIDE, config=NONE):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        base = Path(directory.name)
        for repo, files in (('gwz-git', {**GIT, **(git or {})}), ('gwz-core', {**CORE, **(core or {})})):
            for name, text in files.items():
                if text is None:
                    continue
                (base / repo / name).parent.mkdir(parents=True, exist_ok=True)
                (base / repo / name).write_text(text)
        (base / 'gwz-core' / 'dev-docs').mkdir(parents=True, exist_ok=True)
        (base / 'gwz-core' / adoption.GUIDE).write_text(guide)
        return adoption.check(base / 'gwz-core', base / 'gwz-git', config)

    def test_the_surface_is_the_exported_callables_and_nothing_else(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        for name, text in GIT.items():
            (Path(directory.name) / name).parent.mkdir(parents=True, exist_ok=True)
            (Path(directory.name) / name).write_text(text)
        capabilities, exported, modules = adoption.surface(Path(directory.name))
        self.assertEqual(sorted(c.key for c in capabilities),
                         ['Flags::make', 'Gate', 'Repository::head', 'Repository::open', 'Repository::status',
                          'ops::commit'])
        self.assertEqual((exported, modules), ({'Repository', 'Flags'}, {'ops'}))

    def test_everything_consumed_and_documented_is_green(self):
        failures, stats = self.world()
        self.assertEqual(failures, [])
        self.assertEqual(stats, (6, 6, 0))

    def test_a_new_public_function_is_named_unconsumed_and_undocumented(self):
        git = {'src/repo.rs': GIT['src/repo.rs'].replace(
            '    fn private(&self) {}', '    pub fn adoption_probe(&self) -> bool { true }\n    fn private(&self) {}')}
        failures, _ = self.world(git=git)
        self.assertEqual(len(failures), 2, failures)
        self.assertTrue(any(f.startswith('Repository::adoption_probe has no production caller') for f in failures))
        self.assertTrue(any(f.startswith('Repository::adoption_probe is public in gwz-git but') for f in failures))

    def test_a_caller_only_in_test_code_is_not_a_caller(self):
        for name, text in {
            'src/reads.rs': CORE['src/reads.rs'].replace('repo.head();', '').replace(
                'fn read', '#[cfg(test)] mod t { fn x(repo: gwz_git::Repository) { repo.head(); } }\nfn read'),
            'src/reads_tests.rs': 'use gwz_git::Repository; fn t(r: Repository) { r.head(); }',
            'src/tests/reads.rs': 'use gwz_git::Repository; fn t(r: Repository) { r.head(); }',
        }.items():
            with self.subTest(file=name):
                core = {'src/reads.rs': CORE['src/reads.rs'].replace('repo.head();', '')}
                core[name] = text
                failures, _ = self.world(core=core)
                self.assertTrue(any(f.startswith('Repository::head has no production caller') for f in failures), failures)

    def test_a_call_in_a_file_that_does_not_name_the_owner_does_not_count(self):
        core = {'src/reads.rs': CORE['src/reads.rs'].replace('repo.head();', ''),
                'src/other.rs': 'use gwz_git as g; fn f(x: Thing) { x.head(); }',
                'src/plain.rs': 'fn f(x: Thing) { x.head(); }'}
        failures, _ = self.world(core=core)
        self.assertTrue(any(f.startswith('Repository::head has no production caller') for f in failures), failures)

    def test_comments_and_strings_are_not_calls(self):
        core = {'src/reads.rs': CORE['src/reads.rs'].replace(
            'repo.head();', '// repo.head();\nlet s = "repo.head()"; /* Repository::head */')}
        failures, _ = self.world(core=core)
        self.assertTrue(any(f.startswith('Repository::head has no production caller') for f in failures), failures)

    def test_a_trait_is_adopted_by_an_impl_not_by_a_mention(self):
        core = {'src/ops_user.rs': 'use gwz_git::ops; fn go() { ops::commit(); }\n// impl gwz_git::ops::Gate for S\n'}
        failures, _ = self.world(core=core)
        self.assertTrue(any(f.startswith('Gate has no production caller') for f in failures), failures)

    def test_an_import_is_not_a_call_but_a_called_import_is(self):
        gate = 'impl gwz_git::ops::Gate for S {}\n'
        imported_only = {'src/ops_user.rs': 'use gwz_git::ops::commit;\n' + gate}
        failures, _ = self.world(core=imported_only)
        self.assertTrue(any(f.startswith('ops::commit has no production caller') for f in failures), failures)
        for use in ('use gwz_git::ops::commit;', 'use gwz_git::ops::{commit, other};'):
            called = {'src/ops_user.rs': use + ' fn go() { commit(); }\n' + gate}
            self.assertEqual(self.world(core=called)[0], [], use)
        shadow = {'src/ops_user.rs': 'use gwz_git::ops; fn go() { local::commit(); x.commit(); }\n' + gate}
        failures, _ = self.world(core=shadow)
        self.assertTrue(any(f.startswith('ops::commit has no production caller') for f in failures), failures)

    def test_a_dated_exception_allows_an_unconsumed_capability_and_is_counted(self):
        core = {'src/ops_user.rs': 'impl gwz_git::ops::Gate for S {}\n'}
        failures, stats = self.world(core=core, config={'unconsumed': [entry('ops::commit')]})
        self.assertEqual(failures, [])
        self.assertEqual(stats, (6, 5, 1))

    def test_an_exception_for_a_capability_that_is_now_consumed_is_stale(self):
        failures, _ = self.world(config={'unconsumed': [entry('Repository::head')]})
        self.assertEqual(len(failures), 1)
        self.assertIn('consumed now', failures[0])
        self.assertIn('src/reads.rs', failures[0])

    def test_an_exception_naming_nothing_or_lacking_its_reasons_fails(self):
        failures, _ = self.world(config={'unconsumed': [entry('Repository::gone')]})
        self.assertTrue(any('names no public gwz-git capability' in f for f in failures), failures)
        bad = entry('Repository::head')
        del bad['exit']
        failures, _ = self.world(config={'unconsumed': [bad]})
        self.assertTrue(any('lacks exit' in f for f in failures), failures)
        failures, _ = self.world(config={'unconsumed': [entry('Repository::head', dated='soon')]})
        self.assertTrue(any('YYYY-MM-DD' in f for f in failures), failures)
        failures, _ = self.world(config={'unconsumed': [entry('Repository::head', reason=' ')]})
        self.assertTrue(any('must say something' in f for f in failures), failures)
        failures, _ = self.world(config={'unconsumed': [entry('Repository::head'), entry('Repository::head')]})
        self.assertTrue(any('listed twice' in f for f in failures), failures)

    def test_the_guide_must_document_every_export_and_every_capability(self):
        failures, _ = self.world(guide=GUIDE.replace('Flags', 'Bits'))
        self.assertTrue(any(f.startswith('Flags is exported by gwz-git') for f in failures), failures)
        failures, _ = self.world(guide=GUIDE.replace('`Flags::make()` builds flags.', ''))
        self.assertEqual(len(failures), 1, failures)
        self.assertTrue(failures[0].startswith('Flags::make is public in gwz-git'), failures)
        failures, _ = self.world(guide=GUIDE.replace('pub fn head(&self) -> u8;', ''))
        self.assertTrue(any(f.startswith('Repository::head is public in gwz-git but') and 'code block' in f
                            for f in failures), failures)

    def test_the_guide_may_not_declare_what_gwz_git_does_not_export(self):
        failures, _ = self.world(guide=GUIDE.replace('pub fn head(&self) -> u8;',
                                                     'pub fn head(&self) -> u8;\n    pub fn phantom(&self);'))
        self.assertEqual(len(failures), 1)
        self.assertIn('declares pub fn phantom', failures[0])

    def test_a_missing_gwz_git_checkout_is_an_error_not_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(SystemExit):
                adoption.surface(Path(directory))

    def test_the_real_trees_are_green_under_the_real_config(self):
        sibling = Path(os.environ.get('GWZ_GIT_CHECKOUT') or adoption.ROOT.parent / 'gwz-git')
        if not (sibling / 'src/lib.rs').is_file():
            self.skipTest(f'no gwz-git checkout at {sibling}')
        config = json.loads(adoption.CONFIG.read_text(encoding='utf-8'))
        self.assertEqual(adoption.check(adoption.ROOT, sibling, config)[0], [])
        for item in config['unconsumed']:
            self.assertEqual(sorted(item), sorted(adoption.CONFIG_FIELDS))


if __name__ == '__main__':
    unittest.main()
