//! The ordinary recursive remover behind `DisposalPorts::remove_directory`
//! (design §5.2 step 4: "Remove only the validated target using ordinary
//! filesystem operations, without following symlink entries into external
//! trees. Stop on an error.").
//!
//! Every entry is classified with `symlink_metadata`, so a symbolic link is
//! removed as a link -- whatever it points at, inside or outside the tree,
//! is never entered and never touched -- and the target itself must be a
//! real directory. The walk is depth-first and stops at the first failure,
//! reporting what remains; nothing is retried, rolled back or replayed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gwz_local_disposal::{PortError, RemovalFailure};

/// Remove `target` and everything beneath it, once.
pub fn remove_tree(target: &Path) -> Result<(), RemovalFailure> {
    let metadata = fs::symlink_metadata(target).map_err(|error| failure(target, target, &error))?;
    if metadata.file_type().is_symlink() {
        return Err(RemovalFailure {
            error: PortError::Removal {
                path: target.to_path_buf(),
                detail: "the target is a symbolic link; the remover never follows one".to_owned(),
            },
            remaining: vec![target.to_path_buf()],
        });
    }
    if !metadata.is_dir() {
        return Err(RemovalFailure {
            error: PortError::Removal {
                path: target.to_path_buf(),
                detail: "the target is not a directory".to_owned(),
            },
            remaining: vec![target.to_path_buf()],
        });
    }
    remove_contents(target, target)?;
    fs::remove_dir(target).map_err(|error| failure(target, target, &error))
}

fn remove_contents(target: &Path, directory: &Path) -> Result<(), RemovalFailure> {
    let entries = fs::read_dir(directory).map_err(|error| failure(target, directory, &error))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| failure(target, directory, &error))?;
        paths.push(entry.path());
    }
    paths.sort();
    for path in paths {
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| failure(target, &path, &error))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            // A link is an entry of this tree; what it names is not.
            remove_link(&path).map_err(|error| failure(target, &path, &error))?;
        } else if file_type.is_dir() {
            remove_contents(target, &path)?;
            fs::remove_dir(&path).map_err(|error| failure(target, &path, &error))?;
        } else {
            fs::remove_file(&path).map_err(|error| failure(target, &path, &error))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn remove_link(path: &Path) -> io::Result<()> {
    // A directory symlink is removed with `remove_dir` on Windows; a file
    // symlink with `remove_file`. Neither follows the link.
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(_) => fs::remove_file(path),
    }
}

#[cfg(not(windows))]
fn remove_link(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

fn failure(target: &Path, path: &Path, error: &io::Error) -> RemovalFailure {
    let mut remaining = vec![target.to_path_buf()];
    if path != target {
        remaining.push(path.to_path_buf());
    }
    RemovalFailure {
        error: PortError::Removal {
            path: path.to_path_buf(),
            detail: error.to_string(),
        },
        remaining,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_removed_and_a_link_out_of_it_is_removed_as_a_link() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(outside.join("kept")).unwrap();
        fs::write(outside.join("kept/file"), b"outside").unwrap();
        let target = temp.path().join("target");
        fs::create_dir_all(target.join("a/b")).unwrap();
        fs::write(target.join("a/b/file"), b"x").unwrap();
        fs::write(target.join("top"), b"y").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, target.join("a/link-out")).unwrap();
            std::os::unix::fs::symlink(outside.join("kept/file"), target.join("file-link"))
                .unwrap();
        }

        remove_tree(&target).expect("removed");
        assert!(!target.exists(), "the target is gone");
        assert!(
            outside.join("kept/file").is_file(),
            "the tree a link pointed out to is untouched"
        );
        assert_eq!(fs::read(outside.join("kept/file")).unwrap(), b"outside");
    }

    #[test]
    fn a_missing_or_non_directory_target_is_a_typed_failure_naming_what_remains() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        let failure = remove_tree(&missing).unwrap_err();
        assert!(matches!(failure.error, PortError::Removal { .. }));
        assert_eq!(failure.remaining, vec![missing]);

        let file = temp.path().join("file");
        fs::write(&file, b"x").unwrap();
        let failure = remove_tree(&file).unwrap_err();
        assert!(failure.error.to_string().contains("not a directory"));
        assert!(file.is_file(), "nothing was removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_target_is_refused_and_its_referent_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("file"), b"x").unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let failure = remove_tree(&link).unwrap_err();
        assert!(failure.error.to_string().contains("symbolic link"));
        assert!(real.join("file").is_file());
        assert!(
            link.symlink_metadata().is_ok(),
            "the link itself is left alone too"
        );
    }
}
