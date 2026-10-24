"""Tests for the spawn gate minting guard (step 1.3a): a planted constructor outside the composition turns it red."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('spawn_gate_minting', Path(__file__).with_name('check_spawn_gate_minting.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

CLEAN = {
    'src/lib.rs': 'mod git;\nmod transport_host;\n',
    'src/git.rs': 'mod gitbackend;\n',
    'src/git/gitbackend.rs': 'mod backend;\n',
    'src/git/gitbackend/backend.rs': 'fn backend() { let gate = SpawnGate::ungated(); }\n',
    'src/transport_host.rs': 'pub mod composition;\nmod other;\n',
    'src/transport_host/composition.rs': 'fn gate() -> SpawnGate { SpawnGate::process() }\n',
    'src/transport_host/other.rs': 'fn other() { let slots = HelperSlots::with_gate(gate()); }\n',
}


def repo(files):
    directory = tempfile.TemporaryDirectory()
    root = Path(directory.name)
    for name, text in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')
    return directory, root


def errors_for(**changes):
    files = dict(CLEAN)
    for name, text in changes.items():
        files[name.replace('__', '/').replace('_rs', '.rs')] = text
    directory, root = repo(files)
    with directory:
        return checker.check(root)


class Minting(unittest.TestCase):
    def test_the_clean_tree_passes(self):
        self.assertEqual(errors_for(), [])

    def test_a_planted_spawn_gate_new_in_production_code_is_red(self):
        errors = errors_for(src__transport_host__other_rs='fn other() { let gate = SpawnGate::new(); }\n')
        self.assertEqual(len(errors), 1)
        self.assertIn('SpawnGate::new', errors[0])
        self.assertIn('src/transport_host/other.rs:1', errors[0])

    def test_every_other_minting_spelling_is_red(self):
        for call in ('SpawnGate::default()', 'SpawnGate::with_exclusion(true)', 'HelperSlots::new()',
                     'NativeHost::new(worker)', 'SpawnGate::process()', 'SpawnGate::ungated()'):
            with self.subTest(call=call):
                errors = errors_for(src__transport_host__other_rs=f'fn other() {{ let _x = {call}; }}\n')
                self.assertEqual(len(errors), 1, errors)
                self.assertIn(call.split('(')[0], errors[0])

    def test_test_only_code_is_exempt(self):
        for source in (
            '#[cfg(test)]\nmod tests { fn t() { SpawnGate::new(); HelperSlots::new(); } }\n',
            'cfg_if::cfg_if! { if #[cfg(test)] { fn t() { SpawnGate::default(); } } }\n',
            '#[cfg(test)]\nfn t() { SpawnGate::with_exclusion(true); }\n',
        ):
            with self.subTest(source=source):
                self.assertEqual(errors_for(src__transport_host__other_rs=source), [])

    def test_a_file_reached_only_by_a_test_module_is_exempt(self):
        errors = errors_for(
            src__transport_host_rs='pub mod composition;\n#[cfg(test)]\nmod fixtures;\n',
            src__transport_host__fixtures_rs='fn f() { SpawnGate::new(); }\n',
        )
        self.assertEqual(errors, [])

    def test_a_comment_or_a_string_is_not_a_call(self):
        source = '// SpawnGate::new()\nconst S: &str = "HelperSlots::new()";\n'
        self.assertEqual(errors_for(src__transport_host__other_rs=source), [])

    def test_the_allowed_sites_must_hold_their_call(self):
        errors = errors_for(src__transport_host__composition_rs='fn gate() {}\n')
        self.assertEqual(len(errors), 1)
        self.assertIn('MISSING', errors[0])
        errors = errors_for(src__git__gitbackend__backend_rs='fn backend() {}\n')
        self.assertEqual(len(errors), 1)
        self.assertIn('SpawnGate::ungated', errors[0])

    def test_gwz_core_itself_passes(self):
        self.assertEqual(checker.check(checker.ROOT), [])


if __name__ == '__main__':
    unittest.main()
