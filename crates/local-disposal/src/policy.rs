//! What the caller asked for: the disposal policy, its parse error and the
//! validated request the service runs.

use std::fmt;
use std::path::PathBuf;

use gwz_family_model::MemberName;

use crate::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposePolicy {
    /// Detach the row and pointer; retain every file.
    Keep,
    /// One-shot removal after fresh checks, with explicit waivers.
    Delete { waivers: Vec<HazardWaiver> },
}

impl DisposePolicy {
    /// Decide the policy from the wire pair `(keep, force_hazards)`, so the
    /// three request-shape refusals of design §5.2 live with the vocabulary
    /// that defines them rather than being restated by each driver:
    /// an unknown or empty **hazard name** refuses ([`HazardWaiver::parse_all`];
    /// an empty *list* is simply no force), and `--keep` with any force name
    /// refuses because keep removes no file and so waives nothing.
    pub fn parse(keep: bool, force_hazards: &[String]) -> Result<Self, PolicyError> {
        if keep {
            if !force_hazards.is_empty() {
                return Err(PolicyError::KeepWithForce {
                    names: force_hazards.to_vec(),
                });
            }
            return Ok(Self::Keep);
        }
        Ok(Self::Delete {
            waivers: HazardWaiver::parse_all(force_hazards).map_err(PolicyError::UnknownHazard)?,
        })
    }
}

/// Why a `(keep, force_hazards)` pair is not a policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    UnknownHazard(UnknownHazard),
    KeepWithForce { names: Vec<String> },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHazard(error) => error.fmt(f),
            Self::KeepWithForce { names } => write!(
                f,
                "--keep and --force <hazards> are mutually exclusive; keep removes no file and \
                 waives nothing (got `{}`)",
                names.join(",")
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeRequest {
    pub name: MemberName,
    pub policy: DisposePolicy,
    /// The registering root.
    ///
    /// **Canonical absolute paths.** This library compares `root`, `cwd`,
    /// the row's recorded path and every observed repository path
    /// *lexically* (it is pure: it opens nothing and resolves no symlink),
    /// exactly as the family-store contract's reference resolution does. The
    /// root/cwd/overlap guards below are therefore only as exact as the
    /// spellings core supplies: core must pass canonicalised absolute paths,
    /// or a symlinked spelling could make an overlapping target look
    /// disjoint.
    pub root: PathBuf,
    /// The invoking process's working directory; disposal refuses a target
    /// that contains it.
    pub cwd: PathBuf,
}
