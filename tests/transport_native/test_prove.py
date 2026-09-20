import hashlib
import os
import shutil
import subprocess
from pathlib import Path, PureWindowsPath
from unittest.mock import patch
import unittest

import prove


def fixture_tempdir():
    return prove._temporary_directory('gwz-native-test-')


class WindowsFlavorPath:
    """Filesystem-backed path whose display key follows Windows separators."""

    def __init__(self, value):
        self.inner = Path(value)

    def relative_to(self, other):
        return type(self)(self.inner.relative_to(os.fspath(other)))

    def __truediv__(self, child):
        return type(self)(self.inner / os.fspath(child))

    def __fspath__(self):
        return os.fspath(self.inner)

    def __str__(self):
        return str(PureWindowsPath(self.inner.as_posix()))

    def as_posix(self):
        return self.inner.as_posix()


class WindowsPathKeyTests(unittest.TestCase):
    def fixture(self, git_kind):
        temporary = fixture_tempdir()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        source = root / 'source'
        source.mkdir()
        nested = source / 'nested'
        (nested / 'deep').mkdir(parents=True)
        (nested / 'deep' / 'value.txt').write_bytes(b'nested')
        if os.name != 'nt':
            (nested / 'literal\\name.txt').write_bytes(b'literal')
        nested_git = nested / '.git'
        if git_kind == 'directory':
            nested_git.mkdir()
            (nested_git / 'config').write_text('ignored')
        else:
            nested_git.write_text('gitdir: outside')
        expected = {'nested/deep/value.txt': (0o100644, b'nested')}
        if os.name != 'nt':
            expected['nested/literal\\name.txt'] = (0o100644, b'literal')
        return source, root / 'destination', expected

    def verify_windows_keys(self, source, destination, expected, excluded):
        with patch.object(prove, 'Path', WindowsFlavorPath):
            return prove.verify_copy(source, destination, expected, excluded)

    def test_nested_windows_keys_and_git_directory_or_file_are_excluded(self):
        for git_kind in ('directory', 'file'):
            with self.subTest(git_kind=git_kind):
                source, destination, expected = self.fixture(git_kind)
                self.verify_windows_keys(source, destination, expected,
                                         ('.git', 'nested/.git', 'target'))
                for name, (_, content) in expected.items():
                    self.assertEqual((destination / name).read_bytes(), content)

    def test_windows_key_normalization_keeps_admission_guards(self):
        source, destination, expected = self.fixture('file')
        extra = source / 'nested' / 'unexpected.txt'
        extra.write_bytes(b'unexpected')
        with self.assertRaises(SystemExit):
            self.verify_windows_keys(source, destination, expected,
                                     ('.git', 'nested/.git', 'target'))
        extra.unlink()
        expected['nested/deep/value.txt'] = (0o100644, b'wrong')
        with self.assertRaises(SystemExit):
            self.verify_windows_keys(source, destination, expected,
                                     ('.git', 'nested/.git', 'target'))
        expected['nested/deep/value.txt'] = (0o100644, b'nested')
        guarded = source / 'nested' / 'deep' / 'value.txt'
        guarded.unlink()
        guarded.symlink_to(source / 'nested' / 'deep' / 'other.txt')
        with self.assertRaises(SystemExit):
            self.verify_windows_keys(source, destination, expected,
                                     ('.git', 'nested/.git', 'target'))


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
        self.temp = fixture_tempdir()
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
        def objects(args, cwd, input=None):
            self.assertEqual(cwd, self.source)
            self.assertEqual(args[:2], ['git', '--no-replace-objects'])
            if args[2:] == ['ls-tree', '-rz', prove.RELEASE]:
                return self.listing
            self.assertEqual(args[2:], ['cat-file', '--batch'])
            return b''.join(oid + b' blob ' + str(len(self.blobs[oid.decode()])).encode() +
                            b'\n' + self.blobs[oid.decode()] + b'\n'
                            for oid in input.splitlines())
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


class NativeLockTests(unittest.TestCase):
    def test_native_mode_changes_only_two_source_provenances(self):
        original = (prove.ROOT / 'Cargo.lock').read_text()
        blocks = original.split('[[package]]')
        for i, block in enumerate(blocks[1:], 1):
            if any('\nname = "' + name + '"\n' in block
                   for name in ('git2', 'libgit2-sys')):
                blocks[i] = '\n'.join(line for line in block.splitlines()
                                      if not line.startswith(('source =', 'checksum ='))) + '\n'
        patched = '[[package]]'.join(blocks)
        prove.verify_lock(original, patched, native=True)
        for changed in [patched.replace('0.18.8+1.9.7', '0.18.8+1.9.6'),
                        patched.replace('name = "tempfile"', 'name = "foreign"'),
                        patched.replace(' "libz-sys",', '')]:
            with self.assertRaises(SystemExit):
                prove.verify_lock(original, changed, native=True)
        with self.assertRaises(SystemExit):
            prove.verify_lock(original, original, native=True)

    def test_native_tree_is_exact_and_submodule_head_is_checked(self):
        with fixture_tempdir() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(['git', *args], cwd=root)
            git('init', '--quiet')
            (root / 'source.c').write_text('original\n')
            git('add', '.')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'native fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            expected = prove.read_tree(root, revision)
            prove.verify_copy(root, root.parent / (root.name + '-copy'), expected)
            shutil.rmtree(root.parent / (root.name + '-copy'))
            (root / 'source.c').write_text('unapproved\n')
            with self.assertRaises(SystemExit):
                prove.verify_copy(root, root / 'unused', expected)
            with self.assertRaises(SystemExit):
                prove.check_revision(root, '0' * 40)


class NativeMemberTests(unittest.TestCase):
    def test_native_sys_metadata_and_checkout_drift_are_refused(self):
        fixture = MemberSourceTests()
        fixture.setUp()
        self.addCleanup(fixture.temp.cleanup)
        source = fixture.source
        original = {name: (0o100644, content) for name, content in fixture.original.items()}
        original['.gitmodules'] = (0o100644, b'url = https://github.com/libgit2/libgit2\n')
        aligned = {'libgit2-sys/Cargo.toml': (0o100644, b'version = "0.18.8+1.9.7"\n')}
        native = {'source.c': (0o100644, b'reviewed C source\n')}
        pin = {**fixture.pin, 'native': {'sys_revision': 'sys-pin', 'c_revision': 'c-pin'}}
        (source / 'Cargo.toml').write_bytes(
            b'libgit2-sys = { path = "libgit2-sys", version = "=0.18.8" }\n')
        entries = {**aligned, '.gitmodules': (0o100644, b'url = https://github.com/owebeeone/libgit2\n'),
                   'libgit2-sys/libgit2/source.c': native['source.c']}
        for name, (_, data) in entries.items():
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        def tree(path, revision):
            if revision == prove.RELEASE:
                self.assertEqual(path, source)
                return original.copy()
            if revision == 'sys-pin':
                self.assertEqual(path, source)
                return aligned.copy()
            self.assertEqual((path, revision), (source / 'libgit2-sys/libgit2', 'c-pin'))
            return native.copy()
        def revision(args, cwd):
            self.assertEqual(args[:3], ['git', '--no-replace-objects', 'rev-parse'])
            self.assertIn(args[3], ['HEAD', 'HEAD:libgit2-sys/libgit2'])
            return b'c-pin\n'
        with patch.object(prove, 'read_tree', side_effect=tree), patch.object(
                prove.subprocess, 'check_output', side_effect=revision):
            prove.copy_member(source, fixture.root / 'copy', pin)
            for name in entries:
                path = source / name
                saved = path.read_bytes()
                path.write_bytes(saved + b'drift')
                with self.subTest(name=name), self.assertRaises(SystemExit):
                    prove.copy_member(source, fixture.root / 'copy', pin)
                path.write_bytes(saved)
            (source / 'libgit2-sys/libgit2/source.c').unlink()
            with self.assertRaises(SystemExit):
                prove.copy_member(source, fixture.root / 'copy', pin)


if __name__ == '__main__':
    unittest.main()
