//! D4 render acceptance tests.
//!
//! Golden-style parity against the real `git` binary where the spike proved
//! parity is expected (text headers, `--name-status`, `--numstat`, `-z`
//! framing, `--shortstat`), and a `git apply` round-trip where it is not
//! (binary literals — spike Q3). A rename case proves the extended headers carry
//! **both** rewritten paths + similarity and never degrade to add/delete.
//!
//! Every repo is built at a member path `gwz-core/…` inside a temp dir so the
//! member-prefix rewriting is exercised end to end.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::Repository;

use crate::diff::render::{
    PrefixPolicy, RenderEntry, RenderOptions, ScopeRender, render_entry, render_name_only,
    render_name_status, render_numstat, render_shortstat, render_stat, render_summary,
};
use crate::diff::{
    ComparisonSpec, RepoDiffComparison, RepoDiffManifest, RepoDiffOptions, diff_repo,
    resolve_comparison,
};

const MEMBER: &str = "gwz-core";

// ---------------------------------------------------------------------------
// Fixture: a self-cleaning temp repo + git-CLI golden helpers.
// ---------------------------------------------------------------------------

struct TempRepo {
    dir: PathBuf,
    repo: Repository,
}

impl TempRepo {
    fn new(tag: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gwz-core-render-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let repo = Repository::init(&dir).unwrap();
        run_git(&dir, &["config", "user.name", "GWZ"]);
        run_git(&dir, &["config", "user.email", "gwz@example.invalid"]);
        run_git(&dir, &["config", "core.autocrlf", "false"]);
        run_git(&dir, &["config", "diff.renames", "true"]);
        TempRepo { dir, repo }
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn write(&self, rel: &str, contents: &[u8]) {
        let full = self.dir.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, contents).unwrap();
    }

    fn remove(&self, rel: &str) {
        fs::remove_file(self.dir.join(rel)).unwrap();
    }

    fn commit(&self, message: &str) {
        run_git(self.path(), &["add", "-A"]);
        run_git(self.path(), &["commit", "-m", message]);
    }

    fn stage(&self) {
        run_git(self.path(), &["add", "-A"]);
    }

    /// The cached (index-vs-HEAD) manifest with rename detection on.
    fn cached_manifest(&self) -> (RepoDiffComparison, RepoDiffOptions, RepoDiffManifest) {
        let spec = ComparisonSpec {
            kind: crate::diff::RepoDiffComparisonKind::IndexVsTree,
            ..Default::default()
        };
        let comparison = resolve_comparison(&self.repo, &spec).expect("resolve");
        let mut options = RepoDiffOptions::full_repo();
        options.find_renames = true;
        let manifest = diff_repo(&self.repo, &comparison, &options).expect("diff");
        (comparison, options, manifest)
    }

    /// The worktree-vs-index manifest (`git diff`).
    fn worktree_manifest(&self) -> (RepoDiffComparison, RepoDiffOptions, RepoDiffManifest) {
        let comparison =
            resolve_comparison(&self.repo, &ComparisonSpec::default()).expect("resolve");
        let options = RepoDiffOptions::full_repo();
        let manifest = diff_repo(&self.repo, &comparison, &options).expect("diff");
        (comparison, options, manifest)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = git_command(dir, args).status().expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn git_stdout(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = git_command(dir, args).output().expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd
}

fn member_scope() -> ScopeRender {
    ScopeRender::member(MEMBER, PrefixPolicy::Default)
}

fn member_opts() -> RenderOptions {
    RenderOptions::member(MEMBER)
}

/// Build the `RenderEntry` list for a manifest under one scope.
fn render_entries<'a>(
    manifest: &'a RepoDiffManifest,
    scope: &'a ScopeRender,
) -> Vec<RenderEntry<'a>> {
    manifest
        .entries
        .iter()
        .map(|e| RenderEntry::new(e, scope))
        .collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Concatenate the rendered patch for every entry in a manifest.
fn render_all_patches(
    repo: &Repository,
    comparison: &RepoDiffComparison,
    options: &RepoDiffOptions,
    manifest: &RepoDiffManifest,
    render: &RenderOptions,
) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in &manifest.entries {
        out.extend_from_slice(&render_entry(repo, comparison, options, entry, render).unwrap());
    }
    out
}

// ===========================================================================
// Patch header parity (spike Q1a, Q2b, Q2c): diff --git / --- / +++ / mode /
// new/deleted headers come out member-prefixed straight from libgit2.
// ===========================================================================

#[test]
fn modify_patch_headers_are_workspace_relative() {
    let r = TempRepo::new("modify");
    r.write("src/lib.rs", b"line1\nline2\nline3\n");
    r.commit("init");
    r.write("src/lib.rs", b"line1\nline2 changed\nline3\n");

    let (comparison, options, manifest) = r.worktree_manifest();
    let patch = text(&render_all_patches(
        &r.repo,
        &comparison,
        &options,
        &manifest,
        &member_opts(),
    ));

    assert!(patch.contains(&format!(
        "diff --git a/{MEMBER}/src/lib.rs b/{MEMBER}/src/lib.rs"
    )));
    assert!(patch.contains(&format!("--- a/{MEMBER}/src/lib.rs")));
    assert!(patch.contains(&format!("+++ b/{MEMBER}/src/lib.rs")));
    // Body hunk is forwarded intact.
    assert!(patch.contains("-line2\n"));
    assert!(patch.contains("+line2 changed\n"));

    // Golden: same three header lines as git with equivalent custom prefixes.
    let golden = text(&git_stdout(
        r.path(),
        &[
            "diff",
            &format!("--src-prefix=a/{MEMBER}/"),
            &format!("--dst-prefix=b/{MEMBER}/"),
            "--",
            "src/lib.rs",
        ],
    ));
    for needle in [
        format!("diff --git a/{MEMBER}/src/lib.rs b/{MEMBER}/src/lib.rs"),
        format!("--- a/{MEMBER}/src/lib.rs"),
        format!("+++ b/{MEMBER}/src/lib.rs"),
    ] {
        assert!(golden.contains(&needle), "git golden missing {needle}");
    }
}

#[test]
fn new_and_deleted_file_headers_are_workspace_relative() {
    let r = TempRepo::new("newdel");
    r.write("keep.txt", b"seed\n");
    r.commit("init");
    r.write("fresh.txt", b"fresh\n");
    r.remove("keep.txt");
    r.stage();

    let (comparison, options, manifest) = r.cached_manifest();
    let patch = text(&render_all_patches(
        &r.repo,
        &comparison,
        &options,
        &manifest,
        &member_opts(),
    ));

    assert!(patch.contains("new file mode 100644"));
    assert!(patch.contains(&format!("+++ b/{MEMBER}/fresh.txt")));
    assert!(patch.contains("--- /dev/null"));
    assert!(patch.contains("deleted file mode 100644"));
    assert!(patch.contains(&format!("--- a/{MEMBER}/keep.txt")));
    assert!(patch.contains("+++ /dev/null"));
}

#[cfg(unix)]
#[test]
fn mode_change_headers_are_path_free_and_prefixed() {
    let r = TempRepo::new("mode");
    r.write("run.sh", b"echo hi\n");
    r.commit("init");
    use std::os::unix::fs::PermissionsExt;
    let full = r.path().join("run.sh");
    let mut perms = fs::metadata(&full).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&full, perms).unwrap();
    r.stage();

    let (comparison, options, manifest) = r.cached_manifest();
    let patch = text(&render_all_patches(
        &r.repo,
        &comparison,
        &options,
        &manifest,
        &member_opts(),
    ));
    assert!(patch.contains(&format!("diff --git a/{MEMBER}/run.sh b/{MEMBER}/run.sh")));
    assert!(patch.contains("old mode 100644"));
    assert!(patch.contains("new mode 100755"));
}

// ===========================================================================
// Rename patch (spike Q1b/Q1c): extended headers carry BOTH rewritten paths +
// similarity, and never degrade to add/delete.
// ===========================================================================

#[test]
fn rename_extended_headers_carry_both_workspace_paths_and_similarity() {
    let r = TempRepo::new("rename");
    r.write("old.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon\n");
    r.commit("init");
    r.remove("old.txt");
    r.write("new.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n");
    r.stage();

    let (comparison, options, manifest) = r.cached_manifest();

    // The manifest itself must classify this as a single rename entry, not an
    // add + a delete (guards against similarity/pairing regressions upstream).
    assert_eq!(manifest.entries.len(), 1, "expected one rename entry");
    let entry = &manifest.entries[0];
    assert!(matches!(entry.status, crate::diff::RepoDiffStatus::Renamed));
    assert_eq!(entry.old_path.as_deref(), Some("old.txt"));
    assert_eq!(entry.new_path.as_deref(), Some("new.txt"));
    let sim = entry.similarity.expect("rename similarity present");
    assert!(
        (50..=100).contains(&sim),
        "similarity in rename range: {sim}"
    );

    let patch = text(&render_entry(&r.repo, &comparison, &options, entry, &member_opts()).unwrap());

    // BOTH extended-header path lines are workspace-relative (the Q1c fix).
    assert!(
        patch.contains(&format!("rename from {MEMBER}/old.txt")),
        "rename-from not workspace-relative:\n{patch}"
    );
    assert!(
        patch.contains(&format!("rename to {MEMBER}/new.txt")),
        "rename-to not workspace-relative:\n{patch}"
    );
    // similarity index header present and forwarded verbatim from libgit2.
    assert!(
        patch.contains(&format!("similarity index {sim}%")),
        "similarity index header must match manifest value {sim}:\n{patch}"
    );
    // The already-correct positions are untouched.
    assert!(patch.contains(&format!("diff --git a/{MEMBER}/old.txt b/{MEMBER}/new.txt")));
    assert!(patch.contains(&format!("--- a/{MEMBER}/old.txt")));
    assert!(patch.contains(&format!("+++ b/{MEMBER}/new.txt")));
    // It did NOT degrade to an add/delete pair.
    assert!(
        !patch.contains("new file mode"),
        "must not degrade to add:\n{patch}"
    );
    assert!(
        !patch.contains("deleted file mode"),
        "must not degrade to delete:\n{patch}"
    );
    // Non-prefixed (bare) rename headers must not leak through.
    assert!(!patch.contains("rename from old.txt\n"));
    assert!(!patch.contains("rename to new.txt\n"));
}

// ===========================================================================
// Root scope: no member prefix; rename headers stay bare (git default).
// ===========================================================================

#[test]
fn root_scope_uses_bare_git_default_paths() {
    let r = TempRepo::new("root");
    r.write("Cargo.toml", b"[package]\nname = \"x\"\n");
    r.commit("init");
    r.write("Cargo.toml", b"[package]\nname = \"y\"\n");

    let (comparison, options, manifest) = r.worktree_manifest();
    let render = RenderOptions::root();
    let patch = text(&render_all_patches(
        &r.repo,
        &comparison,
        &options,
        &manifest,
        &render,
    ));
    assert!(patch.contains("diff --git a/Cargo.toml b/Cargo.toml"));
    assert!(patch.contains("--- a/Cargo.toml"));
    assert!(patch.contains("+++ b/Cargo.toml"));
    assert!(!patch.contains(&format!("{MEMBER}/")));
}

// ===========================================================================
// line-prefix (spike Q4a): applied per physical line, including the multi-line
// header block.
// ===========================================================================

#[test]
fn line_prefix_is_applied_to_every_physical_line() {
    let r = TempRepo::new("lineprefix");
    r.write("file.txt", b"a\nb\nc\n");
    r.commit("init");
    r.write("file.txt", b"a\nB\nc\n");

    let (comparison, options, manifest) = r.worktree_manifest();
    let mut render = member_opts();
    render.line_prefix = Some("> ".to_owned());
    let patch = text(&render_all_patches(
        &r.repo,
        &comparison,
        &options,
        &manifest,
        &render,
    ));

    for line in patch.lines() {
        assert!(
            line.is_empty() || line.starts_with("> "),
            "every non-empty line must carry the prefix: {line:?}"
        );
    }
    // Header lines within the single 'F' block are each prefixed.
    assert!(patch.contains(&format!(
        "> diff --git a/{MEMBER}/file.txt b/{MEMBER}/file.txt"
    )));
    assert!(patch.contains(&format!("> --- a/{MEMBER}/file.txt")));
    assert!(patch.contains(&format!("> +++ b/{MEMBER}/file.txt")));
    // Hunk body lines are prefixed too.
    assert!(patch.contains("> -b\n"));
    assert!(patch.contains("> +B\n"));
}

// ===========================================================================
// Custom / no prefix (spike Q1a composition): --src/--dst-prefix and
// --no-prefix keep the unified member path.
// ===========================================================================

#[test]
fn custom_and_no_prefix_keep_member_path() {
    let r = TempRepo::new("prefix");
    r.write("f.txt", b"one\n");
    r.commit("init");
    r.write("f.txt", b"two\n");

    let (comparison, options, manifest) = r.worktree_manifest();
    let entry = &manifest.entries[0];

    // Custom prefixes.
    let custom = RenderOptions {
        scope: ScopeRender::member(
            MEMBER,
            PrefixPolicy::Custom {
                src: "x/".to_owned(),
                dst: "y/".to_owned(),
            },
        ),
        ..Default::default()
    };
    let patch = text(&render_entry(&r.repo, &comparison, &options, entry, &custom).unwrap());
    assert!(
        patch.contains(&format!("diff --git x/{MEMBER}/f.txt y/{MEMBER}/f.txt")),
        "custom prefix not composed with member:\n{patch}"
    );
    assert!(patch.contains(&format!("--- x/{MEMBER}/f.txt")));
    assert!(patch.contains(&format!("+++ y/{MEMBER}/f.txt")));

    // No prefix: keep the member path, drop a/ b/.
    let noprefix = RenderOptions {
        scope: ScopeRender::member(MEMBER, PrefixPolicy::None),
        ..Default::default()
    };
    let patch = text(&render_entry(&r.repo, &comparison, &options, entry, &noprefix).unwrap());
    assert!(
        patch.contains(&format!("diff --git {MEMBER}/f.txt {MEMBER}/f.txt")),
        "no-prefix must keep member path:\n{patch}"
    );
    assert!(patch.contains(&format!("--- {MEMBER}/f.txt")));
    assert!(patch.contains(&format!("+++ {MEMBER}/f.txt")));
}

// ===========================================================================
// Binary (spike Q3): byte-correct emission proven by git-apply round-trip, not
// by matching git's compressed stream.
// ===========================================================================

#[test]
fn binary_placeholder_is_prefixed() {
    let r = TempRepo::new("binph");
    r.write("blob.bin", &[0u8, 1, 2, 0, 255, 254, 0, 128]);
    r.commit("init");
    r.write("blob.bin", &[0u8, 9, 8, 0, 100, 200, 0, 7, 7]);
    r.stage();

    let (comparison, options, manifest) = r.cached_manifest();
    let entry = &manifest.entries[0];
    assert!(entry.is_binary);
    let patch = text(&render_entry(&r.repo, &comparison, &options, entry, &member_opts()).unwrap());
    assert!(patch.contains(&format!(
        "Binary files a/{MEMBER}/blob.bin and b/{MEMBER}/blob.bin differ"
    )));
}

#[test]
fn binary_literal_round_trips_through_git_apply() {
    let r = TempRepo::new("binlit");
    let before: Vec<u8> = (0u8..64).map(|b| b.wrapping_mul(7)).collect();
    let after: Vec<u8> = (0u8..80)
        .map(|b| b.wrapping_mul(11).wrapping_add(3))
        .collect();
    r.write("data.bin", &before);
    r.commit("init");
    r.write("data.bin", &after);
    r.stage();

    let (comparison, options, manifest) = r.cached_manifest();
    let entry = &manifest.entries[0];

    // Render at ROOT scope with default a/,b/ so `git apply` strips cleanly.
    let mut render = RenderOptions::root();
    render.show_binary = true;
    let patch = render_entry(&r.repo, &comparison, &options, entry, &render).unwrap();
    assert!(text(&patch).contains("GIT binary patch"));

    // Round-trip: clone at HEAD (== before), apply the rendered patch, expect
    // exactly `after`.
    let clone_dir = r.path().join("apply-check");
    let head = r
        .repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();
    git_stdout(
        clone_dir.parent().unwrap(),
        &[
            "clone",
            "-q",
            r.path().to_str().unwrap(),
            clone_dir.to_str().unwrap(),
        ],
    );
    run_git(&clone_dir, &["checkout", "-q", &head]);

    let mut child = Command::new("git")
        .arg("-C")
        .arg(&clone_dir)
        .args(["apply", "--binary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child.stdin.as_mut().unwrap().write_all(&patch).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let applied = fs::read(clone_dir.join("data.bin")).unwrap();
    assert_eq!(
        applied, after,
        "binary patch did not reconstruct exact bytes"
    );
}

// ===========================================================================
// name-status / name-only / numstat, built from the manifest (spike Q4b), with
// golden parity against real git (paths workspace-relative, rename pairing).
// ===========================================================================

#[test]
fn name_status_matches_git_with_member_prefix() {
    let r = TempRepo::new("namestatus");
    r.write("old.txt", b"aaa\nbbb\nccc\nddd\neee\n");
    r.write("mod.txt", b"1\n2\n3\n");
    r.commit("init");
    r.remove("old.txt");
    r.write("new.txt", b"aaa\nbbb\nccc\nddd\neee\nfff\n");
    r.write("mod.txt", b"1\n2\n3\n4\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let rendered = text(&render_name_status(&entries, &member_opts()));

    // Prefix every git path with the member to form the expected golden. The
    // rename similarity field (`R090` vs git's `R083`) legitimately diverges —
    // libgit2 and git use different similarity heuristics (spike Q2a) — so both
    // sides drop the `R<sim>` digits to `R` before comparing. Rename *pairing*
    // and path rewriting are the contract here, not the percentage.
    let golden_raw = text(&git_stdout(
        r.path(),
        &["diff", "--cached", "-M", "--name-status"],
    ));
    let golden: String = golden_raw
        .lines()
        .map(|l| normalize_rename_sim(&prefix_name_status_line(l)))
        .collect::<Vec<_>>()
        .join("\n");
    let golden = format!("{golden}\n");

    let normalized: String = rendered
        .lines()
        .map(normalize_rename_sim)
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = format!("{normalized}\n");

    assert_eq!(normalized, golden, "name-status mismatch");
    // Explicit: rename line carries BOTH member-prefixed paths, and the status
    // field is a valid `R<sim>` percentage forwarded from libgit2.
    assert!(rendered.contains(&format!("{MEMBER}/old.txt\t{MEMBER}/new.txt")));
    let rename_line = rendered
        .lines()
        .find(|l| l.starts_with('R'))
        .expect("rename line present");
    let sim: u16 = rename_line[1..4].parse().expect("R<nnn> similarity");
    assert!(sim <= 100);
}

#[test]
fn name_only_lists_new_side_workspace_relative() {
    let r = TempRepo::new("nameonly");
    r.write("old.txt", b"aaa\nbbb\nccc\nddd\neee\n");
    r.commit("init");
    r.remove("old.txt");
    r.write("new.txt", b"aaa\nbbb\nccc\nddd\neee\nfff\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let rendered = text(&render_name_only(&entries, &member_opts()));
    assert_eq!(rendered, format!("{MEMBER}/new.txt\n"));
}

#[test]
fn numstat_matches_git_with_member_prefix() {
    let r = TempRepo::new("numstat");
    r.write("keep.txt", b"1\n2\n3\n");
    r.commit("init");
    r.write("keep.txt", b"1\n2\n3\n4\n5\n");
    r.write("added.txt", b"x\ny\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let rendered = text(&render_numstat(&entries, &member_opts()));

    let golden_raw = text(&git_stdout(r.path(), &["diff", "--cached", "--numstat"]));
    // git orders files alphabetically the same way libgit2 does here.
    let golden: String = golden_raw
        .lines()
        .map(prefix_numstat_line)
        .collect::<Vec<_>>()
        .join("\n");
    let golden = format!("{golden}\n");
    assert_eq!(rendered, golden, "numstat mismatch");
}

// ===========================================================================
// -z framing (spike Q4b): NUL-separated name-status, rename as three NUL fields.
// ===========================================================================

#[test]
fn z_name_status_frames_rename_as_three_nul_fields() {
    let r = TempRepo::new("znamestatus");
    r.write("old.txt", b"aaa\nbbb\nccc\nddd\neee\n");
    r.write("mod.txt", b"1\n2\n3\n");
    r.commit("init");
    r.remove("old.txt");
    r.write("new.txt", b"aaa\nbbb\nccc\nddd\neee\nfff\n");
    r.write("mod.txt", b"1\n2\n3\n4\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let mut render = member_opts();
    render.null_terminated = true;
    let rendered = render_name_status(&entries, &render);

    assert!(rendered.contains(&0u8), "-z output must be NUL-framed");
    assert!(
        !rendered.contains(&b'\n'),
        "-z output must not use newlines"
    );

    // git's -z --name-status frames a rename as R<sim>\0old\0new\0.
    let golden = git_stdout(r.path(), &["diff", "--cached", "-M", "-z", "--name-status"]);
    // Both must carry the rename pair; ours member-prefixed.
    let rtext = String::from_utf8_lossy(&rendered);
    assert!(rtext.contains(&format!("{MEMBER}/old.txt")));
    assert!(rtext.contains(&format!("{MEMBER}/new.txt")));
    // The record shape (field count / NUL count) matches git's for the rename.
    let our_nuls = rendered.iter().filter(|&&b| b == 0).count();
    let git_nuls = golden.iter().filter(|&&b| b == 0).count();
    assert_eq!(our_nuls, git_nuls, "NUL field count must match git framing");
}

// ===========================================================================
// shortstat (exact arithmetic parity) + stat/summary (structural).
// ===========================================================================

#[test]
fn shortstat_matches_git_exactly() {
    let r = TempRepo::new("shortstat");
    r.write("a.txt", b"1\n2\n3\n");
    r.write("b.txt", b"x\n");
    r.commit("init");
    r.write("a.txt", b"1\n2\n3\n4\n5\n");
    r.remove("b.txt");
    r.write("c.txt", b"new\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let rendered = text(&render_shortstat(&entries, &member_opts()));

    let golden = text(&git_stdout(r.path(), &["diff", "--cached", "--shortstat"]));
    assert_eq!(rendered, golden, "shortstat must match git byte-for-byte");
}

#[test]
fn summary_reports_create_delete_rename_with_member_paths() {
    let r = TempRepo::new("summary");
    r.write("old.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon\n");
    r.write("gone.txt", b"bye\n");
    r.commit("init");
    r.remove("old.txt");
    r.write("new.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n");
    r.remove("gone.txt");
    r.write("fresh.txt", b"hi\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let rendered = text(&render_summary(&entries, &member_opts()));

    assert!(rendered.contains(&format!(" create mode 100644 {MEMBER}/fresh.txt")));
    assert!(rendered.contains(&format!(" delete mode 100644 {MEMBER}/gone.txt")));
    assert!(
        rendered.contains(&format!(" rename {MEMBER}/old.txt => {MEMBER}/new.txt (")),
        "summary rename with member paths:\n{rendered}"
    );
}

#[test]
fn stat_lists_files_workspace_relative_in_order() {
    let r = TempRepo::new("stat");
    r.write("zeta.txt", b"1\n2\n");
    r.write("alpha.txt", b"a\n");
    r.commit("init");
    r.write("zeta.txt", b"1\n2\n3\n4\n");
    r.write("alpha.txt", b"a\nb\n");
    r.stage();

    let (_c, _o, manifest) = r.cached_manifest();
    let scope = member_scope();
    let entries = render_entries(&manifest, &scope);
    let rendered = text(&render_stat(&entries, &member_opts()));

    // Every changed file appears with the member prefix and a `|` graph column.
    for e in &manifest.entries {
        let p = format!("{MEMBER}/{}", e.new_path.as_deref().unwrap());
        assert!(rendered.contains(&p), "stat missing {p}:\n{rendered}");
    }
    assert!(rendered.contains('|'), "stat should have a graph column");
    // Ends with the shortstat line.
    let last = rendered.lines().last().unwrap();
    assert!(last.contains("files changed") || last.contains("file changed"));
    // Manifest order preserved: first stat line is the first manifest entry.
    let first_line = rendered.lines().next().unwrap();
    let first_path = format!(
        "{MEMBER}/{}",
        manifest.entries[0].new_path.as_deref().unwrap()
    );
    assert!(
        first_line.trim_start().starts_with(&first_path),
        "stat order must match manifest order:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Golden helpers: prefix git's repo-relative path columns with the member.
// ---------------------------------------------------------------------------

fn prefix_name_status_line(line: &str) -> String {
    let mut parts = line.split('\t');
    let status = parts.next().unwrap_or("");
    let rest: Vec<String> = parts.map(|p| format!("{MEMBER}/{p}")).collect();
    format!("{status}\t{}", rest.join("\t"))
}

/// Drop the similarity digits from a name-status rename status field
/// (`R090\t…` -> `R\t…`) so a comparison is metric-agnostic (spike Q2a).
fn normalize_rename_sim(line: &str) -> String {
    if let Some(rest) = line.strip_prefix('R') {
        // rest = "<digits>\t<old>\t<new>"; drop the leading digits.
        let after = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        format!("R{after}")
    } else {
        line.to_owned()
    }
}

fn prefix_numstat_line(line: &str) -> String {
    // `<ins>\t<del>\t<path>`; only the path column gets the prefix.
    let mut parts = line.splitn(3, '\t');
    let ins = parts.next().unwrap_or("");
    let del = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    format!("{ins}\t{del}\t{MEMBER}/{path}")
}
