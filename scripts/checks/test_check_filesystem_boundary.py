import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('filesystem_boundary', Path(__file__).with_name('check_filesystem_boundary.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

class BoundaryTests(unittest.TestCase):
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

if __name__ == '__main__':
    unittest.main()
