import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location("boundary", Path(__file__).with_name("check_target_selection_boundary.py"))
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)

class SelectionBoundaryTests(unittest.TestCase):
    def test_literal_root_rescan_is_rejected(self):
        self.assertTrue(boundary.findings('if selection.targets.iter().any(|x| x == "@root") { }'))

    def test_multiline_and_empty_envelope_rescans_are_rejected(self):
        self.assertTrue(boundary.findings('selection\n .member_ids.is_empty()'))
        self.assertTrue(boundary.findings('request.meta.selection.is_some()'))

    def test_passing_selection_to_owner_is_allowed(self):
        self.assertFalse(boundary.findings('resolve_action_targets(&manifest, request.meta.selection.as_ref(), action)'))

    def test_current_source_tree_passes(self):
        self.assertEqual([], boundary.scan(boundary.ROOT))

if __name__ == '__main__': unittest.main()
