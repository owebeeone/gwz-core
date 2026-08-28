//! Hand-edit detection for the machine-written workspace state files.
//!
//! `gwz.conf/gwz.yml` and `gwz.conf/gwz.lock.yml` are written by gwz and read back by
//! every command. An agent that edits them by hand desynchronises the recorded workspace
//! from the filesystem — which has actually happened. This module records the SHA-256 of
//! each file *as gwz last wrote it* in a side-car marker under `gwz.conf/markers/`, and
//! classifies the workspace against it.
//!
//! **This module only classifies; it never refuses.** [`inspect_conf_integrity`] returns a
//! [`ConfIntegrityVerdict`] and cannot fail. The decision — refuse, reconcile, or ignore —
//! belongs to the command gate in `workspace_ops::workspace_bootstrap::conf_gate`, which
//! runs at structural-mutation call sites and needs a Git backend that this layer does not
//! have. An earlier design gated `read_manifest` instead; that is the seam the merge lane
//! reads through mid-rewrite, and it displaced that lane's own errors, so it was abandoned.
//!
//! Two properties shape every decision here:
//!
//! * **Git compatibility.** The marker is committed alongside the conf files (the existing
//!   `gwz.conf` staging sweep takes the whole directory), so a `git pull`, checkout, or
//!   branch switch rewrites all three together and the digests still agree. When they do
//!   not, the gate — not this module — asks git whether the difference is committed, and
//!   only an uncommitted one is treated as a hand edit.
//! * **Blast radius.** A false positive bricks every structural command in the workspace,
//!   so every ambiguous state classifies as non-refusing: no marker (every pre-upgrade
//!   workspace), an unreadable or future-schema marker, a marker vouching for a file that
//!   is not on disk, a half-initialised workspace. Only a positive digest mismatch on a
//!   file that exists yields [`ConfIntegrityVerdict::Mismatch`].
//!
//! The accepted residuals of the whole layer — what it deliberately does not catch — are
//! documented on `conf_gate`, which is where the trade is actually made.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::{WORKSPACE_DIR, WORKSPACE_MANIFEST};

use super::LOCK_PATH;

/// Workspace-relative location of the conf-integrity marker.
///
/// It lives beside the commit markers so the existing `gwz.conf` staging sweep commits it
/// in the same commit as the files it vouches for — that is what makes `git pull` and
/// branch switches pass. The `.yml` extension (deliberately not `.yaml`) keeps it out of
/// [`super::list_markers`], which reads every `*.yaml` here as a commit marker.
pub const CONF_INTEGRITY_MARKER_PATH: &str = "gwz.conf/markers/conf-integrity.yml";

/// Schema tag of the marker document. An unrecognised tag is treated as unreadable
/// (adopt, do not refuse) so a future gwz cannot brick an older one.
pub const CONF_INTEGRITY_SCHEMA: &str = "gwz.conf-integrity/v0";

/// The machine-written files the marker vouches for, in a stable order.
pub const GUARDED_CONF_PATHS: [&str; 2] = [WORKSPACE_MANIFEST, LOCK_PATH];

/// The comment banner prepended to every `gwz.yml` gwz writes.
///
/// YAML treats comments as transparent, so readers are unaffected; the banner is part of
/// the file's bytes and therefore part of the digest recorded below.
///
/// `gwz.lock.yml` deliberately does NOT carry it: the merge lane re-renders an accepted
/// lock through a YAML value round trip that drops comments, then requires the result to
/// be byte-identical to the baseline it read off disk. A banner there breaks that
/// invariant for every no-op root merge. Banner-ing the lock needs a one-line change in
/// `merge/acceptance/v1/support.rs::render_complete_lock` first.
pub const CONF_BANNER: &str = "\
# Machine-managed by gwz. Hand edits to gwz.conf/ are detected and refused.
# Structural changes: `gwz repo <add|clone|create|detach|attach|sync>`.
# Already edited? Revert it, or `gwz init --update --force` to accept this state.
";

const MARKER_BANNER: &str = "\
# Machine-managed by gwz: SHA-256 of the conf files as gwz last wrote them.
# Commit this together with gwz.yml and gwz.lock.yml — git must move all three.
";

/// What a conf-integrity check concluded. Only [`Self::Mismatch`] refuses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfIntegrityVerdict {
    /// Every digest the marker records matches the file on disk.
    Verified,
    /// No marker, or one that vouches for nothing yet: a workspace created before conf
    /// integrity existed, or one mid-initialisation. Grandfathered.
    NotEnrolled,
    /// A marker exists but cannot be read as one — truncated, conflict-marked by a git
    /// merge, or written by a newer gwz. Ambiguous, so it passes.
    MarkerUnreadable(String),
    /// Positive drift: these workspace-relative paths exist and no longer hash to what
    /// the marker recorded. This is the only refusing verdict.
    Mismatch(Vec<String>),
}

impl ConfIntegrityVerdict {
    /// Whether this verdict blocks the workspace load.
    pub fn refuses(&self) -> bool {
        matches!(self, Self::Mismatch(_))
    }

    /// Advisory text for a caller that has an output channel.
    ///
    /// A [`Self::Mismatch`] has none: it is either refused with [`conf_hand_edit_error`]
    /// or reconciled by the gate, never merely reported.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Verified | Self::NotEnrolled | Self::Mismatch(_) => None,
            Self::MarkerUnreadable(reason) => Some(format!(
                "{CONF_INTEGRITY_MARKER_PATH} could not be read ({reason}); \
                 gwz.conf hand-edit detection is inactive until gwz rewrites it"
            )),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfIntegrityMarker {
    schema: String,
    files: BTreeMap<String, String>,
}

/// Classify the workspace's conf files against the marker. Never fails: an I/O or parse
/// problem is itself one of the non-refusing verdicts.
pub fn inspect_conf_integrity(root: &Path) -> ConfIntegrityVerdict {
    let marker = match read_marker(root) {
        Ok(None) => return ConfIntegrityVerdict::NotEnrolled,
        Ok(Some(marker)) => marker,
        Err(reason) => return ConfIntegrityVerdict::MarkerUnreadable(reason),
    };
    if marker.schema != CONF_INTEGRITY_SCHEMA {
        return ConfIntegrityVerdict::MarkerUnreadable(format!(
            "unsupported schema '{}'",
            marker.schema
        ));
    }

    let mut compared = 0usize;
    let mut drifted = Vec::new();
    for relative in GUARDED_CONF_PATHS {
        // No recorded digest: the marker makes no claim about this file (a workspace
        // enrolled before the lock existed). Nothing to contradict.
        let Some(recorded) = marker.files.get(relative) else {
            continue;
        };
        // Absent or unreadable on disk: ambiguous, and a deleted lock is legitimately
        // rebuildable. Refusing here would brick the workspace over a missing file.
        let Ok(Some(actual)) = file_digest(&root.join(relative)) else {
            continue;
        };
        compared += 1;
        if !actual.eq_ignore_ascii_case(recorded) {
            drifted.push(relative.to_owned());
        }
    }

    if !drifted.is_empty() {
        ConfIntegrityVerdict::Mismatch(drifted)
    } else if compared == 0 {
        ConfIntegrityVerdict::NotEnrolled
    } else {
        ConfIntegrityVerdict::Verified
    }
}

/// The refusal a caller raises once it has established the drift is a hand edit: what was
/// edited, why gwz cares, the sanctioned verbs, and the two ways out.
pub fn conf_hand_edit_error(paths: &[String]) -> ModelError {
    hand_edit_refusal(paths)
}

/// Record the conf files as they currently sit on disk.
///
/// This is both the grandfathering adoption point and the sanctioned acceptance path: it
/// blesses exactly the bytes that are there now. Writing no marker at all when neither
/// conf file exists keeps a bare directory from sprouting one.
pub fn refresh_conf_integrity_marker(root: &Path) -> ModelResult<()> {
    let mut files = BTreeMap::new();
    for relative in GUARDED_CONF_PATHS {
        if let Some(digest) = file_digest(&root.join(relative))? {
            files.insert((*relative).to_owned(), digest);
        }
    }
    if files.is_empty() {
        return Ok(());
    }
    let marker = ConfIntegrityMarker {
        schema: CONF_INTEGRITY_SCHEMA.to_owned(),
        files,
    };
    let yaml = serde_yaml::to_string(&marker).map_err(|err| {
        ModelError::new(
            ErrorCode::InternalError,
            format!("failed to serialize the conf-integrity marker: {err}"),
        )
    })?;
    super::write_atomic(
        &root.join(CONF_INTEGRITY_MARKER_PATH),
        format!("{MARKER_BANNER}{yaml}"),
    )
}

fn read_marker(root: &Path) -> Result<Option<ConfIntegrityMarker>, String> {
    let text = match fs::read_to_string(root.join(CONF_INTEGRITY_MARKER_PATH)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    serde_yaml::from_str(&text)
        .map(Some)
        .map_err(|err| err.to_string())
}

fn file_digest(path: &Path) -> ModelResult<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(format!("sha256:{}", sha256_hex(&bytes)))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ModelError::new(ErrorCode::IoError, error.to_string())),
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The teaching refusal: what was edited, why gwz cares, the sanctioned verbs, and the
/// two ways out.
fn hand_edit_refusal(paths: &[String]) -> ModelError {
    ModelError::new(
        ErrorCode::PermissionDenied,
        format!(
            "hand edits detected in {}: the bytes no longer match the digest gwz recorded when it \
             last wrote the file. {WORKSPACE_DIR}/ is machine-managed — {WORKSPACE_MANIFEST} and \
             {LOCK_PATH} must stay synchronized with the filesystem, so gwz writes them and \
             nothing else may. Structural changes go through \
             `gwz repo <add|clone|create|detach|attach|sync>`; there is no rename or move verb, so \
             relocate a member with `gwz repo detach` and then re-add it at the new path. \
             Recovery: revert the hand edit (`git checkout -- {WORKSPACE_DIR}`), or accept the \
             current on-disk state with `gwz init --update --force`.",
            paths.join(", "),
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::artifact::tests::{TempDir, sample_lock, sample_manifest};
    use crate::artifact::{write_lock, write_manifest};

    fn enrolled_workspace(name: &str) -> TempDir {
        let temp = TempDir::new(name);
        write_manifest(temp.path(), &sample_manifest()).unwrap();
        write_lock(temp.path(), &sample_lock()).unwrap();
        temp
    }

    #[test]
    fn conf_writes_enroll_the_workspace_and_verify_afterwards() {
        let temp = enrolled_workspace("integrity-enrol");

        assert!(temp.path().join(CONF_INTEGRITY_MARKER_PATH).is_file());
        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Verified
        );
        assert!(!inspect_conf_integrity(temp.path()).refuses());
    }

    #[test]
    fn hand_editing_the_manifest_refuses_with_a_teaching_error() {
        let temp = enrolled_workspace("integrity-manifest-edit");
        let manifest_path = temp.path().join(WORKSPACE_MANIFEST);
        let edited = fs::read_to_string(&manifest_path)
            .unwrap()
            .replace("repos/example", "repos/moved");
        fs::write(&manifest_path, edited).unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Mismatch(vec![WORKSPACE_MANIFEST.to_owned()])
        );
        let error = conf_hand_edit_error(&[WORKSPACE_MANIFEST.to_owned()]);
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        // The refusal has to teach: the file, the policy, the verbs, both recoveries.
        assert!(error.message.contains(WORKSPACE_MANIFEST));
        assert!(error.message.contains("machine-managed"));
        assert!(
            error
                .message
                .contains("`gwz repo <add|clone|create|detach|attach|sync>`")
        );
        assert!(error.message.contains("git checkout -- gwz.conf"));
        assert!(error.message.contains("gwz init --update --force"));
    }

    #[test]
    fn hand_editing_the_lock_refuses_and_names_the_lock() {
        let temp = enrolled_workspace("integrity-lock-edit");
        fs::write(
            temp.path().join(LOCK_PATH),
            fs::read_to_string(temp.path().join(LOCK_PATH))
                .unwrap()
                .replace("abc123", "def456"),
        )
        .unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Mismatch(vec![LOCK_PATH.to_owned()])
        );
        assert!(
            conf_hand_edit_error(&[LOCK_PATH.to_owned()])
                .message
                .contains(LOCK_PATH)
        );
    }

    #[test]
    fn tampering_with_the_marker_alone_shows_as_drift_on_both_files() {
        // The marker moving without the files is drift on every file it vouches for, so
        // this layer classifies both as mismatched.
        //
        // It is NOT, by itself, caught: the conf files still match HEAD, so the gate
        // cannot tell this from git having moved them and repairs the marker instead.
        // `conf_gate::a_tampered_marker_alone_is_repaired_because_the_files_match_head`
        // asserts that shipped outcome, and the accepted residual is documented there.
        let temp = enrolled_workspace("integrity-marker-edit");
        let marker_path = temp.path().join(CONF_INTEGRITY_MARKER_PATH);
        let tampered = fs::read_to_string(&marker_path)
            .unwrap()
            .replace("sha256:", "sha256:0");
        fs::write(&marker_path, tampered).unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Mismatch(vec![
                WORKSPACE_MANIFEST.to_owned(),
                LOCK_PATH.to_owned()
            ])
        );
    }

    #[test]
    fn a_consistent_git_style_rewrite_of_files_and_marker_passes() {
        // What `git pull` / checkout / branch-switch does: all three files land together.
        let temp = enrolled_workspace("integrity-git-move");
        let mut moved = sample_manifest();
        moved.members[0].path = "repos/relocated".to_owned();
        let manifest_yaml = moved.to_yaml().unwrap();
        let lock_yaml = sample_lock().to_yaml().unwrap();
        let marker = format!(
            "{MARKER_BANNER}schema: {CONF_INTEGRITY_SCHEMA}\nfiles:\n  {WORKSPACE_MANIFEST}: sha256:{}\n  {LOCK_PATH}: sha256:{}\n",
            sha256_hex(manifest_yaml.as_bytes()),
            sha256_hex(lock_yaml.as_bytes()),
        );

        fs::write(temp.path().join(WORKSPACE_MANIFEST), &manifest_yaml).unwrap();
        fs::write(temp.path().join(LOCK_PATH), &lock_yaml).unwrap();
        fs::write(temp.path().join(CONF_INTEGRITY_MARKER_PATH), marker).unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Verified
        );
        assert!(!inspect_conf_integrity(temp.path()).refuses());
    }

    #[test]
    fn a_workspace_with_no_marker_is_grandfathered_not_refused() {
        // Every pre-upgrade workspace, including the operator's real ones.
        let temp = TempDir::new("integrity-grandfather");
        fs::create_dir_all(temp.path().join(WORKSPACE_DIR)).unwrap();
        fs::write(
            temp.path().join(WORKSPACE_MANIFEST),
            sample_manifest().to_yaml().unwrap(),
        )
        .unwrap();
        fs::write(
            temp.path().join(LOCK_PATH),
            sample_lock().to_yaml().unwrap(),
        )
        .unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::NotEnrolled
        );
        assert!(!inspect_conf_integrity(temp.path()).refuses());
        assert!(inspect_conf_integrity(temp.path()).warning().is_none());
    }

    #[test]
    fn an_unreadable_or_future_schema_marker_warns_but_never_refuses() {
        let temp = enrolled_workspace("integrity-unreadable");
        let marker_path = temp.path().join(CONF_INTEGRITY_MARKER_PATH);

        // A git merge that conflicted inside the marker.
        fs::write(&marker_path, "<<<<<<< HEAD\nschema: [\n=======\n").unwrap();
        let verdict = inspect_conf_integrity(temp.path());
        assert!(matches!(verdict, ConfIntegrityVerdict::MarkerUnreadable(_)));
        assert!(!verdict.refuses());
        assert!(verdict.warning().is_some());
        assert!(!inspect_conf_integrity(temp.path()).refuses());

        // A marker written by a newer gwz.
        fs::write(&marker_path, "schema: gwz.conf-integrity/v9\nfiles: {}\n").unwrap();
        assert!(matches!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::MarkerUnreadable(_)
        ));
        assert!(!inspect_conf_integrity(temp.path()).refuses());
    }

    #[test]
    fn a_marker_vouching_for_files_that_are_absent_does_not_refuse() {
        // Half-initialised workspace, or a lock deleted before `gwz materialize --lock`.
        let temp = enrolled_workspace("integrity-absent");
        fs::remove_file(temp.path().join(LOCK_PATH)).unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Verified
        );
        assert!(!inspect_conf_integrity(temp.path()).refuses());

        fs::remove_file(temp.path().join(WORKSPACE_MANIFEST)).unwrap();
        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::NotEnrolled
        );
        assert!(!inspect_conf_integrity(temp.path()).refuses());
    }

    #[test]
    fn a_manifest_only_workspace_enrols_and_stays_verified_when_the_lock_arrives() {
        // The init ordering: manifest first, lock second. Neither step may refuse.
        let temp = TempDir::new("integrity-partial-init");
        write_manifest(temp.path(), &sample_manifest()).unwrap();
        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Verified
        );

        write_lock(temp.path(), &sample_lock()).unwrap();
        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Verified
        );
    }

    #[test]
    fn refresh_accepts_the_current_on_disk_state() {
        // The `gwz init --update --force` core: bless what is there now.
        let temp = enrolled_workspace("integrity-accept");
        let manifest_path = temp.path().join(WORKSPACE_MANIFEST);
        fs::write(
            &manifest_path,
            fs::read_to_string(&manifest_path)
                .unwrap()
                .replace("repos/example", "repos/accepted"),
        )
        .unwrap();
        assert!(inspect_conf_integrity(temp.path()).refuses());

        refresh_conf_integrity_marker(temp.path()).unwrap();

        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::Verified
        );
    }

    #[test]
    fn refresh_writes_nothing_when_no_conf_file_exists() {
        let temp = TempDir::new("integrity-empty");

        refresh_conf_integrity_marker(temp.path()).unwrap();

        assert!(!temp.path().join(CONF_INTEGRITY_MARKER_PATH).exists());
        assert_eq!(
            inspect_conf_integrity(temp.path()),
            ConfIntegrityVerdict::NotEnrolled
        );
    }

    #[test]
    fn the_marker_is_not_read_as_a_commit_marker() {
        // `list_markers` parses every *.yaml in gwz.conf/markers; a .yml side-car must
        // stay invisible to it or every marker listing would fail.
        let temp = enrolled_workspace("integrity-list-markers");

        assert!(super::super::list_markers(temp.path()).unwrap().is_empty());
    }
}
