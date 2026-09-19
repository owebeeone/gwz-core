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


if __name__ == '__main__':
    unittest.main()
