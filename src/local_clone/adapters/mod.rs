//! One adapter per library port (LCM1.1, lane C wiring; gwz-dev
//! `dev-docs/GwzLocalCloneLibraryBoundaries.md` §2 "Core implements those
//! ports by translating to concrete libraries or existing GWZ helpers").
//!
//! Each adapter is thin: it translates types, calls the library or the
//! existing helper, and translates errors. No policy lives here -- the
//! ordering, refusals and completion rules are `gwz-workspace-install`'s
//! and `gwz-local-disposal`'s; the layout hazards are `gwz-repo-inspect`'s;
//! the family metadata is `gwz-family-store`'s; the copy is
//! `gwz-refcopy`'s.
//!
//! | Port | Adapter | Behind it |
//! |---|---|---|
//! | `InstallPorts::snapshot_source`, `recheck_source` | [`install::CoreInstallPorts`] over [`inventory`] | `gwz-repo-inspect` layout inspection of every included repository, `git2` for branches and remotes, the manifest/lock digest |
//! | `InstallPorts::observe_destination` | [`install::CoreInstallPorts`] | the filesystem, `gwz-family-store`'s metadata reading, `gwz-repo-inspect` and `gwz-history-check::check_connectivity` for dest-complete, bounded by [`object_census`] (LCM1.1 fix 2) |
//! | `InstallPorts::allocate_destination` | [`install::CoreInstallPorts`] | `std::fs::create_dir` |
//! | `InstallPorts::install_destination_git` | [`git_config`] | `git2` config edits under `gwz-repo-factory::origin_is_kept`; then the managed `.git/info/exclude` block through `workspace_ops::ensure_workspace_exclude`, in every mode (the family record is ignored by the destination's own repository); every create regenerates the same block at the family root under the family lock |
//! | `InstallPorts::construct_repositories` | refuses `Unimplemented` | `gwz-repo-factory` is LCM2.3/LCM3.1 |
//! | `InstallPorts::recapture_configuration`, `publish_manifest` | [`install::CoreInstallPorts`] | `crate::artifact` (`read_lock`, `write_manifest`, the conf-integrity marker) |
//! | `TreeCopier` | `gwz-refcopy::SystemTreeCopier` with [`exclusions`] | -- |
//! | `FamilySession` | `gwz-family-store::YamlFamilyStore` | -- |
//! | `DisposalPorts::observe_target` | [`disposal::CoreDisposalPorts`] | the store's metadata reading, [`inventory`], `gwz-repo-inspect` |
//! | `DisposalPorts::check_history` | [`disposal::CoreDisposalPorts`] | `gwz-history-check`, one call per witness store |
//! | `DisposalPorts::remove_directory` | [`removal`] | an ordinary recursive remover that never follows a symlink |

pub mod disposal;
pub mod exclusions;
mod generated_marker;
pub mod git_config;
pub mod install;
pub mod inventory;
pub mod member_paths;
pub mod object_census;
pub mod removal;
pub mod store;
