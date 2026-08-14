use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cap_fs_ext::MetadataExt;
use cap_std::fs::{Dir, File};

use super::filesystem::{filesystem_provider_for_test, filesystem_provider_with_hook_for_test};
use super::*;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PathEquivalenceProvider,
};

#[derive(Clone, Copy)]
struct FakePlatform {
    mode: PathComponentMode,
}

impl FakePlatform {
    fn sensitive() -> Self {
        Self {
            mode: PathComponentMode::Sensitive,
        }
    }

    fn folded() -> Self {
        Self {
            mode: PathComponentMode::AsciiCaseFold,
        }
    }

    fn identity(metadata: &impl MetadataExt) -> Vec<u8> {
        let mut value = Vec::with_capacity(16);
        value.extend_from_slice(&metadata.dev().to_be_bytes());
        value.extend_from_slice(&metadata.ino().to_be_bytes());
        value
    }

    fn dir_fact(
        directory: &Dir,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
        let invocation = Self::identity(
            &directory
                .dir_metadata()
                .map_err(|source| CheckedFsError::io("test directory identity", source))?,
        );
        Ok(ObjectIdentityFact::new(
            DurableObjectIdentityV1::linux_ext4([7; 16], 1, invocation.clone())?,
            invocation,
        ))
    }

    fn file_fact(
        file: &File,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
        let invocation = Self::identity(
            &file
                .metadata()
                .map_err(|source| CheckedFsError::io("test file identity", source))?,
        );
        Ok(ObjectIdentityFact::new(
            DurableObjectIdentityV1::linux_ext4([7; 16], 1, invocation.clone())?,
            invocation,
        ))
    }
}

impl PathEquivalenceProvider<Dir> for FakePlatform {
    fn parent_mode(&self, _parent: &Dir) -> Result<PathComponentMode, CheckedFsError> {
        Ok(self.mode)
    }
}

impl super::super::super::DurableIdentityProvider<Dir, File> for FakePlatform {
    type InvocationIdentity = Vec<u8>;
    type RenameDomain = Vec<u8>;

    fn support_profile(&self) -> SupportedFilesystemProfile {
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1
    }

    fn dir_identity(
        &self,
        directory: &Dir,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
        Self::dir_fact(directory)
    }

    fn file_identity(
        &self,
        file: &File,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
        Self::file_fact(file)
    }

    fn rename_domain(&self, _directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
        Ok(vec![9; 16])
    }
}

#[derive(Debug)]
struct CapturedCatalog {
    digest: [u8; 32],
    root_kind: PreCatalogRootKindV1,
    support_profile: SupportedFilesystemProfile,
    path: Vec<Vec<u8>>,
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "gwz-r2b-pre-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        fs::create_dir(root.join(".gwz")).unwrap();
        Self { root }
    }

    fn repo(&self) -> git2::Repository {
        git2::Repository::open(&self.root).unwrap()
    }

    fn add_index(&self, path: &[u8], stage: u16, mode: u32, extended: u16) {
        self.add_index_with_content(path, stage, mode, extended, b"index fixture\n");
    }

    fn add_index_with_content(
        &self,
        path: &[u8],
        stage: u16,
        mode: u32,
        extended: u16,
        content: &[u8],
    ) {
        Self::add_index_at(&self.root, path, stage, mode, extended, content);
    }

    fn add_index_at(
        root: &Path,
        path: &[u8],
        stage: u16,
        mode: u32,
        extended: u16,
        content: &[u8],
    ) {
        let repo = git2::Repository::open(root).unwrap();
        let id = repo.blob(content).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode,
                uid: 0,
                gid: 0,
                file_size: 0,
                id,
                flags: stage << 12,
                flags_extended: extended,
                path: path.to_vec(),
            })
            .unwrap();
        index.write().unwrap();
    }

    fn private_catalog(&self) -> PathBuf {
        self.root.join(".gwz/checked-artifacts")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_workspace(
    root: &Path,
    platform: FakePlatform,
    hook: Option<Arc<dyn Fn() + Send + Sync>>,
) -> (Result<CapturedCatalog, CheckedFsError>, Arc<AtomicBool>) {
    let called = Arc::new(AtomicBool::new(false));
    let provider = filesystem_provider_with_hook_for_test(platform, hook);
    let result = provider
        .observe_and_revalidate_workspace_for_test(root)
        .map(|observation| {
            called.store(true, Ordering::SeqCst);
            capture(observation, PreCatalogRootKindV1::Workspace)
        });
    (result, called)
}

fn capture(
    observation: RawPreCatalogObservationV1<RetainedPlatformRoot>,
    root_kind: PreCatalogRootKindV1,
) -> CapturedCatalog {
    CapturedCatalog {
        digest: observation.collision_snapshot_digest,
        root_kind,
        support_profile: observation.support_profile,
        path: observation
            .path_profile
            .components()
            .iter()
            .map(|component| component.original().as_bytes().to_vec())
            .collect(),
    }
}

#[test]
fn workspace_observation_binds_real_index_and_revalidates_before_bootstrap() {
    let fixture = Fixture::new();
    fixture.add_index(b"ordinary.txt", 0, 0o100644, 0);
    let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), None);
    let catalog = result.unwrap();
    assert!(called.load(Ordering::SeqCst));
    assert_ne!(catalog.digest, [0; 32]);
    assert_eq!(catalog.root_kind, PreCatalogRootKindV1::Workspace);
    assert_eq!(catalog.path, [b".gwz".to_vec()]);
    assert!(!fixture.private_catalog().exists());
    assert!(fixture.root.exists());
}

#[test]
fn exact_ancestor_descendant_stage_flag_and_gitlink_collisions_are_read_only() {
    let rows: &[(&[u8], u16, u32, u16)] = &[
        (b".gwz/checked-artifacts", 0, 0o100644, 0),
        (b".gwz", 0, 0o100644, 0),
        (b".gwz/checked-artifacts/owned", 0, 0o100644, 0),
        (b".gwz/checked-artifacts", 1, 0o100644, 0),
        (b".gwz/checked-artifacts", 2, 0o100644, 0),
        (b".gwz/checked-artifacts", 3, 0o100644, 0),
        (b".gwz/checked-artifacts", 0, 0o100644, 0x4000),
        (b".gwz/checked-artifacts", 0, 0o160000, 0),
    ];
    for (path, stage, mode, extended) in rows {
        let fixture = Fixture::new();
        fixture.add_index(path, *stage, *mode, *extended);
        let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), None);
        assert!(result.is_err(), "collision should reject: {path:?}");
        assert!(!called.load(Ordering::SeqCst));
        assert!(!fixture.private_catalog().exists());
    }
}

#[test]
fn platform_equivalent_index_spelling_collides() {
    let fixture = Fixture::new();
    fixture.add_index(b".GWZ/CHECKED-ARTIFACTS", 0, 0o100644, 0);
    let (result, called) = run_workspace(&fixture.root, FakePlatform::folded(), None);
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!fixture.private_catalog().exists());
}

#[test]
fn platform_equivalent_workspace_parent_spelling_rejects() {
    let fixture = Fixture::new();
    fs::rename(fixture.root.join(".gwz"), fixture.root.join(".GWZ")).unwrap();
    let (result, called) = run_workspace(&fixture.root, FakePlatform::folded(), None);
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!fixture.private_catalog().exists());
}

#[test]
fn complete_index_change_between_observation_and_revalidation_blocks_bootstrap() {
    let fixture = Fixture::new();
    fixture.add_index(b"ordinary.txt", 0, 0o100644, 0);
    let root = fixture.root.clone();
    let hook = Arc::new(move || {
        let repo = git2::Repository::open(&root).unwrap();
        let id = repo.blob(b"changed fixture\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size: 0,
                id,
                flags: 0,
                flags_extended: 0,
                path: b"changed-after-observe.txt".to_vec(),
            })
            .unwrap();
        index.write().unwrap();
    });
    let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), Some(hook));
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!fixture.private_catalog().exists());
}

#[test]
fn same_path_object_change_between_observation_and_revalidation_blocks_bootstrap() {
    let fixture = Fixture::new();
    fixture.add_index(b"ordinary.txt", 0, 0o100644, 0);
    let root = fixture.root.clone();
    let hook = Arc::new(move || {
        Fixture::add_index_at(&root, b"ordinary.txt", 0, 0o100644, 0, b"changed object\n");
    });
    let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), Some(hook));
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!fixture.private_catalog().exists());
}

#[test]
fn tracked_worktree_kind_change_before_revalidation_blocks_bootstrap() {
    for directory in [false, true] {
        let fixture = Fixture::new();
        fixture.add_index(b"ordinary.txt", 0, 0o100644, 0);
        let path = fixture.root.join("ordinary.txt");
        let hook = Arc::new(move || {
            if directory {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, b"appeared\n").unwrap();
            }
        });
        let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), Some(hook));
        assert!(result.is_err());
        assert!(!called.load(Ordering::SeqCst));
        assert!(!fixture.private_catalog().exists());
    }
}

#[cfg(unix)]
#[test]
fn tracked_worktree_symlink_appearance_before_revalidation_blocks_bootstrap() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    fixture.add_index(b"ordinary.txt", 0, 0o100644, 0);
    let path = fixture.root.join("ordinary.txt");
    let hook = Arc::new(move || symlink("missing-target", &path).unwrap());
    let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), Some(hook));
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!fixture.private_catalog().exists());
}

#[test]
fn private_namespace_change_between_observation_and_revalidation_blocks_bootstrap() {
    let fixture = Fixture::new();
    let private = fixture.private_catalog();
    let hook = Arc::new(move || fs::create_dir(&private).unwrap());
    let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), Some(hook));
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
}

#[test]
fn git_directory_namespace_change_before_revalidation_blocks_bootstrap() {
    let fixture = Fixture::new();
    let git_dir = fixture.repo().path().to_path_buf();
    let private_parent = git_dir.join("gwz");
    let hook = Arc::new(move || fs::create_dir(&private_parent).unwrap());
    let called = Arc::new(AtomicBool::new(false));
    let provider = filesystem_provider_with_hook_for_test(FakePlatform::sensitive(), Some(hook));
    let result = provider
        .observe_and_revalidate_git_directory_for_test(&git_dir)
        .map(|observation| {
            called.store(true, Ordering::SeqCst);
            capture(observation, PreCatalogRootKindV1::GitDirectory)
        });
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
}

#[cfg(unix)]
#[test]
fn symlinked_workspace_parent_rejects_before_catalog_bootstrap() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    fs::remove_dir(fixture.root.join(".gwz")).unwrap();
    let outside = fixture.root.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.root.join(".gwz")).unwrap();
    let (result, called) = run_workspace(&fixture.root, FakePlatform::sensitive(), None);
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!outside.join("checked-artifacts").exists());
}

#[test]
fn wrong_kind_git_directory_parent_rejects_without_replacement() {
    let fixture = Fixture::new();
    let git_dir = fixture.repo().path().to_path_buf();
    fs::write(git_dir.join("gwz"), b"foreign\n").unwrap();
    let called = Arc::new(AtomicBool::new(false));
    let provider = filesystem_provider_for_test(FakePlatform::sensitive());
    assert!(
        provider
            .observe_and_revalidate_git_directory_for_test(&git_dir)
            .is_err()
    );
    assert!(!called.load(Ordering::SeqCst));
    assert_eq!(fs::read(git_dir.join("gwz")).unwrap(), b"foreign\n");
}

#[test]
fn git_directory_entry_requires_the_actual_git_directory() {
    let fixture = Fixture::new();
    let git_dir = fixture.repo().path().to_path_buf();
    let called = Arc::new(AtomicBool::new(false));
    let provider = filesystem_provider_for_test(FakePlatform::sensitive());
    let catalog = provider
        .observe_and_revalidate_git_directory_for_test(&git_dir)
        .map(|observation| {
            called.store(true, Ordering::SeqCst);
            capture(observation, PreCatalogRootKindV1::GitDirectory)
        })
        .unwrap();
    assert!(called.load(Ordering::SeqCst));
    assert_eq!(catalog.root_kind, PreCatalogRootKindV1::GitDirectory);
    assert_eq!(catalog.path, [b"gwz".to_vec()]);
    assert!(!git_dir.join("gwz").exists());

    let called = Arc::new(AtomicBool::new(false));
    assert!(
        provider
            .observe_and_revalidate_git_directory_for_test(&fixture.root)
            .is_err()
    );
    assert!(!called.load(Ordering::SeqCst));
}

#[test]
fn linked_worktree_observation_retains_actual_and_common_git_directories() {
    let main = Fixture::new();
    let linked_parent = Fixture::new();
    let linked_root = linked_parent.root.join("linked");
    let main_repo = main.repo();
    let tree_id = main_repo.index().unwrap().write_tree().unwrap();
    let tree = main_repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
    main_repo
        .commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
        .unwrap();
    main_repo.worktree("linked", &linked_root, None).unwrap();
    fs::create_dir(linked_root.join(".gwz")).unwrap();

    let linked_repo = git2::Repository::open(&linked_root).unwrap();
    assert_ne!(linked_repo.path(), linked_repo.commondir());
    let (result, called) = run_workspace(&linked_root, FakePlatform::sensitive(), None);
    let catalog = result.unwrap();
    assert!(called.load(Ordering::SeqCst));
    assert_eq!(catalog.root_kind, PreCatalogRootKindV1::Workspace);
    assert_eq!(catalog.path, [b".gwz".to_vec()]);
}

#[cfg(target_os = "macos")]
#[test]
fn native_macos_provider_issues_the_persistent_identity_profile() {
    let fixture = Fixture::new();
    let called = Arc::new(AtomicBool::new(false));
    let catalog = super::filesystem::platform_pre_catalog_provider()
        .observe_and_revalidate_workspace_for_test(&fixture.root)
        .map(|observation| {
            called.store(true, Ordering::SeqCst);
            capture(observation, PreCatalogRootKindV1::Workspace)
        })
        .unwrap();
    assert!(called.load(Ordering::SeqCst));
    assert_eq!(
        catalog.support_profile,
        SupportedFilesystemProfile::MacPersistentObjectIdV1
    );
    assert_eq!(catalog.path, [b".gwz".to_vec()]);
    assert!(!fixture.private_catalog().exists());
}

#[test]
fn production_provider_observation_is_derived_from_the_workspace_lease() {
    let fixture = Fixture::new();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime
        .catalog_mutation_lease()
        .begin_preflight()
        .expect("lease-derived target witness");
    let provider = super::filesystem::platform_pre_catalog_provider();
    let bound = provider.inspect_bound_catalog_target(witness).unwrap();
    provider
        .revalidate_bound_target(&bound.target, &bound.observation)
        .unwrap();
    assert_eq!(
        bound.target.facts().unwrap().root_kind(),
        PreCatalogRootKindV1::Workspace
    );
}

#[test]
fn provider_rejects_target_substitution_after_witness_issuance() {
    let fixture = Fixture::new();
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(&fixture.root);
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("Git target lease");
    let witness = leases
        .leases()
        .next()
        .unwrap()
        .begin_preflight()
        .expect("lease-derived target witness");
    let git = fs::canonicalize(fixture.repo().commondir()).unwrap();
    let retired = fixture.root.join("retired-provider-git-target");
    fs::rename(&git, &retired).unwrap();
    git2::Repository::init(&fixture.root).unwrap();

    assert!(
        super::filesystem::platform_pre_catalog_provider()
            .inspect_bound_catalog_target(witness)
            .is_err()
    );
    assert!(!git.join("gwz").exists());
    assert!(!retired.join("gwz").exists());
}

#[test]
fn provider_rejects_cross_target_lease_and_observation_pairing() {
    let first = Fixture::new();
    let second = Fixture::new();
    let first_runtime = try_acquire_workspace_runtime(&first.root)
        .unwrap()
        .expect("first workspace lease");
    let second_runtime = try_acquire_workspace_runtime(&second.root)
        .unwrap()
        .expect("second workspace lease");
    let first_target = first_runtime
        .catalog_mutation_lease()
        .begin_preflight()
        .unwrap();
    let second_target = second_runtime
        .catalog_mutation_lease()
        .begin_preflight()
        .unwrap();
    let provider = super::filesystem::platform_pre_catalog_provider();
    let second_bound = provider
        .inspect_bound_catalog_target(second_target)
        .unwrap();
    let forged = LeaseBoundPreCatalogObservationV1 {
        target: first_target,
        observation: second_bound.observation,
    };

    assert!(
        provider
            .revalidate_bound_target(&forged.target, &forged.observation)
            .is_err()
    );
    assert!(!first.private_catalog().exists());
    assert!(!second.private_catalog().exists());
}

#[test]
fn provider_rejects_a_substituted_related_git_directory_capability() {
    let first = Fixture::new();
    let second = Fixture::new();
    let first_runtime = try_acquire_workspace_runtime(&first.root)
        .unwrap()
        .expect("first workspace lease");
    let second_runtime = try_acquire_workspace_runtime(&second.root)
        .unwrap()
        .expect("second workspace lease");
    let provider = super::filesystem::platform_pre_catalog_provider();
    let mut first_bound = provider
        .inspect_bound_catalog_target(
            first_runtime
                .catalog_mutation_lease()
                .begin_preflight()
                .unwrap(),
        )
        .unwrap();
    let mut second_bound = provider
        .inspect_bound_catalog_target(
            second_runtime
                .catalog_mutation_lease()
                .begin_preflight()
                .unwrap(),
        )
        .unwrap();
    first_bound
        .observation
        .retained_root
        .swap_repository_for_test(&mut second_bound.observation.retained_root);

    assert!(revalidate_bound_observation(&first_bound).is_err());
    assert!(!first.private_catalog().exists());
    assert!(!second.private_catalog().exists());
}

#[test]
fn case_fold_alias_scan_rejects_maximum_plus_one_parent_entries() {
    let fixture = Fixture::new();
    for index in 0..=crate::checked_artifact::catalog::MAX_CATALOG_PARENT_ENTRIES_V1 {
        fs::write(fixture.root.join(format!("ordinary-{index:04}")), []).unwrap();
    }

    let (result, called) = run_workspace(&fixture.root, FakePlatform::folded(), None);
    assert!(result.is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(!fixture.private_catalog().exists());
}
