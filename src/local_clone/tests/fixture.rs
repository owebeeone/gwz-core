//! Tiny real fixtures for the adapter slices: a temporary directory, a Git
//! repository with one commit, and a GWZ workspace created through the public
//! handler. Small on purpose; the shared fixture crate (lane T) is for the
//! library lanes.

use std::path::{Path, PathBuf};

use crate::workspace_ops::handle_create_workspace;

pub(super) struct TempDir {
    inner: tempfile::TempDir,
}

impl TempDir {
    pub(super) fn new(label: &str) -> Self {
        Self {
            inner: tempfile::Builder::new()
                .prefix(&format!("gwz-local-clone-{label}-"))
                .tempdir()
                .expect("create temp dir"),
        }
    }

    pub(super) fn path(&self) -> &Path {
        self.inner.path()
    }
}

/// A repository whose `main` holds one commit unique to `label`; returns
/// its hex id. Identities and times are fixed, so two repositories with the
/// same label hold the same commit and two labels never do.
pub(super) fn init_repo_with_commit(path: &Path, bare: bool, label: &str) -> String {
    let repo = if bare {
        git2::Repository::init_bare(path).expect("init bare")
    } else {
        git2::Repository::init(path).expect("init")
    };
    let signature = git2::Signature::new(
        "GWZ Fixture",
        "fixture@example.invalid",
        &git2::Time::new(1_700_000_000, 0),
    )
    .unwrap();
    let tree_id = {
        let mut builder = repo.treebuilder(None).unwrap();
        let blob = repo.blob(format!("fixture {label}\n").as_bytes()).unwrap();
        builder.insert("README", blob, 0o100644).unwrap();
        builder.write().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    let commit = repo
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            &format!("fixture {label}"),
            &tree,
            &[],
        )
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    if !bare {
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }
    commit.to_string()
}

pub(super) fn meta(request_id: &str) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: request_id.to_owned(),
        schema_version: "gwz.v0".to_owned(),
        ..crate::RequestMeta::default()
    }
}

/// A GWZ workspace root created through the public handler.
pub(super) fn workspace(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("ws");
    handle_create_workspace(
        crate::CreateWorkspaceRequest {
            meta: meta("req-create"),
            workspace_root: root.to_string_lossy().into_owned(),
            workspace_id: None,
        },
        "op-create",
    )
    .expect("create workspace");
    root
}

pub(super) fn lock_file(root: &Path) -> PathBuf {
    root.join(gwz_family_model::LOCK_RELATIVE_PATH)
}

pub(super) fn family_files_absent(root: &Path) -> bool {
    !lock_file(root).exists()
        && !root.join(gwz_family_model::INDEX_RELATIVE_PATH).exists()
        && !root.join(gwz_family_model::POINTER_RELATIVE_PATH).exists()
        && !root
            .join(gwz_family_model::ALLOCATION_MARKER_RELATIVE_PATH)
            .exists()
}

/// A real family root built with lane T's fixture harness and the public
/// handlers: a root repository and one member (`app`), each with one
/// commit, registered through `handle_create_workspace` and
/// `handle_add_existing_repo`, then given the dirt a verbatim copy must
/// carry -- an untracked note in the member and an unstaged edit at the
/// root.
pub(super) struct FamilyFixture {
    pub(super) tree: gwz_local_testrepo::TempTree,
    pub(super) workspace: gwz_local_testrepo::TestWorkspace,
    /// The workspace root, canonical (the harness hands out canonical
    /// paths).
    pub(super) root: PathBuf,
}

impl FamilyFixture {
    /// The default destination `gwz clone --local --name <name>` picks from
    /// this root: its sibling `root-<name>`.
    pub(super) fn sibling(&self, name: &str) -> PathBuf {
        self.tree.path().join(format!("root-{name}"))
    }
}

pub(super) fn family_workspace(label: &str) -> FamilyFixture {
    let fixture = registered_family_workspace(label);
    fixture
        .workspace
        .member("app")
        .work_untracked("notes.txt", b"scratch\n");
    fixture
        .workspace
        .root()
        .work_unstaged("README", b"edited\n");
    fixture
}

/// The same family root with **no unsaved work anywhere**: the root's
/// registration files (`gwz.conf/`) are committed, so a verbatim clone of it
/// is the clean, preserved lane ordinary deletion accepts (design §12
/// "Clean intact lane with all protected history elsewhere").
pub(super) fn clean_family_workspace(label: &str) -> FamilyFixture {
    let fixture = registered_family_workspace(label);
    commit_root_configuration(&fixture.root);
    fixture
}

fn registered_family_workspace(label: &str) -> FamilyFixture {
    let tree = gwz_local_testrepo::TempTree::new(label);
    let workspace = tree.workspace("root", &["app"]);
    workspace.commit_all("init");
    let root = workspace.path().to_path_buf();
    handle_create_workspace(
        crate::CreateWorkspaceRequest {
            meta: meta("req-create"),
            workspace_root: root.to_string_lossy().into_owned(),
            workspace_id: None,
        },
        "op-create",
    )
    .expect("create the workspace over the fixture root repository");
    let backend = crate::git::Git2Backend::without_credential_helpers();
    crate::workspace_ops::handle_add_existing_repo(
        &backend,
        &root,
        crate::AddExistingRepoRequest {
            meta: meta("req-add"),
            repository_path: root.join("app").to_string_lossy().into_owned(),
            member_path: Some("app".to_owned()),
            member_id: None,
            source_id: None,
        },
        "op-add",
    )
    .expect("register the member repository");
    FamilyFixture {
        tree,
        workspace,
        root,
    }
}

/// Commit everything the registration left untracked at the root (the
/// manifest, the lock and the marker under `gwz.conf/`), as an operator
/// does; the managed exclude block keeps `.gwz/` and the member paths out.
fn commit_root_configuration(root: &Path) {
    let repository = git2::Repository::open(root).expect("open the root repository");
    let mut index = repository.index().expect("index");
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .expect("stage the registration files");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repository.find_tree(tree_id).expect("tree");
    let parent = repository
        .head()
        .expect("HEAD")
        .peel_to_commit()
        .expect("HEAD commit");
    let signature = gwz_local_testrepo::fixture_signature();
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "register app",
            &tree,
            &[&parent],
        )
        .expect("commit the registration");
}
