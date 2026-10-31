import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("boundary", Path(__file__).with_name("check_member_step_boundary.py"))
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)

FILE = "src/workspace_ops/handle_x.rs"


def scan(source: str, allowed=None) -> list[str]:
    return boundary.findings(source, FILE, {} if allowed is None else allowed)


class SessionHostRuleTests(unittest.TestCase):
    def test_a_session_host_path_is_rejected(self):
        self.assertTrue(scan("use crate::session_host::members::BoundMemberLocks;\n"))
        self.assertTrue(scan("fn f() { let _ = crate::session_host::context::HostContext::new(); }"))

    def test_the_operation_module_is_allowed(self):
        self.assertEqual([], scan("use crate::operation::BoundMemberLocks;\n"))

    def test_a_mention_in_a_comment_or_a_string_is_not_a_use(self):
        self.assertEqual([], scan("// session_host\n/// the session_host serves it\n/* session_host */\nfn f() { let _ = \"session_host\"; }"))

    def test_the_session_host_crate_is_not_the_module(self):
        self.assertEqual([], scan("use gwz_session_host::CancellationToken;\n"))


class MemberStepRuleTests(unittest.TestCase):
    def test_an_unlocked_call_is_rejected(self):
        for call in ("backend.fetch(&path, remote)?;", "backend.push_prepared(&path, plan)?;",
                     "backend.tag_fetch(&path, remote)?;", "backend.push(&path, remote, spec)?;",
                     "scoped_backend.fetch(&path, remote)?;", "self.backend.fetch(&path, remote)?;"):
            with self.subTest(call=call):
                self.assertEqual(1, len(scan(f"fn f() {{\n    {call}\n}}\n")), call)

    def test_a_multiline_chain_is_rejected(self):
        self.assertEqual(1, len(scan("fn f() {\n    backend\n        .push(&path, remote, spec)\n        .map_err(e)?;\n}\n")))

    def test_a_guard_binding_before_the_call_passes(self):
        self.assertEqual([], scan("fn f() {\n    let _guard = steps.enter(root, id)?;\n    backend.fetch(&path, remote)?;\n}\n"))

    def test_a_guard_in_an_enclosing_block_passes(self):
        self.assertEqual([], scan(
            "fn f() {\n    let _guard = steps.enter(root, id)?;\n    if ok {\n        for x in xs {\n"
            "            backend.tag_fetch(&path, remote)?;\n        }\n    }\n}\n"))

    def test_a_match_on_the_entry_passes(self):
        self.assertEqual([], scan(
            "fn f() {\n    match steps.enter(root, id) {\n        Ok(_guard) => finish(backend.push_prepared(&p, plan)),\n"
            "        Err(error) => refuse(error),\n    }\n}\n"))

    def test_a_closure_after_the_entry_passes(self):
        self.assertEqual([], scan(
            "fn f() {\n    let r = steps.enter(root, id).and_then(|_guard| {\n        backend.fetch(&p, remote)?;\n        Ok(())\n    });\n}\n"))

    def test_an_entry_in_a_block_that_closed_before_the_call_guards_nothing(self):
        self.assertEqual(1, len(scan(
            "fn f() {\n    {\n        let _guard = steps.enter(root, id)?;\n        backend.fetch(&p, remote)?;\n    }\n"
            "    backend.fetch(&p, remote)?;\n}\n")))

    def test_an_entry_after_the_call_guards_nothing(self):
        self.assertEqual(1, len(scan("fn f() {\n    backend.fetch(&p, remote)?;\n    let _guard = steps.enter(root, id)?;\n}\n")))

    def test_an_entry_in_another_function_guards_nothing(self):
        self.assertEqual(1, len(scan(
            "fn g() {\n    let _guard = steps.enter(root, id)?;\n}\nfn f() {\n    backend.fetch(&p, remote)?;\n}\n")))

    def test_an_entry_in_a_sibling_arm_guards_nothing(self):
        self.assertEqual(1, len(scan(
            "fn f() {\n    match op {\n        A => {\n            let _guard = steps.enter(root, id)?;\n"
            "            backend.fetch(&p, remote)?;\n        }\n        B => {\n            backend.tag_fetch(&p, remote)?;\n        }\n    }\n}\n")))

    def test_a_vec_push_is_not_a_network_update(self):
        self.assertEqual([], scan("fn f() {\n    targets.push(x);\n    responses.push(push_root(backend, &root, &request));\n}\n"))

    def test_a_call_in_a_comment_or_a_string_is_not_a_call(self):
        self.assertEqual([], scan("fn f() {\n    // backend.fetch(&p, remote)\n    let _ = \"backend.push(&p)\";\n}\n"))

    def test_an_inline_test_module_is_exempt(self):
        self.assertEqual([], scan("fn f() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n        backend.fetch(&p, remote);\n    }\n}\n"))

    def test_production_after_a_test_module_is_still_checked(self):
        self.assertEqual(1, len(scan("#[cfg(test)]\nmod tests {\n    fn t() {}\n}\nfn f() {\n    backend.fetch(&p, remote)?;\n}\n")))

    def test_lifetimes_and_char_literals_do_not_hide_a_call(self):
        self.assertEqual(1, len(scan("fn f<'a>(x: &'a str) {\n    let c = '{';\n    backend.fetch(&p, remote)?;\n}\n")))

    def test_the_allow_list_excuses_the_named_function_only(self):
        source = "fn read_only() {\n    backend.fetch(&p, remote)?;\n}\nfn other() {\n    backend.fetch(&p, remote)?;\n}\n"
        failures = scan(source, {(FILE, "read_only"): "reads only"})
        self.assertEqual(1, len(failures))
        self.assertIn("`other`", failures[0])


class TreeTests(unittest.TestCase):
    def test_current_source_tree_passes(self):
        self.assertEqual([], boundary.scan(boundary.ROOT))

    def test_the_scan_looks_at_the_calls_it_exists_for(self):
        # A scan that found no call would pass vacuously. The handlers have at least these network updates:
        # fetch (1), push (member and root: 2), pull_head (member, root and merge plan fetches: 3),
        # tag (push, fetch and remote delete: 3).
        calls = 0
        for path in boundary.production_files(boundary.ROOT):
            code = boundary.without_test_modules(boundary.blank(path.read_text(encoding="utf-8")))
            calls += len(boundary.UPDATE.findall(code))
        self.assertGreaterEqual(calls, 9)

    def test_a_stale_allow_list_entry_fails(self):
        failures = boundary.scan(boundary.ROOT, {("src/workspace_ops/handle_fetch.rs", "nothing"): "stale"})
        self.assertEqual(1, len(failures))
        self.assertIn("excuses no call", failures[0])

    def test_test_files_are_not_production_files(self):
        names = {path.name for path in boundary.production_files(boundary.ROOT)}
        self.assertNotIn("g29.rs", names)
        self.assertNotIn("tag_member_step.rs", names)
        self.assertIn("handle_fetch.rs", names)


if __name__ == "__main__":
    unittest.main()
