//! Work-state constructors: one method per `dev-docs/GwzLocalCloneDesign.md`
//! §5.1 state, named for the state it makes.
//!
//! §5.1 is the pre-deletion inspection: staged, unstaged, untracked and
//! *ignored* data, conflict stages, mode and link changes, tracked paths whose
//! status is suppressed by `assume-unchanged` / `skip-worktree` / a flag the
//! observer does not interpret, valid sparse absence, an unfinished native Git
//! operation and native stash entries. `gwz_repo_contract::WorkObservation` is
//! the vocabulary an observer reports them in; each constructor here makes one
//! of them on a repository that is otherwise clean.
//!
//! Every one of these needs a non-bare repository with at least one commit,
//! so build the history with [`TestRepo::commit_files`] first: it force-checks
//! out what it commits and would otherwise overwrite the work state.

use std::path::Path;

use git2::{IndexEntryExtendedFlag, IndexEntryFlag};
use gwz_repo_contract::{NativeOperation, ObjectId, SuppressionFlag};

use crate::repo::{FILE_MODE, index_entry};
use crate::{TestRepo, write_file};

/// libgit2's index stage shift: an entry's stage lives in bits 12-13 of
/// `flags` (`GIT_INDEX_ENTRY_STAGESHIFT`).
const STAGE_SHIFT: u16 = 12;

impl TestRepo {
    /// **Staged**: a change written and added to the index, not committed.
    pub fn work_staged(&self, relative: &str, contents: &[u8]) {
        self.write_file(relative, contents);
        self.stage(relative);
    }

    /// **Unstaged**: a tracked path changed in the worktree only.
    pub fn work_unstaged(&self, relative: &str, contents: &[u8]) {
        assert!(
            self.is_tracked(relative),
            "{relative} must be tracked before it can be unstaged-modified"
        );
        self.write_file(relative, contents);
    }

    /// **Untracked**: a file Git has never seen and no ignore rule covers.
    pub fn work_untracked(&self, relative: &str, contents: &[u8]) {
        self.write_file(relative, contents);
    }

    /// **Ignored user data**: the path is added to `.git/info/exclude` and
    /// then written, so the ignore rule itself does not add a second
    /// untracked file. "Ignored does not mean disposable" (design §5.1).
    pub fn work_ignored(&self, relative: &str, contents: &[u8]) {
        let exclude = self.layout_git_dir().join("info").join("exclude");
        let mut rules = if exclude.exists() {
            crate::read_file(&exclude)
        } else {
            Vec::new()
        };
        rules.extend_from_slice(format!("/{relative}\n").as_bytes());
        write_file(&exclude, &rules);
        self.write_file(relative, contents);
    }

    /// **Conflict stages**: `relative` is removed from stage 0 and re-added at
    /// stages 1, 2 and 3 with three different blobs, and the worktree file is
    /// left holding conflict markers.
    pub fn work_conflict(&self, relative: &str) {
        let repository = self.open();
        let mut index = repository.index().expect("repository index");
        index
            .remove_path(Path::new(relative))
            .unwrap_or_else(|error| panic!("clear stage 0 of {relative}: {error}"));
        for stage in 1..=3u16 {
            let contents = format!("conflict stage {stage} of {relative}\n");
            let blob = repository
                .blob(contents.as_bytes())
                .expect("conflict stage blob");
            let mut entry = index_entry(relative, FILE_MODE, blob, contents.len());
            entry.flags = stage << STAGE_SHIFT;
            index
                .add(&entry)
                .unwrap_or_else(|error| panic!("add stage {stage} of {relative}: {error}"));
        }
        index.write().expect("write index");
        self.write_file(
            relative,
            format!(
                "<<<<<<< ours\nconflict stage 2 of {relative}\n=======\n\
                 conflict stage 3 of {relative}\n>>>>>>> theirs\n"
            )
            .as_bytes(),
        );
    }

    /// **Mode change**: a tracked ordinary file made executable on disk while
    /// the index still records `100644`.
    #[cfg(unix)]
    pub fn work_mode_change(&self, relative: &str) {
        assert!(
            self.is_tracked(relative),
            "{relative} must be tracked before its mode can change"
        );
        self.set_executable(relative, true);
    }

    /// **Link change**: a tracked ordinary file replaced by a symbolic link.
    #[cfg(unix)]
    pub fn work_link_change(&self, relative: &str, target: &Path) {
        assert!(
            self.is_tracked(relative),
            "{relative} must be tracked before it can become a link"
        );
        self.write_symlink(relative, target);
    }

    /// **Deleted**: a tracked path removed from the worktree, index untouched.
    pub fn work_deleted(&self, relative: &str) {
        assert!(
            self.is_tracked(relative),
            "{relative} must be tracked before it can be deleted"
        );
        self.remove_file(relative);
    }

    /// **Renamed**: a tracked path moved in the worktree with both halves
    /// staged, so a rename detector sees one rename rather than a delete and
    /// an add.
    pub fn work_renamed(&self, from: &str, to: &str) {
        assert!(
            self.is_tracked(from),
            "{from} must be tracked before it can be renamed"
        );
        let contents = self.read_file(from);
        self.remove_file(from);
        self.write_file(to, &contents);
        let repository = self.open();
        let mut index = repository.index().expect("repository index");
        index
            .remove_path(Path::new(from))
            .unwrap_or_else(|error| panic!("unstage {from}: {error}"));
        index
            .add_path(Path::new(to))
            .unwrap_or_else(|error| panic!("stage {to}: {error}"));
        index.write().expect("write index");
    }

    /// **Status suppression**: set `assume-unchanged`, `skip-worktree`, or a
    /// flag the observer is not required to interpret
    /// ([`SuppressionFlag::Other`], built here as `intent-to-add`), on a
    /// tracked path's index entry. The worktree bytes are left exactly as
    /// they are, so the caller decides whether the physical state matches the
    /// index, differs, or is absent.
    pub fn work_suppressed(&self, relative: &str, flag: SuppressionFlag) {
        let repository = self.open();
        let mut index = repository.index().expect("repository index");
        let mut entry = index
            .get_path(Path::new(relative), 0)
            .unwrap_or_else(|| panic!("{relative} is not tracked at stage 0"));
        match flag {
            SuppressionFlag::AssumeUnchanged => entry.flags |= IndexEntryFlag::VALID.bits(),
            SuppressionFlag::SkipWorktree => {
                entry.flags |= IndexEntryFlag::EXTENDED.bits();
                entry.flags_extended |= IndexEntryExtendedFlag::SKIP_WORKTREE.bits();
            }
            SuppressionFlag::Other => {
                entry.flags |= IndexEntryFlag::EXTENDED.bits();
                entry.flags_extended |= IndexEntryExtendedFlag::INTENT_TO_ADD.bits();
            }
        }
        index
            .add(&entry)
            .unwrap_or_else(|error| panic!("suppress {relative}: {error}"));
        index.write().expect("write index");
    }

    /// **Valid sparse absence**: `core.sparseCheckout` on, a
    /// `info/sparse-checkout` pattern set that excludes `relative`, the
    /// entry marked `skip-worktree`, and the file removed from the worktree.
    /// A status command alone cannot distinguish this from a deletion, which
    /// is why design §5.1 requires explicit handling.
    pub fn work_sparse_absent(&self, relative: &str) {
        self.set_config("core.sparseCheckout", "true");
        let patterns = self.layout_git_dir().join("info").join("sparse-checkout");
        let mut rules = if patterns.exists() {
            crate::read_file(&patterns)
        } else {
            b"/*\n".to_vec()
        };
        rules.extend_from_slice(format!("!/{relative}\n").as_bytes());
        write_file(&patterns, &rules);
        self.work_suppressed(relative, SuppressionFlag::SkipWorktree);
        self.remove_file(relative);
    }

    /// **An unfinished native Git operation**: the on-disk state Git itself
    /// leaves mid-merge, mid-rebase, mid-cherry-pick, mid-revert, mid-bisect
    /// or mid-`am`. [`NativeOperation::Other`] builds `rebase-apply/` without
    /// its `applying` marker, the genuinely ambiguous
    /// "apply-mailbox-or-rebase" state.
    pub fn work_native_operation(&self, operation: NativeOperation) {
        let git_dir = self.layout_git_dir();
        let head = self.head_id().to_hex();
        match operation {
            NativeOperation::Merge => {
                write_file(&git_dir.join("MERGE_HEAD"), format!("{head}\n").as_bytes());
                write_file(&git_dir.join("MERGE_MSG"), b"fixture merge\n");
            }
            NativeOperation::Rebase => {
                let state = git_dir.join("rebase-merge");
                write_file(&state.join("head-name"), b"refs/heads/main\n");
                write_file(&state.join("onto"), format!("{head}\n").as_bytes());
            }
            NativeOperation::CherryPick => {
                write_file(
                    &git_dir.join("CHERRY_PICK_HEAD"),
                    format!("{head}\n").as_bytes(),
                );
            }
            NativeOperation::Revert => {
                write_file(&git_dir.join("REVERT_HEAD"), format!("{head}\n").as_bytes());
            }
            NativeOperation::Bisect => {
                write_file(&git_dir.join("BISECT_LOG"), b"fixture bisect\n");
            }
            NativeOperation::ApplyMailbox => {
                let state = git_dir.join("rebase-apply");
                write_file(&state.join("applying"), b"");
                write_file(&state.join("next"), b"1\n");
                write_file(&state.join("last"), b"1\n");
            }
            NativeOperation::Other => {
                let state = git_dir.join("rebase-apply");
                write_file(&state.join("next"), b"1\n");
                write_file(&state.join("last"), b"1\n");
            }
        }
    }

    /// **A native stash entry**: tracked modifications *and* untracked files
    /// are stashed, so the entry carries an untracked-files commit too and the
    /// worktree is clean afterwards. Call it twice for the "older stash
    /// entries" case design §5.1 also protects. Returns the stash commit id.
    pub fn work_stash(&self, message: &str) -> ObjectId {
        let mut repository = self.open();
        let id = repository
            .stash_save(
                &crate::fixture_signature(),
                message,
                Some(git2::StashFlags::INCLUDE_UNTRACKED),
            )
            .unwrap_or_else(|error| panic!("stash {message:?}: {error}"));
        self.oid(id)
    }

    // ---- observation ----------------------------------------------------

    /// Whether `relative` has a stage-0 index entry.
    pub fn is_tracked(&self, relative: &str) -> bool {
        self.open()
            .index()
            .expect("repository index")
            .get_path(Path::new(relative), 0)
            .is_some()
    }

    /// The index entry for `relative` at `stage`, for a test asserting on
    /// modes, ids or suppression flags.
    pub fn index_entry_at(&self, relative: &str, stage: i32) -> Option<git2::IndexEntry> {
        self.open()
            .index()
            .expect("repository index")
            .get_path(Path::new(relative), stage)
    }

    /// Whether the index holds conflict stages.
    pub fn has_conflicts(&self) -> bool {
        self.open()
            .index()
            .expect("repository index")
            .has_conflicts()
    }

    /// The unfinished native operation the repository is in, if any.
    pub fn repository_state(&self) -> git2::RepositoryState {
        self.open().state()
    }

    /// How many native stash entries the repository holds.
    pub fn stash_count(&self) -> usize {
        let mut repository = self.open();
        let mut count = 0usize;
        repository
            .stash_foreach(|_, _, _| {
                count += 1;
                true
            })
            .expect("stash entries are readable");
        count
    }

    /// The worktree status of `relative`, for a test asserting that a work
    /// state really is what its constructor claims.
    pub fn status_of(&self, relative: &str) -> git2::Status {
        self.open()
            .status_file(Path::new(relative))
            .unwrap_or_else(|error| panic!("status of {relative}: {error}"))
    }
}
