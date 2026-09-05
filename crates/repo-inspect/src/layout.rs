//! `inspect_layout`: open one repository read-only and either describe it or
//! name every design §4.0 reason it may not be copied.
//!
//! The rule throughout is **refuse, never rewrite** (design §4.0: "No
//! convert-in-v0. Default is **refuse**, not rewrite."). Nothing here fetches,
//! rewrites an index, clears a flag or runs maintenance; the repository is
//! opened with `NO_SEARCH` so a path that is not itself a repository is not
//! answered with its parent's, and without `FROM_ENV` so libgit2 never applies
//! the environment redirections this module refuses.
//!
//! Hazards are **aggregated**: one call reports every reason it found, because
//! the caller refuses the whole create before reservation (design §4).

use std::path::{Path, PathBuf};

use git2::{ErrorCode, Repository, RepositoryOpenFlags};
use gwz_repo_contract::{HeadState, LayoutError, LayoutHazard, ObjectFormat, RepositoryInfo};

use crate::config_scan;
use crate::environment::Environment;
use crate::oid::to_contract_oid;
use crate::paths::{self, Resolution};

/// Metadata that must live inside the copied boundary. Each is probed under
/// both the Git directory and the common directory.
const METADATA_ENTRIES: &[&str] = &[
    "",
    "objects",
    "objects/info",
    "objects/pack",
    "refs",
    "logs",
    "HEAD",
    "config",
    "index",
];

/// The two alternates files design §4.0 refuses outright, including an
/// inherited one from an earlier `clone --shared` / `--reference`.
const ALTERNATES_FILES: &[&str] = &["objects/info/alternates", "objects/info/http-alternates"];

pub(crate) fn inspect_layout(
    environment: &Environment,
    path: &Path,
) -> Result<RepositoryInfo, LayoutError> {
    let mut hazards = environment.overrides();
    hazards.extend(gitfile_hazard(path));

    let repository = match Repository::open_ext(
        path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&std::ffi::OsStr>(),
    ) {
        Ok(repository) => repository,
        Err(error) if error.code() == ErrorCode::NotFound => {
            return Err(if hazards.is_empty() {
                LayoutError::NotARepository {
                    path: path.to_path_buf(),
                }
            } else {
                LayoutError::Unsupported {
                    path: path.to_path_buf(),
                    hazards,
                }
            });
        }
        Err(error) => {
            return Err(LayoutError::ReadFailed {
                path: path.to_path_buf(),
                detail: error.message().to_owned(),
            });
        }
    };

    let git_dir = real_or_read_failed(path, repository.path())?;
    let common_dir = real_or_read_failed(path, repository.commondir())?;
    let bare = repository.is_bare() || repository.workdir().is_none();
    let work_dir = match repository.workdir() {
        Some(work_dir) => paths::real(work_dir).ok(),
        None => None,
    };
    // The copied boundary is **the directory that will be copied**: the path
    // the caller asked about, resolved. Deriving it from libgit2's notion of
    // the worktree instead would let a `core.worktree` redirection move the
    // boundary along with the escape it is supposed to expose.
    let boundary = real_or_read_failed(path, path)?;

    if repository.is_worktree() {
        hazards.push(LayoutHazard::GitFile {
            path: git_dir.clone(),
        });
    }
    if !paths::contains(&boundary, &common_dir) {
        hazards.push(LayoutHazard::ExternalCommonDir {
            path: common_dir.clone(),
        });
    }
    hazards.extend(alternates_hazards(&git_dir, &common_dir));
    hazards.extend(metadata_link_hazards(&boundary, &git_dir, &common_dir));
    hazards.extend(partial_clone_hazards(&common_dir));
    hazards.extend(config_scan::hazards(
        &boundary,
        &git_dir,
        &common_dir,
        work_dir.as_deref(),
    ));

    if !hazards.is_empty() {
        return Err(LayoutError::Unsupported {
            path: path.to_path_buf(),
            hazards,
        });
    }

    Ok(RepositoryInfo {
        path: boundary,
        git_dir,
        common_dir,
        bare,
        object_format: object_format(&repository),
        head: head_state(&repository, path)?,
    })
}

/// `.git` as a **file** is a gitfile / linked worktree: design §4.0 refuses it
/// before libgit2 gets a chance to follow it somewhere else.
fn gitfile_hazard(path: &Path) -> Option<LayoutHazard> {
    let dot_git = path.join(".git");
    match std::fs::symlink_metadata(&dot_git) {
        Ok(metadata) if metadata.file_type().is_file() => {
            Some(LayoutHazard::GitFile { path: dot_git })
        }
        _ => None,
    }
}

fn alternates_hazards(git_dir: &Path, common_dir: &Path) -> Vec<LayoutHazard> {
    let mut hazards = Vec::new();
    for base in dedup_bases(git_dir, common_dir) {
        for name in ALTERNATES_FILES {
            let candidate = base.join(name);
            if std::fs::symlink_metadata(&candidate).is_ok() {
                hazards.push(LayoutHazard::Alternates { path: candidate });
            }
        }
    }
    hazards
}

/// Git metadata that is *lexically* inside the boundary but *resolves*
/// outside it has been redirected by a symlink; design §4.0 refuses that.
fn metadata_link_hazards(boundary: &Path, git_dir: &Path, common_dir: &Path) -> Vec<LayoutHazard> {
    let mut hazards = Vec::new();
    for base in dedup_bases(git_dir, common_dir) {
        for entry in METADATA_ENTRIES {
            let candidate = if entry.is_empty() {
                base.clone()
            } else {
                base.join(entry)
            };
            if std::fs::symlink_metadata(&candidate).is_err() {
                continue;
            }
            match paths::real(&candidate) {
                Ok(resolved) if !paths::contains(boundary, &resolved) => {
                    hazards.push(LayoutHazard::EscapingMetadataLink {
                        path: candidate,
                        target: resolved,
                    });
                }
                Ok(_) => {}
                Err(error) => hazards.push(LayoutHazard::UnresolvableConfig {
                    key: "metadata".to_owned(),
                    detail: format!("{}: {error}", candidate.display()),
                }),
            }
        }
    }
    hazards
}

/// A partial clone depends on a promisor remote for objects it does not hold.
/// Design §4.0: refuse; there is no implicit network hydration.
fn partial_clone_hazards(common_dir: &Path) -> Vec<LayoutHazard> {
    let mut hazards = Vec::new();
    let config_file = common_dir.join("config");
    if let Ok(config) = git2::Config::open(&config_file)
        && let Ok(entries) = config.entries(None)
    {
        let mut found = Vec::new();
        let _ = entries.for_each(|entry| {
            let name = String::from_utf8_lossy(entry.name_bytes()).to_lowercase();
            if name.starts_with("extensions.partialclone")
                || (name.starts_with("remote.") && name.ends_with(".promisor"))
            {
                found.push(name);
            }
        });
        for name in found {
            hazards.push(LayoutHazard::PartialClone {
                detail: format!("{name} is configured; promised objects are not local"),
            });
        }
    }
    if let Ok(entries) = std::fs::read_dir(common_dir.join("objects/pack")) {
        for entry in entries.flatten() {
            if entry.file_name().as_encoded_bytes().ends_with(b".promisor") {
                hazards.push(LayoutHazard::PartialClone {
                    detail: format!("{} is a promisor pack", entry.path().display()),
                });
            }
        }
    }
    hazards
}

fn dedup_bases(git_dir: &Path, common_dir: &Path) -> Vec<PathBuf> {
    if git_dir == common_dir {
        vec![git_dir.to_path_buf()]
    } else {
        vec![git_dir.to_path_buf(), common_dir.to_path_buf()]
    }
}

fn object_format(repository: &Repository) -> ObjectFormat {
    match repository.object_format() {
        git2::ObjectFormat::Sha256 => ObjectFormat::Sha256,
        _ => ObjectFormat::Sha1,
    }
}

/// `HEAD`, attached, detached or unborn. `branch` carries the **full**
/// reference name (`refs/heads/main`), the same spelling
/// `RootSource::Ref { name }` uses, so the two are comparable without
/// re-deriving a shorthand.
fn head_state(repository: &Repository, path: &Path) -> Result<HeadState, LayoutError> {
    let format = object_format(repository);
    let reference = repository
        .find_reference("HEAD")
        .map_err(|error| read_failed(path, &error))?;
    let symbolic = reference
        .symbolic_target()
        .map_err(|error| read_failed(path, &error))?;
    if let Some(target) = symbolic {
        return match repository.refname_to_id(target) {
            Ok(oid) => Ok(HeadState::Attached {
                branch: target.to_owned(),
                target: to_contract_oid(format, oid).map_err(|detail| LayoutError::ReadFailed {
                    path: path.to_path_buf(),
                    detail,
                })?,
            }),
            Err(error) if error.code() == ErrorCode::NotFound => Ok(HeadState::Unborn {
                branch: target.to_owned(),
            }),
            Err(error) => Err(read_failed(path, &error)),
        };
    }
    let oid = reference.target().ok_or_else(|| LayoutError::ReadFailed {
        path: path.to_path_buf(),
        detail: "HEAD is neither symbolic nor direct".to_owned(),
    })?;
    Ok(HeadState::Detached {
        target: to_contract_oid(format, oid).map_err(|detail| LayoutError::ReadFailed {
            path: path.to_path_buf(),
            detail,
        })?,
    })
}

fn real_or_read_failed(path: &Path, candidate: &Path) -> Result<PathBuf, LayoutError> {
    paths::real(candidate).map_err(|error| LayoutError::ReadFailed {
        path: path.to_path_buf(),
        detail: format!("{}: {error}", candidate.display()),
    })
}

fn read_failed(path: &Path, error: &git2::Error) -> LayoutError {
    LayoutError::ReadFailed {
        path: path.to_path_buf(),
        detail: error.message().to_owned(),
    }
}

/// Shared by [`config_scan`]: turn one resolution into the hazard it implies.
pub(crate) fn hazard_for(key: &str, value: &str, resolution: &Resolution) -> Option<LayoutHazard> {
    match resolution {
        Resolution::Inside(_) => None,
        Resolution::Outside(_) => Some(LayoutHazard::EscapingConfig {
            key: key.to_owned(),
            value: value.to_owned(),
        }),
        Resolution::Escapes { link, target } => Some(LayoutHazard::EscapingConfig {
            key: key.to_owned(),
            value: format!("{value} ({} -> {})", link.display(), target.display()),
        }),
        Resolution::Unresolvable(detail) => Some(LayoutHazard::UnresolvableConfig {
            key: key.to_owned(),
            detail: detail.clone(),
        }),
    }
}
