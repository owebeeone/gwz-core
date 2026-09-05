//! Source-layout hazard constructors: one method per
//! `dev-docs/GwzLocalCloneDesign.md` §4.0 item, named for the hazard it makes.
//!
//! Design §4.0 refuses a create whose source has any of these, aggregating
//! them before reservation, and `gwz_repo_contract::LayoutHazard` is the
//! vocabulary the inspector reports them in. Each constructor here builds
//! exactly one of them on a repository that is otherwise ordinary, so a
//! preflight test can pin one outcome per fixture.
//!
//! Several of these deliberately leave the repository in a state `git2`
//! cannot open (an external `commondir`, a raised `core.repositoryformatversion`
//! with an extension libgit2 does not implement). That is the point: the
//! preflight refuses *before* it relies on opening the source. Use
//! [`TestRepo::try_open`] and [`TestRepo::layout_git_dir`] on those, and apply
//! at most one layout hazard per fixture repository.

use std::path::{Path, PathBuf};

use crate::{TestRepo, create_dir_all, read_file, write_file};

impl TestRepo {
    /// `.git` as a **file** (a gitfile / linked-worktree source): the real
    /// Git directory is moved to a sibling of the repository and `.git`
    /// becomes a one-line `gitdir:` pointer at it. Returns the relocated
    /// directory.
    pub fn hazard_git_file(&self) -> PathBuf {
        assert!(
            !self.is_bare(),
            "a bare repository has no `.git` entry to replace"
        );
        let git_dir = self.layout_git_dir();
        let name = self
            .path()
            .file_name()
            .expect("repository path has a final component");
        let relocated = self
            .path()
            .parent()
            .expect("repository path has a parent")
            .join(format!("{}.gitdir", name.to_string_lossy()));
        std::fs::rename(&git_dir, &relocated).unwrap_or_else(|error| {
            panic!(
                "move {} to {}: {error}",
                git_dir.display(),
                relocated.display()
            )
        });
        write_file(
            &git_dir,
            format!("gitdir: {}\n", relocated.display()).as_bytes(),
        );
        relocated
    }

    /// A `commondir` naming a directory outside this member. Returns the
    /// `commondir` file.
    pub fn hazard_external_common_dir(&self, external: &Path) -> PathBuf {
        create_dir_all(external);
        let marker = self.layout_git_dir().join("commondir");
        write_file(&marker, format!("{}\n", external.display()).as_bytes());
        marker
    }

    /// `objects/info/alternates` naming a borrowed object store — what an
    /// inherited `clone --shared` / `--reference` leaves behind. Returns the
    /// alternates file.
    pub fn hazard_alternates(&self, external_objects: &Path) -> PathBuf {
        create_dir_all(external_objects);
        let alternates = self
            .layout_git_dir()
            .join("objects")
            .join("info")
            .join("alternates");
        write_file(
            &alternates,
            format!("{}\n", external_objects.display()).as_bytes(),
        );
        alternates
    }

    /// `objects/info/http-alternates`, the remote sibling of the above.
    /// Returns the file.
    pub fn hazard_http_alternates(&self, url: &str) -> PathBuf {
        let alternates = self
            .layout_git_dir()
            .join("objects")
            .join("info")
            .join("http-alternates");
        write_file(&alternates, format!("{url}\n").as_bytes());
        alternates
    }

    /// The **object store** reached through a symlink out of the repository:
    /// `<git dir>/objects` is moved to `external` and replaced by a link.
    /// The repository stays openable, which is exactly why a copy that
    /// followed the link would silently borrow objects. Returns `external`.
    #[cfg(unix)]
    pub fn hazard_escaping_object_store_link(&self, external: &Path) -> PathBuf {
        let objects = self.layout_git_dir().join("objects");
        if let Some(parent) = external.parent() {
            create_dir_all(parent);
        }
        std::fs::rename(&objects, external).unwrap_or_else(|error| {
            panic!(
                "move {} to {}: {error}",
                objects.display(),
                external.display()
            )
        });
        std::os::unix::fs::symlink(external, &objects)
            .unwrap_or_else(|error| panic!("symlink {}: {error}", objects.display()));
        external.to_path_buf()
    }

    /// **Git metadata** reached through a symlink out of the repository: the
    /// whole Git directory is moved to `external` and `.git` becomes a
    /// symlink to it (distinct from [`Self::hazard_git_file`], which makes
    /// `.git` a regular file). Returns `external`.
    #[cfg(unix)]
    pub fn hazard_escaping_metadata_link(&self, external: &Path) -> PathBuf {
        assert!(
            !self.is_bare(),
            "a bare repository has no `.git` entry to replace"
        );
        let git_dir = self.layout_git_dir();
        if let Some(parent) = external.parent() {
            create_dir_all(parent);
        }
        std::fs::rename(&git_dir, external).unwrap_or_else(|error| {
            panic!(
                "move {} to {}: {error}",
                git_dir.display(),
                external.display()
            )
        });
        std::os::unix::fs::symlink(external, &git_dir)
            .unwrap_or_else(|error| panic!("symlink {}: {error}", git_dir.display()));
        external.to_path_buf()
    }

    /// `core.worktree` naming a directory outside the destination.
    pub fn hazard_core_worktree(&self, external: &Path) {
        create_dir_all(external);
        self.set_config("core.worktree", &external.to_string_lossy());
    }

    /// An **absolute** `core.hooksPath` outside the destination.
    pub fn hazard_absolute_hooks_path(&self, external: &Path) {
        create_dir_all(external);
        self.set_config("core.hooksPath", &external.to_string_lossy());
    }

    /// A **relative** `core.hooksPath`. Design §4.0 resolves these against
    /// the hook's working directory before reservation, so this constructor
    /// serves both outcomes: `"../shared-hooks"` escapes and refuses,
    /// `"hooks"` stays inside and is admitted. The resolved directory is
    /// created (a resolver that checks existence finds it) and returned.
    pub fn hazard_relative_hooks_path(&self, relative: &str) -> PathBuf {
        self.set_config("core.hooksPath", relative);
        let resolved = self.path().join(relative);
        create_dir_all(&resolved);
        resolved
    }

    /// A `core.hooksPath` that cannot be resolved at all: it is reached
    /// *through* a regular file, so no working-directory base makes it a
    /// directory. Returns the blocking file.
    pub fn hazard_unresolvable_hooks_path(&self) -> PathBuf {
        let blocker = self.layout_git_dir().join("gwz-fixture-not-a-directory");
        write_file(&blocker, b"not a directory\n");
        self.set_config("core.hooksPath", &blocker.join("hooks").to_string_lossy());
        blocker
    }

    /// `include.path` naming a configuration file outside the destination.
    /// The included file is created (with one harmless key) and returned.
    pub fn hazard_include_path(&self, external: &Path) -> PathBuf {
        write_file(external, b"[gwz]\n\tfixture = included\n");
        self.append_config(&format!("[include]\n\tpath = {}\n", external.display()));
        external.to_path_buf()
    }

    /// `includeIf "<condition>"` naming a configuration file outside the
    /// destination — the conditional form, which a resolver must expand
    /// before it can say whether the effective path escapes. Returns the
    /// included file.
    pub fn hazard_include_if(&self, condition: &str, external: &Path) -> PathBuf {
        write_file(external, b"[gwz]\n\tfixture = conditionally-included\n");
        self.append_config(&format!(
            "[includeIf \"{condition}\"]\n\tpath = {}\n",
            external.display()
        ));
        external.to_path_buf()
    }

    /// `url.<replacement>.insteadOf = <prefix>`: a rewrite that can turn an
    /// ordinary-looking remote into a path outside the destination.
    pub fn hazard_url_insteadof(&self, prefix: &str, replacement: &str) {
        self.set_config(&format!("url.{replacement}.insteadOf"), prefix);
    }

    /// A partial clone whose promised objects are not available locally:
    /// `extensions.partialClone` naming `promisor_remote`, that remote marked
    /// `promisor` with a `blob:none` filter, `core.repositoryformatversion`
    /// raised to 1 as Git requires for an extension, and a `.promisor` marker
    /// in the pack directory. Returns the marker.
    ///
    /// Apply this last: libgit2 does not implement the `partialClone`
    /// extension, so it refuses to open a repository that declares it at
    /// format version 1 — which is itself the honest source state a preflight
    /// must refuse rather than open.
    pub fn hazard_partial_clone(&self, promisor_remote: &str) -> PathBuf {
        self.set_config(
            &format!("remote.{promisor_remote}.partialclonefilter"),
            "blob:none",
        );
        self.set_config(&format!("remote.{promisor_remote}.promisor"), "true");
        self.set_config("extensions.partialClone", promisor_remote);
        self.set_config("core.repositoryformatversion", "1");
        let marker = self
            .layout_git_dir()
            .join("objects")
            .join("pack")
            .join("pack-gwzfixture.promisor");
        write_file(&marker, b"");
        marker
    }

    // ---- shared helpers -------------------------------------------------

    /// The Git directory **by construction** (`<path>` when bare,
    /// `<path>/.git` otherwise), which stays right even when a hazard has
    /// made the repository unopenable. [`TestRepo::git_dir`] asks the
    /// repository instead, and so follows a gitfile or a metadata symlink.
    pub fn layout_git_dir(&self) -> PathBuf {
        if self.is_bare() {
            self.path().to_path_buf()
        } else {
            self.path().join(".git")
        }
    }

    /// Set one local configuration key.
    pub fn set_config(&self, key: &str, value: &str) {
        let repository = self.open();
        repository
            .config()
            .expect("repository configuration")
            .set_str(key, value)
            .unwrap_or_else(|error| panic!("set {key}: {error}"));
    }

    /// Read one effective configuration value, if it is set.
    pub fn config_value(&self, key: &str) -> Option<String> {
        self.open()
            .config()
            .expect("repository configuration")
            .get_string(key)
            .ok()
    }

    /// The raw text of the repository's own configuration file — the way to
    /// observe keys (`include.path`, `includeIf`) that a config *parser*
    /// consumes rather than reports.
    pub fn config_text(&self) -> String {
        String::from_utf8(read_file(&self.layout_git_dir().join("config")))
            .expect("repository configuration is UTF-8")
    }

    /// Append raw text to the repository's own configuration file.
    pub fn append_config(&self, text: &str) {
        let path = self.layout_git_dir().join("config");
        let mut contents = read_file(&path);
        contents.extend_from_slice(text.as_bytes());
        write_file(&path, &contents);
    }
}
