"""The Windows qualification switch is retired (GwzTransportWindowsParityPlan.md, step 5.1; GwzV110Plan.md, S4.5).

gwz-core's Windows candidate was once reachable only under `all(windows, gwz_transport_candidate,
gwz_windows_https_qualification)`. S4.5 turns every such site into the plain unix or windows arm under
`gwz_transport_candidate`, so this file asserts the inverse of what it once did: the cfg name is gone from gwz-core, and
the sites that carried it select the build by platform and the candidate switch alone.

gwz-cli (step 5.2) and gwz-py (step 5.3) no longer name the switch either; the class that checked their sites is gone.
Only this repository's assertions remain.

Inspects inactive branches with the source lexer; reads source only, so it runs without a Windows host or a build.
"""
from pathlib import Path
import importlib.util
import itertools
import unittest

CORE = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("cfg_guard", CORE / "scripts/checks/check_cfg_boundaries.py")
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)
# Spelled in two halves so that this file does not name the switch it asserts the absence of.
SWITCH = "gwz_windows_https_" + "qualification"
# The shape the switch's sites had before S4.5 gave them the plain arm below; no file may carry it again.
UNION = f"any(all(unix, gwz_transport_candidate), all(windows, gwz_transport_candidate, {SWITCH}))"
# The shape S4.5 gives a site whose unix and windows arms are one.
PLATFORMS = "all(gwz_transport_candidate, any(unix, windows))"
SCANNED = ("src", "tests", "scripts", ".github", "build.rs", "Cargo.toml", "BUILD.bazel", "AGENTS.md")


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


def files_under(root):
    if root.is_file():
        yield root
        return
    for path in sorted(root.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            yield path


class CoreBoundary(unittest.TestCase):
    def test_the_cfg_name_is_gone(self):
        named = []
        for entry in SCANNED:
            for path in files_under(CORE / entry):
                if path == Path(__file__).resolve():
                    continue
                try:
                    text = path.read_text(encoding="utf-8")
                except UnicodeDecodeError:
                    continue
                if SWITCH in text:
                    named.append(str(path.relative_to(CORE)))
        self.assertEqual(named, [], "the retired qualification switch is still named")

    def test_build_script_declares_only_the_two_candidate_switches(self):
        text = (CORE / "build.rs").read_text(encoding="utf-8")
        self.assertIn("cfg(gwz_transport_candidate)", text)
        self.assertIn("cfg(gwz_session_candidate)", text)
        self.assertEqual(text.count("rustc-check-cfg"), 2)

    def test_host_module_is_selected_by_platform_and_the_candidate_switch_alone(self):
        host = enclosing_predicate((CORE / "src/lib.rs").read_text(), "mod transport_host")
        for windows, unix, candidate in itertools.product([False, True], repeat=3):
            if windows and unix:
                continue
            enabled = {name for name, value in [("windows", windows), ("unix", unix), ("gwz_transport_candidate", candidate)] if value}
            self.assertEqual(evaluate(host, enabled), candidate and (unix or windows), enabled)

    def test_the_candidate_switch_alone_has_no_refusal_to_compile(self):
        source = (CORE / "src/lib.rs").read_text()
        self.assertNotIn("compile_error!", source)

    def test_union_sites_use_the_platform_and_candidate_predicate(self):
        for name in ["src/git/mod.rs", "src/git/gitbackend/transport_binding.rs", "src/lib.rs"]:
            self.assertIn(PLATFORMS, (CORE / name).read_text(), name)
            self.assertNotIn(UNION, (CORE / name).read_text(), name)

    def test_fixed_backend_rule_is_at_shared_request_constructor(self):
        source = (CORE / "src/transport_host/mod.rs").read_text()
        body = source[source.index("async fn open_request("):source.index("pub async fn shutdown(")]
        # One backend for every build: configured credential helpers are on (plan step 4.4), so there is no
        # Windows-only constructor that leaves them off.
        self.assertIn("Git2Backend::new().with_host_context(guard.context.clone())", body)
        self.assertNotIn("without_credential_helpers", body)
        self.assertNotIn("cfg!", body)


if __name__ == "__main__":
    unittest.main()
