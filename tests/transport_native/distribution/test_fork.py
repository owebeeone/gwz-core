import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location('fork', Path(__file__).with_name('fork.py'))
fork = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fork)


class ForkManifestTests(unittest.TestCase):
    def test_identity_changes_only_at_package_boundary(self):
        source = '''[package]
name = "git2"
version = "0.21.0"
description = "upstream"
repository = "https://github.com/rust-lang/git2-rs"
[lib]
name = "git2"
[dependencies.libgit2-sys]
version = "0.18.4"
'''
        result = fork.manifest(source)
        self.assertIn('name = "gwz-git2"', result)
        self.assertIn('version = "0.21.0-gwz.1"', result)
        self.assertIn('[lib]\nname = "git2"', result)
        self.assertIn('[dependencies.libgit2-sys]\nversion = "0.18.4"', result)
        self.assertIn('[package.metadata.gwz-fork]', result)

    def test_wrong_upstream_identity_is_refused(self):
        for source in ['[package]\nname = "foreign"\nversion = "0.21.0"\n',
                       '[package]\nname = "git2"\nversion = "0.22.0"\n']:
            with self.assertRaises(ValueError):
                fork.manifest(source)

    def test_fixture_alias_is_explicit_and_single(self):
        source = (Path(__file__).parents[1] / 'Cargo.toml').read_text()
        result = fork.fixture_manifest(source)
        self.assertIn('git2 = { package = "gwz-git2", version = "=0.21.0-gwz.1",', result)
        self.assertNotIn('version = "=0.21.0"', result)
        with self.assertRaises(ValueError):
            fork.fixture_manifest(result)


if __name__ == '__main__':
    unittest.main()
