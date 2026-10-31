//! Other spellings of one existing directory, for the rows that prove a key is the filesystem's identity of a path
//! and not the way a request wrote it.

use std::fs;
use std::path::{Path, PathBuf};

/// `path` spelled with the case of its last component swapped, when the volume it is on treats that spelling as the
/// same directory (Windows, default macOS APFS), and `None` with the reason when it does not (a case-sensitive
/// volume, or a last component without letters). The two spellings are checked by what the filesystem says, not
/// assumed from the platform.
pub(crate) fn case_variant(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no last component to respell", path.display()))?;
    let swapped: String = name
        .chars()
        .map(|letter| {
            if letter.is_lowercase() {
                letter.to_uppercase().collect::<String>()
            } else {
                letter.to_lowercase().collect::<String>()
            }
        })
        .collect();
    if swapped == name {
        return Err(format!("{name} has no letters to change the case of"));
    }
    let variant = path.with_file_name(swapped);
    match (fs::canonicalize(&variant), fs::canonicalize(path)) {
        (Ok(spelled), Ok(real)) if spelled == real => Ok(variant),
        _ => Err(format!(
            "the volume of {} is case-sensitive: {} is not the same directory",
            path.display(),
            variant.display()
        )),
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// A symlink named `link` to the directory `target`.
        pub(crate) fn symlink_spelling(target: &Path, link: &Path) {
            std::os::unix::fs::symlink(target, link).expect("a symlink to the workspace");
        }
    }
}
