"""Tests for the process-global state ratchet (GwzCoreSessionDesign O9)."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import re
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('process_globals', Path(__file__).with_name('check_process_globals.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

TRANSPORT_ALLOWLIST = Path(__file__).with_name('process_globals_allowlist_gwz_transport.json')
BOUNDARY_WORKFLOW = checker.ROOT / '.github' / 'workflows' / 'checked-artifact-boundary.yml'
TRANSPORT_ALLOWLIST_ARG = '--allowlist scripts/checks/process_globals_allowlist_gwz_transport.json'


def found(source, features=frozenset()):
    return sorted((o.kind, o.name) for o in checker.analyze(source, features).occurrences)


class Detection(unittest.TestCase):
    def test_flags_shared_statics_thread_locals_and_ambient_reads(self):
        source = '''
static LOCK: std::sync::Mutex<u8> = std::sync::Mutex::new(0);
static mut RAW: u8 = 0;
static COUNTER: AtomicU64 = AtomicU64::new(0);
static LAZY: OnceLock<String> = OnceLock::new();
static CUSTOM: Registry = Registry::new();
thread_local! { static SLOT: Cell<u8> = const { Cell::new(0) }; }
fn f() {
    let home = std::env::var_os("HOME");
    let cwd = env::current_dir();
    git2::opts::set_server_timeout_in_milliseconds(1);
    opts::set_verify_owner_validation(false);
    std::panic::set_hook(hook);
    let h = dirs::home_dir();
    let p = unsafe { libc::getenv(name) };
    let child = std::process::Command::new("git").arg("tag").status();
    let helper = tokio::process::Command::new(program).spawn();
}
use std::env::{self, var};
'''
        self.assertEqual(found(source), sorted([
            ('static', 'LOCK'), ('static', 'RAW'), ('static', 'COUNTER'), ('static', 'LAZY'),
            ('static', 'CUSTOM'), ('thread_local', 'SLOT'), ('env', 'env::var_os'),
            ('env', 'env::current_dir'), ('libgit2', 'git2::opts::set_server_timeout_in_milliseconds'),
            ('libgit2', 'git2::opts::set_verify_owner_validation'), ('hook', 'panic::set_hook'),
            ('env', 'dirs::home_dir'), ('env', 'libc::getenv'), ('env', 'env::var'),
            ('process', 'Command::new("git")'), ('process', 'Command::new'),
        ]))

    def test_flags_libgit2_credential_helper_spawn_only(self):
        self.assertEqual(found('''
fn f() {
    let credential = git2::Cred::credential_helper(&config, url, None);
    let agent = git2::Cred::ssh_key_from_agent(user);
}
'''), [('process', 'Cred::credential_helper')])

    def test_flags_the_credential_helper_struct_as_the_same_spawn(self):
        # Safety P3-31 of Verdict-3: git2's public CredentialHelper performs
        # the spawn that Cred::credential_helper wraps, so both spellings are
        # one occurrence and a switch between them never reads as debt paid.
        self.assertEqual(found('''
fn f() {
    let found = git2::CredentialHelper::new(url).config(&config).execute();
    let policy = CredentialHelperPolicy::AllowConfigured;
    let helper: Option<CredentialHelper> = None;
}
'''), [('process', 'Cred::credential_helper')])

    def test_ignores_immutable_data_lifetimes_comments_and_literals(self):
        self.assertEqual(found('''
static NAMES: &[&str] = &["a", "b"];
static LIMIT: u32 = 3;
static TABLE: [u8; 4] = [0; 4];
const SHARED: AtomicU64 = AtomicU64::new(0);
fn f(x: &'static str) -> Box<dyn Fn() + Send + 'static> { todo!() }
// static HIDDEN: Mutex<u8> = Mutex::new(0);
/* std::env::var("X") /* nested */ thread_local! {} */
fn g() {
    let text = "static QUOTED: Mutex<u8>; std::env::var(\\"X\\")";
    let raw = r#"thread_local! { static R: Cell<u8> }"#;
    let brace = '{';
    let name = env!("CARGO_PKG_NAME");
    let optional = option_env!("GWZ_X");
    let value = self.env.var("HOME");
}
'''), [])

    def test_exempts_only_code_that_requires_test(self):
        source = '''
#[cfg(test)] static A: Mutex<u8> = Mutex::new(0);
#[cfg(all(test, unix))] fn b() { static B: AtomicU64 = AtomicU64::new(0); }
#[cfg(test)]
#[allow(dead_code)]
mod tests { fn c() { std::env::var("C"); } }
cfg_if::cfg_if! {
    if #[cfg(test)] { static D: Mutex<u8> = Mutex::new(0); }
    else { static E: Mutex<u8> = Mutex::new(0); }
}
cfg_if::cfg_if! {
    if #[cfg(not(test))] { static F: Mutex<u8> = Mutex::new(0); }
    else if #[cfg(unix)] { static G: Mutex<u8> = Mutex::new(0); }
    else { static H: Mutex<u8> = Mutex::new(0); }
}
#[cfg(not(test))] static I: Mutex<u8> = Mutex::new(0);
#[cfg(any(test, windows))] static J: Mutex<u8> = Mutex::new(0);
enum Mode { #[cfg(test)] Probe, Real }
static K: Mutex<u8> = Mutex::new(0);
#[cfg(any(test, feature = "contract-tests"))] static L: Mutex<u8> = Mutex::new(0);
'''
        self.assertEqual(found(source), [('static', name) for name in 'EFIJKL'])
        self.assertEqual(found(source, frozenset({'contract-tests'})), [('static', name) for name in 'EFIJK'])

    def test_inner_test_attribute_exempts_the_whole_scope(self):
        self.assertEqual(found('#![cfg(test)]\nstatic A: Mutex<u8> = Mutex::new(0);'), [])
        self.assertEqual(found('mod m { #![cfg(test)] static A: Mutex<u8> = Mutex::new(0); }\n'
                               'static B: Mutex<u8> = Mutex::new(0);'), [('static', 'B')])

    def test_line_numbers(self):
        occurrence = checker.analyze('\n\nfn f() {\n    std::env::var("X");\n}').occurrences[0]
        self.assertEqual(occurrence.line, 4)


def write(root, files):
    for name, text in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')


GLOBAL = 'static G: Mutex<u8> = Mutex::new(0);\n'


class ModuleTree(unittest.TestCase):
    def test_follows_mod_and_path_declarations_and_exempts_test_modules(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, {
                'src/lib.rs': 'mod a;\n#[cfg(test)]\nmod b;\n#[path = "x/c_impl.rs"]\nmod c;\n'
                              'mod d { pub(crate) mod e; }\n',
                'src/a.rs': GLOBAL + 'mod nested;\n',
                'src/a/nested.rs': GLOBAL,
                'src/b.rs': GLOBAL + 'mod inner;\n',
                'src/b/inner.rs': GLOBAL,
                'src/x/c_impl.rs': 'mod sibling;\n',
                'src/x/sibling.rs': GLOBAL,
                'src/d/e.rs': GLOBAL,
                'src/orphan.rs': GLOBAL,
            })
            result = checker.scan(root, ['src/lib.rs'])
            self.assertEqual(sorted(path for path, _, _ in result.occurrences),
                             ['src/a.rs', 'src/a/nested.rs', 'src/d/e.rs', 'src/orphan.rs', 'src/x/sibling.rs'])
            self.assertEqual(result.unreached, ['src/orphan.rs'])


class Ratchet(unittest.TestCase):
    def run_check(self, entries, files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, files)
            allowlist = root / 'allowlist.json'
            allowlist.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': entries}), encoding='utf-8')
            return checker.check(root, allowlist)[0]

    def entry(self, **overrides):
        entry = {'path': 'src/lib.rs', 'kind': 'static', 'name': 'G', 'disposition': 'debt', 'reason': 'r'}
        entry.update(overrides)
        return entry

    def test_listed_state_passes_and_new_state_fails(self):
        self.assertEqual(self.run_check([self.entry()], {'src/lib.rs': GLOBAL}), [])
        errors = self.run_check([self.entry()], {'src/lib.rs': GLOBAL + 'fn f() { std::env::var("X"); }\n'})
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('NEW   env env::var at src/lib.rs:2'), errors)

    def test_removed_state_must_leave_the_list(self):
        errors = self.run_check([self.entry()], {'src/lib.rs': 'fn f() {}\n'})
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('STALE static G'), errors)

    def test_counts_must_match_exactly(self):
        source = 'fn f() { std::env::var("A"); std::env::var("B"); }\n'
        listed = self.entry(kind='env', name='env::var', count=1)
        self.assertTrue(self.run_check([listed], {'src/lib.rs': source})[0].startswith('COUNT'))
        listed['count'] = 2
        self.assertEqual(self.run_check([listed], {'src/lib.rs': source}), [])

    def test_entries_need_a_disposition_and_reason(self):
        errors = self.run_check([self.entry(disposition='later', reason='')], {'src/lib.rs': GLOBAL})
        self.assertEqual(len(errors), 2)

    def test_injected_credential_helper_struct_is_the_listed_spawn(self):
        # Safety P3-31's closure test: the fault injected into the fixture is
        # CredentialHelper::new(url).execute(). Unlisted, it is NEW as
        # `process` `Cred::credential_helper`. Listed as that, a switch from
        # Cred::credential_helper to it keeps the entry matched, not STALE.
        injected = 'fn f() { let found = CredentialHelper::new(url).execute(); }\n'
        errors = self.run_check([], {'src/lib.rs': injected})
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('NEW   process Cred::credential_helper at src/lib.rs:1'), errors)
        listed = self.entry(kind='process', name='Cred::credential_helper')
        before = 'fn f() { let found = git2::Cred::credential_helper(&config, url, None); }\n'
        self.assertEqual(self.run_check([listed], {'src/lib.rs': before}), [])
        self.assertEqual(self.run_check([listed], {'src/lib.rs': injected}), [])


class ReconciledCommit(unittest.TestCase):
    """An allowlist for another repository records the commit it was reconciled against."""

    def allowlist(self, directory, **fields):
        path = Path(directory) / 'allowlist.json'
        path.write_text(json.dumps({'roots': ['src/lib.rs'], 'entries': [], **fields}), encoding='utf-8')
        return path

    def test_only_a_full_lowercase_sha_is_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            loaded, errors = checker.load_allowlist(self.allowlist(directory, reconciled_commit='a' * 40))
            self.assertEqual((loaded.reconciled_commit, errors), ('a' * 40, []))
            loaded, errors = checker.load_allowlist(self.allowlist(directory))
            self.assertEqual((loaded.reconciled_commit, errors), (None, []))
            for bad in ('46e65a9', 'A' * 40, 'g' * 40, 'a' * 41, '', 40):
                loaded, errors = checker.load_allowlist(self.allowlist(directory, reconciled_commit=bad))
                self.assertIsNone(loaded.reconciled_commit, bad)
                self.assertEqual(len(errors), 1, bad)
                self.assertIn('reconciled_commit', errors[0])

    def test_reader_prints_the_commit_or_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            for fields, status, printed in (({'reconciled_commit': 'b' * 40}, 0, 'b' * 40 + '\n'),
                                            ({}, 1, ''),
                                            ({'reconciled_commit': 'b' * 12}, 1, '')):
                stdout, stderr = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    result = checker.main(['--allowlist', str(self.allowlist(directory, **fields)),
                                           '--reconciled-commit'])
                self.assertEqual((result, stdout.getvalue()), (status, printed), fields)
                self.assertEqual(bool(stderr.getvalue()), status != 0, fields)


def job_steps(text, job):
    """The step blocks of one job in a workflow: jobs sit at two spaces, steps at six."""
    lines = text.splitlines()
    start = lines.index(f'  {job}:')
    end = next((i for i in range(start + 1, len(lines)) if re.match(r'  [\w-]+:', lines[i])), len(lines))
    steps = []
    for line in lines[start + 1:end]:
        if line.startswith('      - '):
            steps.append([line])
        elif steps and (line.startswith('        ') or not line.strip()):
            steps[-1].append(line)
    return '\n'.join(lines[start:end]), ['\n'.join(step) for step in steps]


def field(step, key):
    match = re.search(rf'^ +(?:- )?{key}: *(.*)$', step, re.M)
    return match.group(1).strip() if match else None


class TransportPin(unittest.TestCase):
    """B11 of Verdict-3: gwz-core's boundary job checks gwz-transport at the allowlist's commit."""

    def test_reconciled_commit_is_a_full_sha(self):
        loaded, errors = checker.load_allowlist(TRANSPORT_ALLOWLIST)
        self.assertEqual(errors, [])
        self.assertRegex(loaded.reconciled_commit or '', r'^[0-9a-f]{40}$')
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            self.assertEqual(checker.main(['--allowlist', str(TRANSPORT_ALLOWLIST), '--reconciled-commit']), 0)
        self.assertEqual(stdout.getvalue(), loaded.reconciled_commit + '\n')

    def test_boundary_job_checks_out_gwz_transport_at_the_reconciled_commit(self):
        text = BOUNDARY_WORKFLOW.read_text(encoding='utf-8')
        job, steps = job_steps(text, 'boundary')
        checkouts = [step for step in steps if field(step, 'repository') == 'owebeeone/gwz-transport']
        self.assertEqual(len(checkouts), 1, 'one gwz-transport checkout in the boundary job')
        checkout = checkouts[0]
        ref = re.fullmatch(r'\$\{\{ steps\.([\w-]+)\.outputs\.(\w+) \}\}', field(checkout, 'ref') or '')
        self.assertIsNotNone(ref, 'the checkout takes its ref from a step output')
        step_id, output = ref.groups()
        readers = [step for step in steps if field(step, 'id') == step_id]
        self.assertEqual(len(readers), 1)
        reader = readers[0]
        self.assertLess(steps.index(reader), steps.index(checkout))
        self.assertIn('python scripts/checks/check_process_globals.py', reader)
        self.assertIn(TRANSPORT_ALLOWLIST_ARG, reader)
        self.assertIn('--reconciled-commit', reader)
        self.assertIn(f'{output}=', reader)
        self.assertIn('"$GITHUB_OUTPUT"', reader)
        # Beside gwz-core, as a workspace has them, and every command runs in gwz-core.
        self.assertIn('actions/checkout', steps[0])
        self.assertEqual(field(steps[0], 'path'), 'gwz-core')
        self.assertEqual(field(checkout, 'path'), 'gwz-transport')
        self.assertEqual(field(job, 'working-directory'), 'gwz-core')
        # The check runs over that checkout, and the job has no way to skip it.
        runs = [step for step in steps if '--repo ../gwz-transport' in step]
        self.assertEqual(len(runs), 1)
        self.assertLess(steps.index(checkout), steps.index(runs[0]))
        self.assertIn(TRANSPORT_ALLOWLIST_ARG, runs[0])
        self.assertNotIn('--skip-transport-globals', job)
        # The allowlist is the only place the commit is written.
        loaded, _ = checker.load_allowlist(TRANSPORT_ALLOWLIST)
        self.assertNotIn(loaded.reconciled_commit, text)


class Repository(unittest.TestCase):
    def test_core_adds_no_process_global_state(self):
        errors, _, _ = checker.check(checker.ROOT, checker.DEFAULT_ALLOWLIST)
        self.assertEqual(errors, [], '\n'.join(errors))


if __name__ == '__main__':
    unittest.main()
