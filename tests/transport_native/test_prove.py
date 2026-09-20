import hashlib
import io
import tarfile
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
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode='w') as archive:
            for name, content in self.original.items():
                entry = tarfile.TarInfo(name)
                entry.size = len(content)
                entry.mode = 0o644
                archive.addfile(entry, io.BytesIO(content))
        self.archive = output.getvalue()
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
        with patch.object(prove.subprocess, 'check_output', return_value=self.archive) as command:
            result = prove.copy_member(self.source, self.root / 'copy', self.pin)
            command.assert_called_once_with(
                ['git', '--no-replace-objects', 'archive', '--format=tar', prove.RELEASE], cwd=self.source)
            return result

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


if __name__ == '__main__':
    unittest.main()
