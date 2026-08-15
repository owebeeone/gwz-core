use super::*;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use sha1::Sha1;
use sha2::{Digest, Sha256};

pub(super) const MARKER: &str = "gwz.conf/markers/merge_1.yaml";
pub(super) const BOUNDARY: &[u8] = b"ignored/\n";
static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

pub(super) struct RootFixture {
    pub _temp: TempDir,
    pub backend: Git2Backend,
    pub root: PathBuf,
    pub spec: GitRootPreservationSpec,
}

#[derive(Debug, Eq, PartialEq)]
#[allow(dead_code)] // Shared checkpoint consumed by the following matrix slices.
pub(super) struct ExactRootSnapshot {
    pub head: Vec<u8>,
    pub refs: Vec<(PathBuf, Vec<u8>)>,
    pub packed_refs: Option<Vec<u8>>,
    pub index: Vec<u8>,
    pub marker: Option<Vec<u8>>,
    pub lock: Vec<u8>,
    pub boundary: Vec<u8>,
}

pub(super) fn fixture() -> RootFixture {
    fixture_with_format("sha1")
}

pub(super) fn fixture_with_format(format: &str) -> RootFixture {
    fixture_with_markers(format, None, None, Some(b"handoff marker\n"))
}

pub(super) fn fixture_with_markers(
    format: &str,
    attached_clean_marker: Option<&[u8]>,
    restore_clean_marker: Option<&[u8]>,
    handoff_marker: Option<&[u8]>,
) -> RootFixture {
    let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let temp = TempDir::new(&format!("root-preservation-{format}-{id}"));
    let backend = Git2Backend::new();
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(
        &root,
        &[
            "init",
            "-q",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ],
    );
    // Windows CI: staged checked-artifact sources under repo/.gwz exceed
    // MAX_PATH in the runner temp tree; libgit2 workdir walks honor
    // core.longpaths. Pin autocrlf off for byte-exact comparisons.
    git(&root, &["config", "core.longpaths", "true"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    fs::create_dir_all(root.join("gwz.conf")).unwrap();
    fs::write(root.join(crate::artifact::LOCK_PATH), b"restore lock\n").unwrap();
    write_marker(&root, restore_clean_marker);
    stage_managed(&root);
    commit(&root, "restore", "2000-01-01T00:00:00Z");
    let restore = git_output(&root, &["rev-parse", "HEAD"]);
    fs::write(root.join(crate::artifact::LOCK_PATH), b"attached lock\n").unwrap();
    write_marker(&root, attached_clean_marker);
    stage_managed(&root);
    commit(&root, "attached", "2000-01-02T00:00:00Z");
    let attached = git_output(&root, &["rev-parse", "HEAD"]);
    write_marker(&root, handoff_marker);
    fs::write(root.join(crate::artifact::LOCK_PATH), b"handoff lock\n").unwrap();
    stage_managed(&root);
    fs::write(root.join(".git/info/exclude"), BOUNDARY).unwrap();

    let attached_clean = form(&root, attached_clean_marker, b"attached lock\n");
    let restore_clean = form(&root, restore_clean_marker, b"restore lock\n");
    let handoff = form(&root, handoff_marker, b"handoff lock\n");
    RootFixture {
        _temp: temp,
        backend,
        root,
        spec: GitRootPreservationSpec {
            attached_branch: "main".into(),
            attached_commit: attached,
            restore_commit: restore,
            managed_marker_path: MARKER.into(),
            attached_clean_form: attached_clean,
            restore_clean_form: restore_clean,
            handoff_form: handoff,
            handoff_boundary: BOUNDARY.to_vec(),
            excluded_worktree_paths: Vec::new(),
        },
    }
}

fn write_marker(root: &Path, bytes: Option<&[u8]>) {
    let path = root.join(MARKER);
    if let Some(bytes) = bytes {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    } else {
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        let parent = path.parent().unwrap();
        if parent.is_dir() && parent.read_dir().unwrap().next().is_none() {
            fs::remove_dir(parent).unwrap();
        }
    }
}

fn stage_managed(root: &Path) {
    git(root, &["add", "-A", "--", "gwz.conf"]);
}

fn commit(root: &Path, message: &str, date: &str) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=GWZ Test",
            "-c",
            "user.email=gwz@example.invalid",
            "commit",
            "-q",
            "-m",
            message,
        ])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git commit {message:?} failed");
}

#[allow(dead_code)] // Shared checkpoint consumed by the following matrix slices.
pub(super) fn canonical_form(
    fixture: &RootFixture,
    marker: Option<&[u8]>,
    lock: &[u8],
) -> GitRootManagedForm {
    form(&fixture.root, marker, lock)
}

#[allow(dead_code)] // Shared checkpoint consumed by the following matrix slices.
pub(super) fn exact_snapshot(fixture: &RootFixture) -> ExactRootSnapshot {
    ExactRootSnapshot {
        head: fs::read(fixture.root.join(".git/HEAD")).unwrap(),
        refs: ref_bytes(&fixture.root),
        packed_refs: read_optional(fixture.root.join(".git/packed-refs")),
        index: index_bytes(fixture),
        marker: read_optional(fixture.root.join(MARKER)),
        lock: fs::read(fixture.root.join(crate::artifact::LOCK_PATH)).unwrap(),
        boundary: fs::read(fixture.root.join(".git/info/exclude")).unwrap(),
    }
}

fn ref_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn collect(root: &Path, directory: &Path, refs: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, refs);
            } else {
                refs.push((
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let git = root.join(".git");
    let mut refs = Vec::new();
    collect(&git, &git.join("refs"), &mut refs);
    refs.sort_by(|left, right| left.0.cmp(&right.0));
    refs
}

fn read_optional(path: PathBuf) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("failed to read optional fixture path: {error}"),
    }
}

fn form(root: &Path, marker: Option<&[u8]>, lock: &[u8]) -> GitRootManagedForm {
    let marker = marker.map(|bytes| candidate(MARKER, bytes));
    let lock = candidate(crate::artifact::LOCK_PATH, lock);
    GitRootManagedForm {
        index: GitRootManagedIndexForm {
            marker: fact(root, MARKER, marker.as_ref()),
            lock: fact(root, crate::artifact::LOCK_PATH, Some(&lock)),
        },
        marker,
        lock,
    }
}

fn candidate(path: &str, bytes: &[u8]) -> GitCandidateFile {
    GitCandidateFile {
        path: path.into(),
        bytes: bytes.to_vec(),
    }
}

fn fact(root: &Path, path: &str, file: Option<&GitCandidateFile>) -> GitRootManagedIndexFact {
    file.map_or_else(
        || GitRootManagedIndexFact::Absent {
            path: path.as_bytes().to_vec(),
        },
        |file| {
            GitRootManagedIndexFact::Present(GitRootManagedIndexEntry {
                path: path.as_bytes().to_vec(),
                object_id: git_with_input(root, &["hash-object", "--stdin"], &file.bytes),
                mode: 0o100644,
                stage: 0,
                assume_valid: false,
                skip_worktree: false,
                intent_to_add: false,
            })
        },
    )
}

pub(super) fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

pub(super) fn git_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn git_with_input(root: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(super) fn inject_index_extension(fixture: &RootFixture, signature: &[u8; 4]) {
    let mut bytes = index_body(fixture);
    bytes.extend(signature);
    bytes.extend(0_u32.to_be_bytes());
    write_index_body(fixture, bytes);
}

pub(super) fn index_bytes(fixture: &RootFixture) -> Vec<u8> {
    fs::read(fixture.root.join(".git/index")).unwrap()
}

pub(super) fn force_v3_header(fixture: &RootFixture) {
    let mut bytes = index_body(fixture);
    bytes[4..8].copy_from_slice(&3_u32.to_be_bytes());
    write_index_body(fixture, bytes);
}

pub(super) fn inject_unknown_extended_flag(fixture: &RootFixture) {
    git(
        &fixture.root,
        &[
            "update-index",
            "--skip-worktree",
            crate::artifact::LOCK_PATH,
        ],
    );
    let mut bytes = index_body(fixture);
    let path = crate::artifact::LOCK_PATH.as_bytes();
    let at = bytes
        .windows(path.len())
        .position(|window| window == path)
        .expect("managed lock entry exists");
    assert_ne!(
        u16::from_be_bytes(bytes[at - 4..at - 2].try_into().unwrap()) & 0x4000,
        0
    );
    bytes[at - 2..at].copy_from_slice(&1_u16.to_be_bytes());
    write_index_body(fixture, bytes);
}

fn index_body(fixture: &RootFixture) -> Vec<u8> {
    let mut bytes = index_bytes(fixture);
    bytes.truncate(bytes.len() - fixture.spec.attached_commit.len() / 2);
    bytes
}

fn write_index_body(fixture: &RootFixture, mut bytes: Vec<u8>) {
    let width = fixture.spec.attached_commit.len() / 2;
    let digest = if width == 20 {
        Sha1::digest(&bytes).to_vec()
    } else {
        Sha256::digest(&bytes).to_vec()
    };
    bytes.extend(digest);
    fs::write(fixture.root.join(".git/index"), bytes).unwrap();
}

pub(super) fn managed_step(
    object: GitRootManagedObject,
    source: GitRootManagedFormName,
    goal: GitRootManagedFormName,
) -> GitRootPreservationPhysicalStep {
    GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
        object,
        source,
        goal,
    })
}

pub(super) fn normalize_steps() -> [GitRootPreservationPhysicalStep; 4] {
    [
        managed_step(
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        managed_step(
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        managed_step(
            GitRootManagedObject::LockWorktree,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
        managed_step(
            GitRootManagedObject::Index,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::AttachedClean,
        ),
    ]
}

pub(super) fn restore_steps() -> [GitRootPreservationPhysicalStep; 4] {
    [
        managed_step(
            GitRootManagedObject::Index,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        managed_step(
            GitRootManagedObject::LockWorktree,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        managed_step(
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
        managed_step(
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::Handoff,
        ),
    ]
}

pub(super) fn prepare(fixture: &RootFixture) -> GitPreparedRootStash {
    fixture
        .backend
        .prepare_root_preservation_stash(&fixture.root, &fixture.spec)
        .unwrap()
}

pub(super) fn normalize(fixture: &RootFixture, guard: &GitRootPreservationGuard) {
    for step in normalize_steps() {
        assert!(matches!(
            fixture
                .backend
                .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &step, guard,)
                .unwrap(),
            GitCheckedPreservationMutation::Applied
                | GitCheckedPreservationMutation::AlreadyComplete
        ));
    }
}

pub(super) fn parent_fixture() -> (RootFixture, GitRootPreservationPhysicalStep) {
    let fixture = fixture();
    let guard = guard(&prepare(&fixture));
    normalize(&fixture, &guard);
    let restores = restore_steps();
    for step in &restores[..2] {
        execute_step(&fixture, step, &GitRootPreservationGuard::OtherwiseClean).unwrap();
    }
    fs::remove_dir(fixture.root.join(crate::artifact::MARKER_DIR)).unwrap();
    (fixture, restores[2].clone())
}

pub(super) fn forward_parent_fixture() -> (
    RootFixture,
    GitRootPreservationPhysicalStep,
    GitRootPreservationGuard,
) {
    let fixture = fixture_with_markers("sha1", Some(b"attached marker\n"), None, None);
    let guard = guard(&prepare(&fixture));
    let step = normalize_steps()[0].clone();
    (fixture, step, guard)
}

pub(super) fn execute_step(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> crate::model::ModelResult<GitCheckedPreservationMutation> {
    fixture.backend.execute_root_preservation_step_checked(
        &fixture.root,
        &fixture.spec,
        step,
        guard,
    )
}

pub(super) fn observe_step(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> GitRootPreservationStepObservation {
    fixture
        .backend
        .observe_root_preservation_step(&fixture.root, &fixture.spec, step, guard)
        .unwrap()
}

pub(super) fn execute_parent(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
) -> crate::model::ModelResult<GitCheckedPreservationMutation> {
    execute_step(fixture, step, &GitRootPreservationGuard::OtherwiseClean)
}

pub(super) fn observe_parent(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
) -> GitRootPreservationStepObservation {
    observe_step(fixture, step, &GitRootPreservationGuard::OtherwiseClean)
}

pub(super) fn create_exact_stage(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
) -> PathBuf {
    fail_next_at(FaultBoundary::AfterParentStageCreate);
    assert_eq!(
        execute_parent(fixture, step).unwrap_err().code,
        ErrorCode::GitCommandFailed
    );
    stages(fixture).pop().expect("deterministic stage exists")
}

pub(super) fn stages(fixture: &RootFixture) -> Vec<PathBuf> {
    fs::read_dir(fixture.root.join("gwz.conf"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".gwz-markers-")
        })
        .collect()
}

pub(super) fn guard(prepared: &GitPreparedRootStash) -> GitRootPreservationGuard {
    GitRootPreservationGuard::NormalizedPreimage {
        sha256: prepared.normalized_image.preimage_sha256.clone(),
    }
}
