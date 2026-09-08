use std::path::Path;

use super::*;
use crate::filesystem::FileSystem;

impl TempDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let _ = prefix;
        let workspace = crate::filesystem::make_filesystem()
            .test_workspace()
            .unwrap();
        Self {
            path: workspace.path().to_path_buf(),
            _workspace: workspace,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn new_git(prefix: &str) -> Self {
        let temp = Self::new(prefix);
        // Runners have no init.defaultBranch config, so libgit2 would name
        // the unborn branch `master` there while developer machines name it
        // `main`; fixtures assert `refs/heads/main`, so pin it (W2 class,
        // GwzWindowsMatrix-Classification.md).
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main");
        git2::Repository::init_opts(temp.path(), &options).unwrap();
        temp
    }
}
