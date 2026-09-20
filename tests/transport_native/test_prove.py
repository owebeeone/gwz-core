import hashlib
import os
import subprocess
import tempfile
from pathlib import Path
from unittest.mock import patch
import unittest

import prove


class LockedGraphTests(unittest.TestCase):
    def setUp(self):
        self.original = (prove.ROOT / 'Cargo.lock').read_text()
        blocks = self.original.split('[[package]]')
        for index, block in enumerate(blocks[1:], 1):
            if '\nname = "git2"\n' in block:
                blocks[index] = '\n'.join(
                    line for line in block.splitlines()
                    if not line.startswith(('source =', 'checksum ='))
                ) + '\n'
        self.patched = '[[package]]'.join(blocks)

    def test_only_the_git2_source_may_change(self):
        prove.verify_lock(self.original, self.patched)
        for before, after in [
            ('name = "libgit2-sys"', 'name = "unexpected-native-library"'),
            ('name = "tempfile"', 'name = "unreviewed-helper"'),
            ('version = "0.21.0"', 'version = "0.21.1"'),
        ]:
            with self.subTest(change=after), self.assertRaises(SystemExit):
                prove.verify_lock(self.original, self.patched.replace(before, after))

    def test_stock_or_ambiguous_source_cannot_claim_patched_provenance(self):
        with self.assertRaises(SystemExit):
            prove.verify_lock(self.original, self.original)
        with self.assertRaises(SystemExit):
            prove.verify_lock(self.patched, self.patched)
        with self.assertRaises(SystemExit):
            prove.verify_lock(self.original, self.patched + '\n[[package]]\nname = "git2"\nversion = "0.21.0"\n')


class MemberSourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'member'
        self.source.mkdir()
        self.original = {
            'Cargo.toml': b'libgit2-sys = { path = "libgit2-sys", version = "0.18.4" }\n',
            'src/remote_callbacks.rs': b'original callbacks',
            'src/transport.rs': b'original transport',
            'src/lib.rs': b'unchanged library',
        }
        self.blobs = {hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest(): data
                      for data in self.original.values()}
        self.listing = b''.join(b'100644 blob ' + oid.encode() + b'\t' + name.encode() + b'\0'
                                for name, oid in zip(self.original, self.blobs))
        self.pin = {'patched_files': {}}
        for name, content in self.original.items():
            if name.startswith('src/') and name != 'src/lib.rs':
                content = b'patched ' + content
                self.pin['patched_files'][name] = hashlib.sha256(content).hexdigest()
            if name == 'Cargo.toml':
                content = b'libgit2-sys = "=0.18.8"\n'
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)

    def admit(self):
        def objects(args, cwd):
            self.assertEqual(cwd, self.source)
            self.assertEqual(args[:2], ['git', '--no-replace-objects'])
            if args[2:] == ['ls-tree', '-rz', prove.RELEASE]:
                return self.listing
            self.assertEqual(args[2:4], ['cat-file', 'blob'])
            return self.blobs[args[4]]
        with patch.object(prove.subprocess, 'check_output', side_effect=objects):
            return prove.copy_member(self.source, self.root / 'copy', self.pin)

    def test_exact_release_plus_binding_and_manifest_is_isolated(self):
        result = self.admit()
        self.assertEqual((result / 'src/lib.rs').read_bytes(), b'unchanged library')
        (result / 'src/lib.rs').write_bytes(b'isolated edit')
        self.assertEqual((self.source / 'src/lib.rs').read_bytes(), b'unchanged library')

    def test_source_manifest_and_partial_patch_drift_are_refused(self):
        for name in self.original:
            with self.subTest(name=name):
                path = self.source / name
                original = path.read_bytes()
                path.write_bytes(original + b' drift')
                with self.assertRaises(SystemExit):
                    self.admit()
                path.write_bytes(original)

    def test_extra_ignored_build_input_and_missing_file_are_refused(self):
        extra = self.source / 'build.rs'
        extra.write_text('unapproved build code')
        with self.assertRaises(SystemExit):
            self.admit()
        extra.unlink()
        (self.source / 'src/lib.rs').unlink()
        with self.assertRaises(SystemExit):
            self.admit()

    def test_symlink_substitution_is_refused(self):
        path = self.source / 'src/lib.rs'
        path.unlink()
        path.symlink_to(self.source / 'src/transport.rs')
        with self.assertRaises(SystemExit):
            self.admit()

    def test_real_repository_attributes_cannot_hide_a_missing_release_file(self):
        patched = {name: (self.source / name).read_bytes() for name in self.original}
        for name, content in self.original.items():
            (self.source / name).write_bytes(content)
        env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}
        def git(*args):
            return subprocess.check_output(['git', *args], cwd=self.source, env=env)
        git('init', '--quiet')
        git('add', '.')
        git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
            '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'release fixture')
        release = git('rev-parse', 'HEAD').decode().strip()
        for name, content in patched.items():
            (self.source / name).write_bytes(content)
        (self.source / '.git/info/attributes').write_text('src/lib.rs export-ignore\n')
        (self.source / 'src/lib.rs').unlink()
        with patch.object(prove, 'RELEASE', release), self.assertRaises(SystemExit):
            prove.copy_member(self.source, self.root / 'copy', self.pin)


if __name__ == '__main__':
    unittest.main()
