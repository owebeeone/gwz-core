"""Public selection/caller regression; inspect inactive branches with the source lexer."""
from pathlib import Path
import importlib.util
import unittest
import itertools

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("cfg_guard", ROOT / "gwz-core/scripts/checks/check_cfg_boundaries.py")
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)
QUAL = "all(windows, gwz_transport_candidate, gwz_windows_https_qualification)"
UNION = "any(all(unix, gwz_transport_candidate), all(windows, gwz_transport_candidate, gwz_windows_https_qualification))"

def evaluate(tokens, enabled):
    words = [token.text for token in tokens]
    def expression(i):
        name = words[i]
        if i + 1 >= len(words) or words[i + 1] != "(":
            return name in enabled, i + 1
        children = []
        i += 2
        while words[i] != ")":
            value, i = expression(i)
            children.append(value)
            if words[i] == ",":
                i += 1
        return {"all": all, "any": any, "not": lambda args: not args[0]}[name](children), i + 1
    result, end = expression(0)
    assert end == len(words)
    return result


def enclosing_predicate(source, item):
    analysis = guard.Analysis(source)
    for i in range(analysis.n):
        if analysis.text(i) == "cfg" and analysis.text(i + 1) == "(":
            stop = analysis.match[i + 1]
            boundary = stop + 2
            if analysis.text(boundary) == "{":
                body_end = analysis.match[boundary]
                body = " ".join(t.text for t in analysis.toks[boundary:body_end])
                if item in body:
                    return analysis.toks[i + 2:stop]
    raise AssertionError("missing enclosing cfg: " + item)


class Boundary(unittest.TestCase):
    def test_selected_host_and_invalid_build_predicates_cover_all_configurations(self):
        host = enclosing_predicate((ROOT / "gwz-core/src/lib.rs").read_text(), "mod transport_host")
        for windows, unix, candidate, qualification in itertools.product([False, True], repeat=4):
            if windows and unix:
                continue
            enabled = {name for name, value in [("windows", windows), ("unix", unix), ("gwz_transport_candidate", candidate), ("gwz_windows_https_qualification", qualification)] if value}
            expected = candidate and (unix or windows and qualification)
            self.assertEqual(evaluate(host, enabled), expected, enabled)
            for name in ["gwz-core/src/lib.rs", "gwz-cli/src/lib.rs", "gwz-py/native/src/lib.rs"]:
                refusal = enclosing_predicate((ROOT / name).read_text(), "compile_error !")
                self.assertEqual(evaluate(refusal, enabled), qualification and not (windows and candidate), name)

    def test_actual_routes_use_only_valid_union_and_reject_illegal_qualification(self):
        for name in ["gwz-core/src/lib.rs", "gwz-cli/src/lib.rs", "gwz-py/native/src/lib.rs"]:
            source = (ROOT / name).read_text()
            self.assertIn("gwz_windows_https_qualification", source, name)
            self.assertIn("compile_error!", source, name)
        for name in ["gwz-core/src/git/mod.rs", "gwz-core/src/git/gitbackend/transport_binding.rs", "gwz-cli/src/globalargs/dispatch.rs", "gwz-py/native/src/route.rs"]:
            self.assertIn(UNION, (ROOT / name).read_text(), name)

    def test_actual_original_entry_capture_precedes_detach(self):
        source = (ROOT / "gwz-py/native/src/client_host.rs").read_text()
        self.assertIn(UNION, source)
        network = source[source.index("fn network("):]
        self.assertLess(network.index("NativeCaller::capture"), network.index("route::capture"))
        for method in ["fn call(", "fn submit("]:
            body = source[source.index(method):]
            self.assertLess(body.index("self.network("), body.index("py.detach("))
        source = (ROOT / "gwz-cli/src/globalargs/dispatch.rs").read_text()
        self.assertIn(UNION, source)
        self.assertLess(source.index("NativeCaller::capture"), source.index("with_local_transport_native"))

    def test_fixed_backend_rule_is_at_shared_request_constructor(self):
        source = (ROOT / "gwz-core/src/transport_host/mod.rs").read_text()
        body = source[source.index("async fn open_request("):source.index("pub async fn remove_cli(")]
        self.assertIn(QUAL, body)
        self.assertIn("Git2Backend::without_credential_helpers().with_host_context(guard.context.clone())", body)
        self.assertIn("Git2Backend::new().with_host_context(guard.context.clone())", body)

if __name__ == "__main__":
    unittest.main()
