use std::fs;
use std::path::{Path, PathBuf};

use crate::model::ErrorCode;

use super::*;

#[test]
pub(crate) fn creates_and_detects_ordinary_non_bare_repositories() {
    let temp = TempDir::new("create");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");

    let created = backend.create_repo(&repo_path).unwrap();

    assert_eq!(created.path, repo_path);
    assert!(backend.is_repository(&repo_path).unwrap());
    assert!(!backend.is_repository(&temp.path().join("missing")).unwrap());
    assert!(!git2::Repository::open(&repo_path).unwrap().is_bare());
}

#[test]
pub(crate) fn create_repo_pins_repo_local_filter_neutralization_at_creation() {
    // M5-8 A1 Decision Packet, Decision 1 Option B: gwz-born repositories pin
    // `core.autocrlf=false` + `core.eol=lf` REPO-LOCALLY at creation, before
    // any content exists, so every forward materialization is blob-exact
    // regardless of ambient host filter config. Creation-time-only pin — the
    // windows-matrix run-9 regression proved flipping these keys mid-life
    // manufactures dirt; this test pins the safe (creation-time) placement.
    let temp = TempDir::new("create-filter-pins");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();

    // Read the repo-LOCAL config file only (not the merged stack): the pin
    // must hold without help from any ambient level.
    let local = git2::Config::open(&repo_path.join(".git/config")).unwrap();
    assert!(!local.get_bool("core.autocrlf").unwrap());
    assert_eq!(local.get_string("core.eol").unwrap(), "lf");
}

#[test]
pub(crate) fn created_repo_forward_materialization_is_blob_exact_under_ambient_autocrlf() {
    // Mechanism pair for the creation-time pins. CONTROL: an adopted-style
    // repo (raw git2 init, repo-local autocrlf=true — the ambient Windows
    // host shape) proves the smudge machinery is live in this environment: a
    // default filtered re-materialization writes CRLF. SUBJECT: the identical
    // re-materialization in a gwz-created repo stays blob-exact because the
    // repo-local pins neutralize the filter.
    //
    // SCOPE, precisely ([P3-8] — the name says "under ambient autocrlf" and
    // overstates on a developer host): this test sets NO ambient config. The
    // hostile value is repo-local to the CONTROL, a DIFFERENT repository from
    // the pinned SUBJECT, so what is proven here is that a gwz-born repo is
    // blob-exact while a live smudge source demonstrably exists in the same
    // environment — not that a pin beats a hostile ambient level. The
    // pin-vs-ambient precedence property is exercised only where the RUNNER
    // itself is hostile: the `crlf-sentinel` lane's `git config --global`
    // step, and the windows-2022 image's system-level `core.autocrlf=true`.
    // (Note a `GIT_CONFIG_GLOBAL` fixture could not supply it either —
    // libgit2 reads that variable only under `GIT_REPOSITORY_OPEN_FROM_ENV`,
    // which gwz's plain `Repository::open` does not set.)
    let temp = TempDir::new("create-filter-forward");

    let control = temp.path().join("control");
    let mut opts = git2::RepositoryInitOptions::new();
    opts.bare(false).no_reinit(true).initial_head("main");
    let repository = git2::Repository::init_opts(&control, &opts).unwrap();
    repository
        .config()
        .unwrap()
        .set_bool("core.autocrlf", true)
        .unwrap();
    drop(repository);
    commit_file(&control, "file.txt", "line1\nline2\n", "seed", &[]).unwrap();
    rematerialize_through_filters(&control, "file.txt");
    assert_eq!(
        fs::read(control.join("file.txt")).unwrap(),
        b"line1\r\nline2\r\n",
        "control: the filtered re-materialization must smudge to CRLF"
    );

    let subject = temp.path().join("subject");
    Git2Backend::new().create_repo(&subject).unwrap();
    commit_file(&subject, "file.txt", "line1\nline2\n", "seed", &[]).unwrap();
    rematerialize_through_filters(&subject, "file.txt");
    assert_eq!(
        fs::read(subject.join("file.txt")).unwrap(),
        b"line1\nline2\n",
        "gwz-born worktrees stay blob-exact through forward materialization"
    );
}

/// Delete `relative` and force-checkout HEAD with FILTERS ACTIVE — the
/// ordinary forward-materialization edge (missing files are rewritten through
/// whatever smudge filters the repo config activates).
pub(crate) fn rematerialize_through_filters(repo_path: &Path, relative: &str) {
    fs::remove_file(repo_path.join(relative)).unwrap();
    let repository = git2::Repository::open(repo_path).unwrap();
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repository.checkout_head(Some(&mut checkout)).unwrap();
}

/// Initialize an ADOPTED-style repository: a raw `git2` init carrying
/// `core.autocrlf=true` in its REPO-LOCAL config, set before any content
/// exists — the ordinary shape of a porcelain clone taken on a Windows
/// autocrlf host.
///
/// Repo-local is git's HIGHEST-precedence config level. Using it as the
/// hostile filter host is therefore strictly stronger than an ambient one
/// (`GIT_CONFIG_GLOBAL`, `--global`, system): a birth pin that beats a
/// repo-local `autocrlf=true` beats every ambient level by construction,
/// because the birth pin is itself repo-local and written at creation. It is
/// also hermetic and thread-safe — no process-global environment mutation
/// (edition 2024 makes `set_var` unsafe, and the suite runs tests in
/// parallel), and the real host config is never touched.
pub(crate) fn init_adopted_autocrlf_repo(path: &Path) {
    let mut opts = git2::RepositoryInitOptions::new();
    opts.bare(false).no_reinit(true).initial_head("main");
    let repository = git2::Repository::init_opts(path, &opts).unwrap();
    repository
        .config()
        .unwrap()
        .set_bool("core.autocrlf", true)
        .unwrap();
}

/// Crate-relative paths (forward-slashed) of every `.rs` file under `src/`
/// whose text contains `needle`.
fn source_files_containing(needle: &str) -> Vec<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    let mut stack = vec![source.clone()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            if fs::read_to_string(&path).unwrap().contains(needle) {
                hits.push(
                    path.strip_prefix(&source)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    hits.sort();
    hits
}

/// True when `relative` is test-only source: a `tests` / `interface_tests`
/// path component, or a `tests*.rs` / `*_tests.rs` file name.
///
/// [P3-1]: a third clause once treated ANY file containing `#[cfg(test)]` as
/// test-only. The review measured that as 147 of 681 `src/**.rs` files —
/// production modules that would have escaped the writer assertion below had
/// one of them ever pinned a filter key. It was not load-bearing (every
/// current fixture writer is already test-by-path or test-by-name), so it is
/// deleted rather than narrowed.
fn is_test_source(relative: &str) -> bool {
    let path = Path::new(relative);
    if path.components().any(|component| {
        component.as_os_str() == "tests" || component.as_os_str() == "interface_tests"
    }) {
        return true;
    }
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap();
    stem.starts_with("tests") || stem.ends_with("_tests")
}

/// Slice of `source` running from the first occurrence of `start` to that
/// item's closing brace at column 0 — i.e. one whole free-function body.
///
/// [P2-1] — callers MUST pass LF-normalized text, and this enforces it.
/// `include_str!` hands back WORKING-TREE bytes, which are CRLF on a Windows
/// checkout of this repo (`src/**` carries no `.gitattributes` pin and the
/// windows-2022 image resolves `core.autocrlf=true` at system level; the
/// ledger's run-14 entry records this class biting once already). The earlier
/// version terminated on a literal `"\n}\n"` and, when that needle was absent,
/// fell back to "the rest of the file" — so on CRLF every slice silently
/// widened to the whole file and a genuinely relocated pin PASSED the ordering
/// checks. The guard degraded precisely on the platform it exists to protect.
///
/// Both failure modes are now loud: unnormalized input trips the assertion
/// below, and a missing terminator panics instead of falling back. Normalize
/// at the call site with `.replace("\r\n", "\n")` — the in-tree precedent is
/// `r2d_seam_freeze.rs:219-223`.
fn function_slice<'a>(source: &'a str, start: &str) -> &'a str {
    assert!(
        !source.contains('\r'),
        "creation-time-only guard: `function_slice` requires LF-normalized \
         source; normalize `include_str!` bytes with `.replace(\"\\r\\n\", \"\\n\")`"
    );
    let begin = source
        .find(start)
        .unwrap_or_else(|| panic!("creation-time-only guard: function `{start}` not found"));
    let tail = &source[begin + start.len()..];
    let end = tail
        .match_indices("\n}\n")
        .next()
        .map(|(offset, _)| offset + 3)
        .unwrap_or_else(|| {
            panic!(
                "creation-time-only guard: no column-0 closing brace after `{start}`; \
                 refusing to fall back to the whole file"
            )
        });
    &tail[..end]
}

/// CREATION-TIME ONLY, as an executable structural guard rather than prose.
///
/// The windows-matrix **run 9** regression is the lesson this pins: flipping
/// `core.autocrlf`/`core.eol` on a repository that has ALREADY materialized
/// content through filters turns every smudged worktree file into permanent
/// manufactured dirt against its blob ("the pin is safe only at repo
/// creation, before any filtered materialization",
/// `GwzWindowsMatrix-Classification.md` run-9 entry). Decision 1 Option B is
/// therefore sound only while the pin is unreachable from any mid-life path.
///
/// Prose in the helper's doc comment cannot enforce that; this test can. It
/// fails the moment a third call site appears, the moment either existing
/// call stops being adjacent to its creation primitive, the moment the
/// helper's `pub(super)` containment inside `gitbackend` is widened, or the
/// moment a second production file starts writing `core.autocrlf` on its own.
/// Any of those is a deliberate decision that must re-open Decision 1.
#[test]
fn filter_neutralization_pin_is_reachable_only_from_the_two_creation_time_sites() {
    let helper = "pin_creation_time_filter_neutralization";

    // 1. The complete reference set. `git/tests/g01.rs` is in it because THIS
    //    test names the helper; every other entry is a real call or the
    //    definition. A new entry means a new (possibly mid-life) call site.
    assert_eq!(
        source_files_containing(helper),
        vec![
            "git/gitbackend/repository.rs".to_owned(),
            "git/gitbackend/repository_support.rs".to_owned(),
            "git/gitbackend/transport.rs".to_owned(),
            "git/tests/g01.rs".to_owned(),
        ],
        "creation-time-only guard: the filter-neutralization pin gained or \
         lost a reference. A new call site must be proven to run at repository \
         CREATION, before any filtered materialization (run-9 lesson), or \
         Decision 1 Option B is re-opened."
    );

    // 2. Containment: `pub(super)` keeps the helper inside `git::gitbackend`,
    //    so no module outside the backend can reach it at all.
    //    [P2-1]: every `include_str!` below is LF-normalized before use —
    //    it receives working-tree bytes, which are CRLF on a Windows checkout
    //    (precedent: `r2d_seam_freeze.rs:219-223`).
    let support = include_str!("../gitbackend/repository_support.rs").replace("\r\n", "\n");
    assert!(
        support.contains(&format!("pub(super) fn {helper}(")),
        "creation-time-only guard: the pin helper must stay `pub(super)` \
         (contained inside git::gitbackend)"
    );

    // 3. Call site A is `create_repo`, AFTER the init primitive and inside
    //    that function — i.e. on a repository that cannot yet hold content.
    let repository = include_str!("../gitbackend/repository.rs").replace("\r\n", "\n");
    assert_eq!(
        repository.matches(&format!("{helper}(")).count(),
        1,
        "creation-time-only guard: repository.rs must pin exactly once"
    );
    let create_repo = function_slice(&repository, "fn create_repo(");
    let init = create_repo
        .find("Repository::init_opts(")
        .expect("creation-time-only guard: create_repo must init the repository");
    let pin = create_repo
        .find(&format!("{helper}("))
        .expect("creation-time-only guard: create_repo must pin at creation");
    assert!(
        init < pin,
        "creation-time-only guard: the pin must follow `Repository::init_opts` \
         inside `create_repo`"
    );

    // 4. Call site B is the clone funnel, AFTER the clone that performs the
    //    initial materialization — which itself runs filters-off.
    let transport = include_str!("../gitbackend/transport.rs").replace("\r\n", "\n");
    assert_eq!(
        transport.matches(&format!("{helper}(")).count(),
        1,
        "creation-time-only guard: transport.rs must pin exactly once"
    );
    let funnel = function_slice(&transport, "fn clone_repo_with_progress(");
    assert!(
        funnel.contains("disable_filters(true)"),
        "creation-time-only guard: the clone funnel's initial materialization \
         must run with filters disabled"
    );
    let cloned = funnel
        .find("builder.clone(")
        .expect("creation-time-only guard: the funnel must perform the clone");
    let pin = funnel
        .find(&format!("{helper}("))
        .expect("creation-time-only guard: the funnel must pin after cloning");
    assert!(
        cloned < pin,
        "creation-time-only guard: the pin must follow `builder.clone` inside \
         the clone funnel"
    );

    // 5. No second production writer, for EITHER pinned key. Fixtures pin them
    //    freely (that is test hermeticity); production code writing either one
    //    anywhere but the creation-time helper would be a mid-life pin by
    //    another name. [P3-2]: `core.eol` is checked alongside
    //    `core.autocrlf` — the pin is a PAIR, and a mid-life writer that
    //    flipped only `core.eol` was previously invisible here.
    for key in ["\"core.autocrlf\"", "\"core.eol\""] {
        let production_writers = source_files_containing(key)
            .into_iter()
            .filter(|relative| !is_test_source(relative))
            .collect::<Vec<_>>();
        assert_eq!(
            production_writers,
            vec!["git/gitbackend/repository_support.rs".to_owned()],
            "creation-time-only guard: exactly one production source may write \
             {key}, and only at creation time"
        );
    }
}

/// CRLF class sentinel — the executable answer to the ledger's F6-class
/// "no failing sentinel anywhere" complaint.
///
/// Every other CRLF assertion in this suite is conditional on the smudge
/// machinery actually being live in the environment that runs it. The run-7
/// fixture pins are precisely what made CI structurally blind to the class:
/// with `core.autocrlf=false` everywhere, an assertion that a worktree is
/// blob-exact passes for the wrong reason and nothing ever pages.
///
/// The body asserts the DELIBERATELY FALSE claim that an un-pinned,
/// adopted-style worktree materializes blob-exact, so the assertion fires
/// while the class is real. `#[should_panic]` turns that firing into the
/// GREEN result:
///
/// * **class live (today, every OS)** → assert fires → expected panic → PASS.
/// * **class gone** → assert holds → no panic → libtest fails the test with
///   "did not panic as expected". That is the page: this environment lost its
///   smudge source and every CRLF assertion in the suite has become vacuous,
///   or the raw-byte doctrine changed and the frozen texts must move with it
///   (Decision 1 B; the amendment's Clause A scope limits; the tripwire
///   ledger).
///
/// **Why `should_panic` and not `#[ignore]` + an inverted exit code**
/// (review item 7 / [P2-2]): an ignored test can only be reached by bespoke
/// CI machinery, which is what confined the sentinel to a
/// `workflow_dispatch`-only lane. As a plain `should_panic` test it rides
/// EVERY existing `cargo test` surface — the per-commit lane, `windows-matrix`,
/// `platform-matrix` and `release.yml`'s tagged both-OS run — with no exit
/// inversion, and it cannot be confused with a build failure. The fixture
/// carries its own hostile config repo-locally, so the smudge source is live
/// off Windows too and the test is green everywhere today.
#[test]
#[should_panic(expected = "CRLF-CLASS-SENTINEL: smudge source is LIVE")]
fn crlf_sentinel_unpinned_worktree_materializes_blob_exact() {
    let temp = TempDir::new("crlf-sentinel-unpinned");
    let repo = temp.path().join("adopted");
    init_adopted_autocrlf_repo(&repo);
    commit_file(&repo, "file.txt", "line1\nline2\n", "seed", &[]).unwrap();
    rematerialize_through_filters(&repo, "file.txt");
    assert_eq!(
        fs::read(repo.join("file.txt")).unwrap(),
        b"line1\nline2\n",
        "CRLF-CLASS-SENTINEL: smudge source is LIVE — an un-pinned worktree \
         materialized CRLF, which is the expected state while the class is \
         open. This panic is what makes the test pass. If this test ever fails \
         with 'did not panic as expected', the class is gone: reconcile \
         Decision 1 B, the amendment's Clause A scope limits, and the tripwire \
         ledger before silencing it."
    );
}

#[test]
fn commit_exists_requires_a_local_object_that_peels_to_commit() {
    let temp = TempDir::new("commit-exists");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();
    let commit = commit_file(&repo_path, "tracked.txt", "one", "initial", &[]).unwrap();
    let repository = git2::Repository::open(&repo_path).unwrap();
    let tree = repository
        .find_commit(git2::Oid::from_str(&commit).unwrap())
        .unwrap()
        .tree_id()
        .to_string();

    assert!(backend.commit_exists(&repo_path, &commit).unwrap());
    assert!(!backend.commit_exists(&repo_path, &tree).unwrap());
    assert!(
        !backend
            .commit_exists(&repo_path, "0000000000000000000000000000000000000000")
            .unwrap()
    );
    assert!(!backend.commit_exists(&repo_path, "not-an-oid").unwrap());
}

#[test]
pub(crate) fn stage_paths_matches_porcelain_git_add() {
    // Seed two identical repos; stage one via the primitive and one via
    // porcelain `git add`. The resulting index must be byte-identical —
    // pathspec scoping, recursive add, and `.gitignore` honoring all agree.
    let temp = TempDir::new("stage-parity");
    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    seed_stage_repo(&prim);
    seed_stage_repo(&porc);

    let result = Git2Backend::new()
        .stage_paths(&prim, &["tracked", ".gitignore"])
        .expect("primitive stage");
    // Only ".gitignore" is a top-level file; "tracked" is a directory.
    assert_eq!(result.staged, 1);

    run_git(&porc, &["add", "tracked", ".gitignore"]);

    assert_eq!(
        ls_files_stage(&prim),
        ls_files_stage(&porc),
        "primitive index must match `git add` porcelain (mode+oid+path)"
    );
    // Sanity: the gitignored, out-of-pathspec files are staged by neither.
    assert!(!ls_files_stage(&prim).contains("ignored/"));
    assert!(!ls_files_stage(&prim).contains("loose.txt"));
}

#[test]
pub(crate) fn stage_paths_errors_on_non_repository() {
    let temp = TempDir::new("stage-nonrepo");
    let err = Git2Backend::new()
        .stage_paths(temp.path(), &[".gitignore"])
        .expect_err("staging a non-repository must fail");
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
}

pub(crate) fn seed_stage_repo(root: &Path) {
    fs::create_dir_all(root.join("tracked")).unwrap();
    fs::write(root.join("tracked").join("a.txt"), "a\n").unwrap();
    fs::create_dir_all(root.join("ignored")).unwrap();
    fs::write(root.join("ignored").join("b.txt"), "b\n").unwrap();
    fs::write(root.join(".gitignore"), "/ignored/\n").unwrap();
    fs::write(root.join("loose.txt"), "loose\n").unwrap();
    Git2Backend::new().create_repo(root).unwrap();
}

pub(crate) fn run_git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=GWZ",
            "-c",
            "user.email=gwz@example.invalid",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

pub(crate) fn ls_files_stage(root: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--stage"])
        .output()
        .expect("spawn git ls-files");
    assert!(output.status.success(), "git ls-files failed");
    String::from_utf8(output.stdout).expect("ls-files utf8")
}

#[test]
pub(crate) fn empty_repository_head_reports_unborn_branch_without_commit() {
    let temp = TempDir::new("empty-head");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();

    let head = backend.head(&repo_path).unwrap();

    assert_eq!(head.branch, Some("main".to_owned()));
    assert_eq!(head.commit, None);
    assert!(!head.is_detached);
    assert_eq!(backend.read_ref(&repo_path, "HEAD").unwrap(), None);
}

#[test]
pub(crate) fn reads_and_adds_remotes() {
    let temp = TempDir::new("remotes");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();

    backend
        .add_remote(&repo_path, "origin", "file:///tmp/origin.git")
        .unwrap();

    let remotes = backend.remotes(&repo_path).unwrap();
    assert_eq!(
        remotes,
        vec![GitRemote {
            name: "origin".to_owned(),
            url: Some("file:///tmp/origin.git".to_owned()),
            push_url: None,
        }]
    );
}

#[test]
pub(crate) fn reports_clean_untracked_unstaged_and_staged_status() {
    let temp = TempDir::new("status");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();
    commit_file(&repo_path, "tracked.txt", "one", "initial", &[]).unwrap();

    assert_eq!(backend.status(&repo_path).unwrap(), GitStatus::clean());

    fs::write(repo_path.join("untracked.txt"), "new").unwrap();
    let status = backend.status(&repo_path).unwrap();
    assert!(status.is_dirty);
    assert_eq!(status.untracked, 1);
    fs::remove_file(repo_path.join("untracked.txt")).unwrap();

    fs::write(repo_path.join("tracked.txt"), "two").unwrap();
    let status = backend.status(&repo_path).unwrap();
    assert!(status.is_dirty);
    assert_eq!(status.unstaged, 1);
    assert_eq!(status.staged, 0);

    stage_path(&repo_path, "tracked.txt").unwrap();
    let status = backend.status(&repo_path).unwrap();
    assert!(status.is_dirty);
    assert_eq!(status.staged, 1);
    assert_eq!(status.unstaged, 0);
}

#[test]
pub(crate) fn status_hides_ignored_files_unless_requested() {
    let temp = TempDir::new("status-ignored");
    let backend = Git2Backend::new();
    let repo_path = temp.path().join("repo");
    backend.create_repo(&repo_path).unwrap();
    commit_file(&repo_path, ".gitignore", "ignored/\n", "ignore", &[]).unwrap();
    fs::create_dir_all(repo_path.join("ignored")).unwrap();
    fs::write(repo_path.join("ignored/cache.txt"), "cache").unwrap();

    let default_status = backend.status(&repo_path).unwrap();
    assert_eq!(default_status, GitStatus::clean());

    let ignored_status = backend
        .status_with_options(&repo_path, GitStatusOptions::include_ignored())
        .unwrap();
    assert!(!ignored_status.is_dirty);
    assert_eq!(ignored_status.untracked, 0);
    assert!(ignored_status.files.iter().any(|file| {
        file.path == "ignored/" && file.index_status == " " && file.worktree_status == "!"
    }));
}

#[test]
pub(crate) fn status_detects_a_staged_rename_with_original_path() {
    // F17: rename detection must be ON — a `git mv` should report one `R` entry with
    // the original path, not an unrelated delete + add.
    let temp = TempDir::new("status-rename");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "old.txt", "stable contents\n", "seed", &[]).unwrap();

    run_git(&repo, &["mv", "old.txt", "new.txt"]);

    let status = backend.status(&repo).unwrap();
    assert!(status.is_dirty);
    let rename = status
        .files
        .iter()
        .find(|file| file.path == "new.txt")
        .expect("renamed file present in status");
    assert_eq!(rename.index_status, "R");
    assert_eq!(rename.original_path.as_deref(), Some("old.txt"));
    // The old path is not reported as a separate deletion.
    assert!(status.files.iter().all(|file| file.path != "old.txt"));
}

#[test]
pub(crate) fn clones_local_repo_and_rejects_non_empty_targets_before_mutation() {
    let temp = TempDir::new("clone");
    let backend = Git2Backend::new();
    let source_path = temp.path().join("source");
    backend.create_repo(&source_path).unwrap();
    commit_file(&source_path, "README.md", "hello", "initial", &[]).unwrap();

    let clone_path = temp.path().join("clone");
    backend
        .clone_repo(source_path.to_str().unwrap(), &clone_path)
        .unwrap();
    assert!(backend.is_repository(&clone_path).unwrap());
    assert!(clone_path.join("README.md").is_file());

    let blocked_path = temp.path().join("blocked");
    fs::create_dir_all(&blocked_path).unwrap();
    fs::write(blocked_path.join("keep.txt"), "keep").unwrap();
    let err = backend
        .clone_repo(source_path.to_str().unwrap(), &blocked_path)
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::PathCollision);
    assert!(blocked_path.join("keep.txt").is_file());
    assert!(!blocked_path.join(".git").exists());
}

#[test]
pub(crate) fn clone_funnel_materializes_blob_exact_and_pins_filter_neutralization() {
    // M5-8 A1 Decision Packet, Decision 1 Option B, clone edge: the single
    // production clone funnel (`clone_repo_with_progress`) must (1) run its
    // initial materialization with checkout filters DISABLED so the worktree
    // is blob-exact from birth, and (2) pin `core.autocrlf=false` +
    // `core.eol=lf` repo-locally on the fresh clone before anything else can
    // materialize files. Invariant: no file is ever written through a smudge
    // filter into a gwz-created repository.
    let temp = TempDir::new("clone-filter-pins");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    backend.create_repo(&source).unwrap();
    // An attribute-driven smudge source (`eol=crlf`) is active on EVERY OS,
    // so this test exercises the same class a Windows `autocrlf=true` host
    // would, hermetically. The blob itself stays LF (clean direction).
    let attrs = commit_file(
        &source,
        ".gitattributes",
        "*.txt text eol=crlf\n",
        "attrs",
        &[],
    )
    .unwrap();
    let attrs_oid = git2::Oid::from_str(&attrs).unwrap();
    commit_file(
        &source,
        "file.txt",
        "line1\nline2\n",
        "content",
        &[attrs_oid],
    )
    .unwrap();

    // CONTROL: a porcelain-equivalent clone (filters active) smudges the
    // covered file to CRLF — proof the smudge source is live here, so the
    // funnel assertion below is load-bearing and not vacuous.
    let control = temp.path().join("control");
    git2::build::RepoBuilder::new()
        .clone(source.to_str().unwrap(), &control)
        .unwrap();
    assert_eq!(
        fs::read(control.join("file.txt")).unwrap(),
        b"line1\r\nline2\r\n",
        "control: a filtered clone must smudge the covered file"
    );

    // FUNNEL: blob-exact initial materialization, creation-time pins present
    // in the repo-LOCAL config, and status clean (clean-idempotence on LF).
    let clone = temp.path().join("clone");
    backend
        .clone_repo(source.to_str().unwrap(), &clone)
        .unwrap();
    assert_eq!(
        fs::read(clone.join("file.txt")).unwrap(),
        b"line1\nline2\n",
        "gwz clone funnel must materialize exact blob bytes"
    );
    let local = git2::Config::open(&clone.join(".git/config")).unwrap();
    assert!(!local.get_bool("core.autocrlf").unwrap());
    assert_eq!(local.get_string("core.eol").unwrap(), "lf");
    assert!(!backend.status(&clone).unwrap().is_dirty);
}

#[test]
pub(crate) fn pushes_fetches_fast_forwards_and_checks_out_commits() {
    let temp = TempDir::new("networkless");
    let backend = Git2Backend::new();
    let source_path = temp.path().join("source");
    let bare_path = temp.path().join("remote.git");
    let clone_path = temp.path().join("clone");
    backend.create_repo(&source_path).unwrap();
    init_bare_main(&bare_path);
    backend
        .add_remote(&source_path, "origin", bare_path.to_str().unwrap())
        .unwrap();

    let first = commit_file(&source_path, "README.md", "one", "initial", &[]).unwrap();
    backend
        .push(&source_path, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    backend
        .clone_repo(bare_path.to_str().unwrap(), &clone_path)
        .unwrap();
    let cloned_head = backend.head(&clone_path).unwrap();
    assert_eq!(cloned_head.branch, Some("main".to_owned()));
    assert!(!cloned_head.is_detached);
    assert_eq!(cloned_head.commit, Some(first.clone()));
    assert_eq!(
        backend.read_ref(&clone_path, "HEAD").unwrap(),
        Some(first.clone())
    );

    let parent = git2::Repository::open(&source_path)
        .unwrap()
        .find_commit(git2::Oid::from_str(&first).unwrap())
        .unwrap()
        .id();
    let second = commit_file(&source_path, "dev-docs/new.md", "two", "second", &[parent]).unwrap();
    backend
        .push(&source_path, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    backend.fetch(&clone_path, "origin").unwrap();
    backend
        .fast_forward(&clone_path, "main", "refs/remotes/origin/main")
        .unwrap();
    assert_eq!(backend.head(&clone_path).unwrap().commit, Some(second));
    assert_text_eq(clone_path.join("dev-docs/new.md"), "two");
    assert!(!backend.status(&clone_path).unwrap().is_dirty);

    backend.checkout_commit(&clone_path, &first).unwrap();
    let head = backend.head(&clone_path).unwrap();
    assert!(head.is_detached);
    assert_eq!(head.commit, Some(first));
}

#[test]
pub(crate) fn fast_forward_matches_porcelain_merge_ff_only_and_self_verifies() {
    // main@A behind feature@B (A is an ancestor of B): a clean fast-forward.
    let temp = TempDir::new("ff-parity");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    let b = commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    let result = backend
        .fast_forward(&prim, "main", "refs/heads/feature")
        .unwrap();
    assert!(result.updated);
    assert_eq!(result.commit.as_deref(), Some(b.as_str()));

    run_git(&porc, &["merge", "--ff-only", "feature"]);

    // Byte-identical end state vs porcelain: same HEAD, same tree, clean worktree.
    assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
    assert_eq!(rev_parse(&prim, "HEAD"), b);
    assert_eq!(
        rev_parse(&prim, "HEAD^{tree}"),
        rev_parse(&porc, "HEAD^{tree}")
    );
    assert!(status_porcelain(&prim).trim().is_empty());
    assert_text_eq(prim.join("f.txt"), "b\n");
}

#[test]
pub(crate) fn fast_forward_rejects_divergent_history_without_moving_branch() {
    // main@D and feature@C both descend from A — not fast-forwardable.
    let temp = TempDir::new("ff-diverge");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    commit_file(&base, "f.txt", "c\n", "C", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);
    let d = commit_file(&base, "f.txt", "d\n", "D", &[a_oid]).unwrap();

    let err = backend
        .fast_forward(&base, "main", "refs/heads/feature")
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::DivergedMember);
    // Porcelain agrees it is not fast-forwardable.
    assert!(!run_git_ok(&base, &["merge", "--ff-only", "feature"]));
    // Failed = nothing changed: main is still at D.
    assert_eq!(rev_parse(&base, "HEAD"), d);
}

#[test]
pub(crate) fn checkout_commit_matches_porcelain_and_self_verifies() {
    // Detach onto an older commit A while B is current.
    let temp = TempDir::new("checkout-parity");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    let result = backend.checkout_commit(&prim, &a).unwrap();
    assert_eq!(result.commit.as_deref(), Some(a.as_str()));

    run_git(&porc, &["checkout", &a]);

    assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
    assert_eq!(rev_parse(&prim, "HEAD"), a);
    assert_eq!(
        rev_parse(&prim, "HEAD^{tree}"),
        rev_parse(&porc, "HEAD^{tree}")
    );
    assert!(status_porcelain(&prim).trim().is_empty());
    assert_text_eq(prim.join("f.txt"), "a\n");
    assert!(backend.head(&prim).unwrap().is_detached);
}

#[test]
pub(crate) fn checkout_commit_rejects_unknown_commit_without_moving_head() {
    let temp = TempDir::new("checkout-missing");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let before = rev_parse(&base, "HEAD");

    let bogus = "0".repeat(40);
    let err = backend.checkout_commit(&base, &bogus).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
    assert_eq!(rev_parse(&base, "HEAD"), before);
}

#[test]
pub(crate) fn verify_checkout_state_accepts_match_and_rejects_mismatch() {
    // Direct test of the AD1 self-verify: HEAD is at B.
    let temp = TempDir::new("verify-state");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&repo, "f.txt", "b\n", "B", &[a_oid]).unwrap();
    let b_oid = git2::Oid::from_str(&b).unwrap();

    assert!(verify_checkout_state(&repo, b_oid).is_ok());
    let err = verify_checkout_state(&repo, a_oid).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
}

pub(crate) fn commit_file(
    repo_path: &Path,
    relative_path: &str,
    content: &str,
    message: &str,
    parents: &[git2::Oid],
) -> Result<String, git2::Error> {
    if let Some(parent) = Path::new(relative_path).parent() {
        fs::create_dir_all(repo_path.join(parent)).unwrap();
    }
    fs::write(repo_path.join(relative_path), content).unwrap();
    stage_path(repo_path, relative_path)?;

    let repo = git2::Repository::open(repo_path)?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid")?;
    let parent_commits = parents
        .iter()
        .map(|id| repo.find_commit(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let parent_refs = parent_commits.iter().collect::<Vec<_>>();
    let oid = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )?;
    Ok(oid.to_string())
}

pub(crate) fn stage_path(repo_path: &Path, relative_path: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut index = repo.index()?;
    index.add_path(Path::new(relative_path))?;
    index.write()
}

#[test]
pub(crate) fn merge_upstream_matches_porcelain_merge_on_clean_diverge() {
    // main@D and feature@C diverge from A touching DIFFERENT files → clean 3-way merge.
    let temp = TempDir::new("merge-clean");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    commit_file(&base, "feat.txt", "feature\n", "C", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);
    commit_file(&base, "main.txt", "main\n", "D", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    let result = backend
        .merge_upstream(&prim, "main", "refs/heads/feature")
        .unwrap();
    assert!(result.is_clean());
    let merge_commit = result.commit.clone().unwrap();

    run_git(&porc, &["merge", "--no-edit", "feature"]);

    // Commit OIDs differ (signature/time), but the merged TREE must match porcelain,
    // the worktree is clean, and HEAD is a two-parent merge commit over feature.
    assert_eq!(
        rev_parse(&prim, "HEAD^{tree}"),
        rev_parse(&porc, "HEAD^{tree}")
    );
    assert!(status_porcelain(&prim).trim().is_empty());
    assert_eq!(rev_parse(&prim, "HEAD"), merge_commit);
    assert_eq!(
        rev_parse(&prim, "HEAD^2"),
        rev_parse(&prim, "refs/heads/feature")
    );
    assert_text_eq(prim.join("feat.txt"), "feature\n");
    assert_text_eq(prim.join("main.txt"), "main\n");
}

#[test]
pub(crate) fn merge_upstream_leaves_conflict_in_place_like_porcelain() {
    // main@D and feature@C both rewrite f.txt → a real merge conflict.
    let temp = TempDir::new("merge-conflict");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    commit_file(&base, "f.txt", "feature\n", "C", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);
    let d = commit_file(&base, "f.txt", "main\n", "D", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    let result = backend
        .merge_upstream(&prim, "main", "refs/heads/feature")
        .unwrap();
    // A conflict is reported, not errored: the path is named, HEAD has not moved.
    assert!(!result.is_clean());
    assert_eq!(result.conflicts, vec!["f.txt".to_owned()]);
    assert!(result.commit.is_none());
    assert_eq!(rev_parse(&prim, "HEAD"), d);
    // Faithful to porcelain: worktree is left mid-merge and `git merge --continue`-able.
    assert!(prim.join(".git/MERGE_HEAD").exists());
    assert!(!run_git_ok(&porc, &["merge", "--no-edit", "feature"]));
    assert_eq!(
        status_porcelain(&prim).trim(),
        status_porcelain(&porc).trim()
    );
}

#[test]
pub(crate) fn rebase_onto_matches_porcelain_rebase_on_clean_diverge() {
    // main@D and feature@C diverge from A touching DIFFERENT files → clean replay.
    let temp = TempDir::new("rebase-clean");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    let c = commit_file(&base, "feat.txt", "feature\n", "C", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);
    commit_file(&base, "main.txt", "main\n", "D", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    let result = backend
        .rebase_onto(&prim, "main", "refs/heads/feature")
        .unwrap();
    assert!(result.is_clean());

    run_git(&porc, &["rebase", "feature"]);

    // Linear history replayed onto feature: same tree as porcelain, clean worktree,
    // HEAD reattached to main with the feature tip as its single parent.
    assert_eq!(
        rev_parse(&prim, "HEAD^{tree}"),
        rev_parse(&porc, "HEAD^{tree}")
    );
    assert!(status_porcelain(&prim).trim().is_empty());
    assert_eq!(rev_parse(&prim, "HEAD^"), c);
    assert_eq!(rev_parse(&prim, "HEAD"), result.commit.unwrap());
    let head = backend.head(&prim).unwrap();
    assert!(!head.is_detached);
    assert_eq!(head.branch.as_deref(), Some("main"));
}

#[test]
pub(crate) fn rebase_onto_leaves_conflict_in_place_like_porcelain() {
    // main@D and feature@C both rewrite f.txt → the replay conflicts.
    let temp = TempDir::new("rebase-conflict");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    commit_file(&base, "f.txt", "feature\n", "C", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);
    commit_file(&base, "f.txt", "main\n", "D", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    let result = backend
        .rebase_onto(&prim, "main", "refs/heads/feature")
        .unwrap();
    assert!(!result.is_clean());
    assert_eq!(result.conflicts, vec!["f.txt".to_owned()]);
    assert!(result.commit.is_none());
    // Faithful to porcelain: the rebase is left in progress, `git rebase --continue`-able.
    assert!(prim.join(".git/rebase-merge").exists());
    assert!(!run_git_ok(&porc, &["rebase", "feature"]));
    assert!(porc.join(".git/rebase-merge").exists());
}

#[test]
pub(crate) fn reset_hard_matches_porcelain_and_discards_local() {
    // main@D diverged from feature@C; reset --hard snaps main onto C, discarding D
    // AND any uncommitted changes.
    let temp = TempDir::new("reset-hard");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    run_git(&base, &["branch", "feature"]);
    run_git(&base, &["checkout", "feature"]);
    let c = commit_file(&base, "f.txt", "feature\n", "C", &[a_oid]).unwrap();
    run_git(&base, &["checkout", "main"]);
    commit_file(&base, "f.txt", "main\n", "D", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);
    // Dirty the primary worktree: reset --hard must discard this too.
    fs::write(prim.join("f.txt"), "uncommitted\n").unwrap();
    assert!(backend.status(&prim).unwrap().is_dirty);

    let result = backend
        .reset_hard(&prim, "main", "refs/heads/feature")
        .unwrap();
    assert!(result.updated);
    assert_eq!(result.commit.as_deref(), Some(c.as_str()));

    run_git(&porc, &["reset", "--hard", "feature"]);

    // Byte-identical end state vs porcelain: same HEAD at feature, same tree, clean.
    assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
    assert_eq!(rev_parse(&prim, "HEAD"), c);
    assert_eq!(
        rev_parse(&prim, "HEAD^{tree}"),
        rev_parse(&porc, "HEAD^{tree}")
    );
    assert!(status_porcelain(&prim).trim().is_empty());
    assert_text_eq(prim.join("f.txt"), "feature\n");
    let head = backend.head(&prim).unwrap();
    assert!(!head.is_detached);
    assert_eq!(head.branch.as_deref(), Some("main"));
}

#[test]
pub(crate) fn checkout_branch_matches_porcelain_and_refuses_diverged_reset() {
    let temp = TempDir::new("checkout-branch");
    let backend = Git2Backend::new();
    let base = temp.path().join("base");
    backend.create_repo(&base).unwrap();
    let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();

    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    copy_repo(&base, &prim);
    copy_repo(&base, &porc);

    // Create `feature` at the older commit A and check out onto it.
    let result = backend.checkout_branch(&prim, "feature", &a).unwrap();
    assert_eq!(result.commit.as_deref(), Some(a.as_str()));
    run_git(&porc, &["checkout", "-b", "feature", &a]);

    // Byte-identical end state vs porcelain: on `feature` at A, clean.
    assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
    assert_eq!(rev_parse(&prim, "HEAD"), a);
    assert_eq!(
        rev_parse(&prim, "HEAD^{tree}"),
        rev_parse(&porc, "HEAD^{tree}")
    );
    assert!(status_porcelain(&prim).trim().is_empty());
    let head = backend.head(&prim).unwrap();
    assert!(!head.is_detached);
    assert_eq!(head.branch.as_deref(), Some("feature"));
    // `main` is untouched at B — never silently reset.
    assert_eq!(rev_parse(&prim, "refs/heads/main"), b);

    // Refuse to move `main` (at B) back to A — that would orphan B.
    let err = backend.checkout_branch(&prim, "main", &a).unwrap_err();
    assert_eq!(err.code, ErrorCode::DivergedMember);
    assert_eq!(rev_parse(&prim, "refs/heads/main"), b);
}

pub(crate) fn init_bare_main(path: &Path) {
    let repo = git2::Repository::init_bare(path).unwrap();
    repo.set_head("refs/heads/main").unwrap();
}

#[test]
pub(crate) fn ls_remote_lists_advertised_refs_matching_porcelain() {
    let temp = TempDir::new("ls-remote");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    let bare = temp.path().join("remote.git");
    backend.create_repo(&source).unwrap();
    init_bare_main(&bare);
    backend
        .add_remote(&source, "origin", bare.to_str().unwrap())
        .unwrap();
    let first = commit_file(&source, "README.md", "one", "initial", &[]).unwrap();
    backend
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    run_git(&source, &["tag", "v1"]);
    backend
        .push(&source, "origin", "refs/tags/v1:refs/tags/v1")
        .unwrap();

    // Non-mutating: capture local refs, call ls_remote, confirm unchanged.
    let refs_before = all_local_refs(&source);
    let refs = backend.ls_remote(&source, "origin").unwrap();
    assert_eq!(
        all_local_refs(&source),
        refs_before,
        "ls_remote must not mutate local refs"
    );

    let mut got = refs
        .iter()
        .map(|r| format!("{} {}", r.target, r.name))
        .collect::<Vec<_>>();
    got.sort();
    // Same advertised ref set as porcelain `git ls-remote` (oid + name).
    assert_eq!(got, ls_remote_porcelain(&source, "origin"));
    // Sanity: main resolves to the pushed commit.
    assert!(
        refs.iter()
            .any(|r| r.name == "refs/heads/main" && r.target == first)
    );
}

#[test]
pub(crate) fn ls_remote_rejects_missing_remote() {
    let temp = TempDir::new("ls-remote-missing");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    backend.create_repo(&source).unwrap();
    let err = backend.ls_remote(&source, "origin").unwrap_err();
    assert_eq!(err.code, ErrorCode::MissingRemote);
}

pub(crate) fn ls_remote_porcelain(repo: &Path, remote: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-remote", remote])
        .output()
        .expect("spawn git ls-remote");
    assert!(output.status.success(), "git ls-remote failed");
    let mut lines = String::from_utf8(output.stdout)
        .expect("ls-remote utf8")
        .lines()
        .map(|line| {
            let mut parts = line.split('\t');
            let oid = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default();
            format!("{oid} {name}")
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines
}

pub(crate) fn all_local_refs(repo: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["for-each-ref", "--format=%(objectname) %(refname)"])
        .output()
        .expect("spawn git for-each-ref");
    assert!(output.status.success(), "git for-each-ref failed");
    let mut lines = String::from_utf8(output.stdout)
        .expect("for-each-ref utf8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines.sort();
    lines.join("\n")
}

pub(crate) fn copy_repo(src: &Path, dst: &Path) {
    if dst.exists() {
        fs::remove_dir_all(dst).unwrap();
    }
    copy_dir_all(src, dst);
}

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else if file_type.is_file() {
            fs::copy(entry.path(), target).unwrap();
        } else if file_type.is_symlink() {
            panic!("copy_repo does not support symlinks: {:?}", entry.path());
        }
    }
}

pub(crate) fn assert_text_eq(path: impl AsRef<Path>, expected: &str) {
    assert_eq!(read_text_normalized(path), expected);
}

pub(crate) fn read_text_normalized(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .unwrap()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub(crate) fn rev_parse(repo: &Path, rev: &str) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", rev])
        .output()
        .expect("spawn git rev-parse");
    assert!(output.status.success(), "git rev-parse {rev} failed");
    String::from_utf8(output.stdout)
        .expect("rev-parse utf8")
        .trim()
        .to_owned()
}

pub(crate) fn status_porcelain(repo: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain"])
        .output()
        .expect("spawn git status");
    assert!(output.status.success(), "git status failed");
    String::from_utf8(output.stdout).expect("status utf8")
}

pub(crate) fn run_git_ok(root: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args([
            "-c",
            "user.name=GWZ",
            "-c",
            "user.email=gwz@example.invalid",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("spawn git")
        .success()
}

pub(crate) struct TempDir {
    pub(crate) path: PathBuf,
}
