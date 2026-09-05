//! Pure family model for the GWZ local clone family.
//!
//! A *family* is one original workspace (`root`) plus its named local
//! clones. This crate owns the deterministic values and decisions of that
//! model: names, ids, rows, the frozen metadata format (format 1), the
//! observed-state vocabulary, the one remote-token resolver shared by
//! `merge`, `pull` and `push`, and the index transitions. It performs no
//! I/O, holds no lock and repairs nothing; `gwz-family-store` reads and
//! writes the files, and core supplies observations.
//!
//! Product contract: gwz-dev `dev-docs/GwzLocalCloneDesign.md` §2, §3, §6
//! (revision 8); boundary: `GwzLocalCloneLibraryBoundaries.md` §3.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

mod resolve;
mod transition;

pub use resolve::{BoundMember, RemoteToken, Resolution, Verb, resolve_remote_token};
pub use transition::{FamilyChange, Refusal, RemovalReason, ValidatedChange, validate_transition};

/// `schema:` value of the root index (`.gwz/local-family.yml`), format 1.
pub const INDEX_SCHEMA: &str = "gwz.local-family/v1";
/// `schema:` value of a clone pointer (`.gwz/family-root`), format 1.
pub const POINTER_SCHEMA: &str = "gwz.family-root/v1";
/// Root-relative path of the family index; exists only at the root.
pub const INDEX_RELATIVE_PATH: &str = ".gwz/local-family.yml";
/// Root-relative path of the advisory family lock; exists only at the root.
pub const LOCK_RELATIVE_PATH: &str = ".gwz/local-family.lock";
/// Workspace-relative path of a clone's pointer to its registering root.
pub const POINTER_RELATIVE_PATH: &str = ".gwz/family-root";
/// Workspace-relative path of a clone's allocation marker.
pub const ALLOCATION_MARKER_RELATIVE_PATH: &str = ".gwz/local-clone-allocation";
/// Largest encoded index the store accepts; larger refuses before mutation.
pub const MAX_ENCODED_INDEX_BYTES: u64 = 1024 * 1024;
/// The original workspace's own name; never a clone name.
pub const ROOT_NAME: &str = "root";
/// The root's own root-relative path.
pub const ROOT_PATH: &str = ".";
/// Names a clone may never take (design §2).
pub const RESERVED_NAMES: [&str; 4] = ["root", "origin", "HEAD", "FETCH_HEAD"];

/// Field names of the frozen format-1 index and pointer files. The store
/// encodes and decodes with exactly these keys.
pub mod fields {
    pub const SCHEMA: &str = "schema";
    pub const FAMILY_ID: &str = "family_id";
    pub const ROOT: &str = "root";
    pub const ROOT_PATH: &str = "root_path";
    pub const MEMBERS: &str = "members";
    pub const PATH: &str = "path";
    pub const KIND: &str = "kind";
    pub const STATE: &str = "state";
    pub const ALLOCATION_ID: &str = "allocation_id";
    pub const SOURCE_PATH: &str = "source_path";
    pub const MODE: &str = "mode";
    pub const LAST_ERROR: &str = "last_error";
}

/// A validated clone name: non-empty, not reserved, no `/` or `:`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberName(String);

impl MemberName {
    pub fn parse(name: &str) -> Result<Self, NameError> {
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if let Some(separator) = name.chars().find(|c| matches!(c, '/' | ':')) {
            return Err(NameError::Separator(separator));
        }
        if RESERVED_NAMES.contains(&name) {
            return Err(NameError::Reserved(name.to_owned()));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemberName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Reserved(String),
    Separator(char),
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a clone name must not be empty"),
            Self::Reserved(name) => write!(f, "`{name}` is a reserved name"),
            Self::Separator(c) => write!(f, "a clone name must not contain `{c}`"),
        }
    }
}

impl std::error::Error for NameError {}

/// The core-minted family identity. Never a request input.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FamilyId(String);

impl FamilyId {
    pub fn new(id: impl Into<String>) -> Result<Self, IdError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(IdError::Empty);
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An ordinary allocation marker value for one destination. It catches
/// ordinary mix-ups; it is not a durable filesystem identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AllocationId(String);

impl AllocationId {
    pub fn new(id: impl Into<String>) -> Result<Self, IdError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(IdError::Empty);
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AllocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdError {
    Empty,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an id must not be empty")
    }
}

impl std::error::Error for IdError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Checkout,
    Bare,
}

impl MemberKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Checkout => "checkout",
            Self::Bare => "bare",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "checkout" => Some(Self::Checkout),
            "bare" => Some(Self::Bare),
            _ => None,
        }
    }
}

/// A row's recorded lifecycle state. Only `ready` rows are family endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberState {
    Creating,
    Ready,
    Disposing,
}

impl MemberState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Disposing => "disposing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "creating" => Some(Self::Creating),
            "ready" => Some(Self::Ready),
            "disposing" => Some(Self::Disposing),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneMode {
    Verbatim,
    Clean,
    Bare,
}

impl CloneMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verbatim => "verbatim",
            Self::Clean => "clean",
            Self::Bare => "bare",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "verbatim" => Some(Self::Verbatim),
            "clean" => Some(Self::Clean),
            "bare" => Some(Self::Bare),
            _ => None,
        }
    }
}

/// One clone's row in the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    /// Root-relative path of the clone (for example `../gwz-dev-A`).
    pub path: String,
    pub kind: MemberKind,
    pub state: MemberState,
    pub allocation_id: AllocationId,
    /// Root-relative path of the member the clone was created from
    /// (`.` for the root).
    pub source_path: String,
    pub mode: CloneMode,
    /// Diagnostic recorded when the row was left incomplete.
    pub last_error: Option<String>,
}

/// The root's own entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootEntry {
    pub allocation_id: AllocationId,
}

/// A valid, decoded family index. Constructing one asserts nothing about
/// the filesystem; the store validated the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyView {
    pub family_id: FamilyId,
    pub root: RootEntry,
    pub members: BTreeMap<MemberName, MemberRow>,
}

impl FamilyView {
    /// The index of a family that has just been founded: no clones yet.
    pub fn founded(family_id: FamilyId, root_allocation: AllocationId) -> Self {
        Self {
            family_id,
            root: RootEntry {
                allocation_id: root_allocation,
            },
            members: BTreeMap::new(),
        }
    }

    pub fn member(&self, name: &str) -> Option<(&MemberName, &MemberRow)> {
        self.members.iter().find(|(key, _)| key.as_str() == name)
    }
}

/// What core observed at a recorded clone path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetObservation {
    Present {
        pointer: PointerObservation,
        marker: MarkerObservation,
    },
    Missing,
    Malformed {
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerObservation {
    /// The pointer names this family and this root.
    Matches,
    Absent,
    OtherFamily,
    /// The path holds an index, so it is a root, not a clone.
    IsIndex,
    Malformed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerObservation {
    Matches,
    Absent,
    Mismatch,
    Malformed,
}

/// The observation-only state `gwz local list` reports for a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListState {
    /// Ready row, matching pointer and marker: a usable family member.
    Ready,
    /// Creating row, present or not: incomplete; never auto-promoted.
    Incomplete,
    /// Disposing row with the directory still present: interrupted deletion.
    InterruptedDisposal,
    /// Row present, target absent; only an explicit dispose removes the row.
    Missing,
    /// Present, but pointer or marker disagree with the row.
    Mismatched,
    /// Present, but its metadata could not be decoded.
    Malformed,
    /// The caller supplied no observation for this row.
    Unobserved,
}

/// Classify one recorded row against its observed target. Pure; never
/// repairs, promotes or deletes.
pub fn classify_target(row: &MemberRow, target: Option<&TargetObservation>) -> ListState {
    match target {
        None => ListState::Unobserved,
        Some(TargetObservation::Missing) => ListState::Missing,
        Some(TargetObservation::Malformed { .. }) => ListState::Malformed,
        Some(TargetObservation::Present { pointer, marker }) => match row.state {
            MemberState::Creating => ListState::Incomplete,
            MemberState::Disposing => ListState::InterruptedDisposal,
            MemberState::Ready => {
                if *pointer == PointerObservation::Matches && *marker == MarkerObservation::Matches
                {
                    ListState::Ready
                } else {
                    ListState::Mismatched
                }
            }
        },
    }
}

/// One line of the observation-only listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub name: String,
    pub kind: MemberKind,
    pub recorded: MemberState,
    pub observed: ListState,
    pub path: String,
    pub last_error: Option<String>,
}

/// Project the listing: the root first, then every row in name order.
pub fn project_list(
    view: &FamilyView,
    observations: &BTreeMap<MemberName, TargetObservation>,
) -> Vec<ListRow> {
    let mut rows = vec![ListRow {
        name: ROOT_NAME.to_owned(),
        kind: MemberKind::Checkout,
        recorded: MemberState::Ready,
        observed: ListState::Ready,
        path: ROOT_PATH.to_owned(),
        last_error: None,
    }];
    for (name, row) in &view.members {
        rows.push(ListRow {
            name: name.as_str().to_owned(),
            kind: row.kind,
            recorded: row.state,
            observed: classify_target(row, observations.get(name)),
            path: row.path.clone(),
            last_error: row.last_error.clone(),
        });
    }
    rows
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) fn row(path: &str, state: MemberState) -> MemberRow {
        MemberRow {
            path: path.to_owned(),
            kind: MemberKind::Checkout,
            state,
            allocation_id: AllocationId::new(format!("alloc-{path}")).unwrap(),
            source_path: ROOT_PATH.to_owned(),
            mode: CloneMode::Verbatim,
            last_error: None,
        }
    }

    /// A ready `A`, a creating `B` and a disposing `C`.
    pub(crate) fn view() -> FamilyView {
        let mut view = FamilyView::founded(
            FamilyId::new("fam_test").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        );
        view.members.insert(
            MemberName::parse("A").unwrap(),
            row("../ws-A", MemberState::Ready),
        );
        view.members.insert(
            MemberName::parse("B").unwrap(),
            row("../ws-B", MemberState::Creating),
        );
        view.members.insert(
            MemberName::parse("C").unwrap(),
            row("../ws-C", MemberState::Disposing),
        );
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_names_reject_reserved_empty_and_separators() {
        assert_eq!(MemberName::parse("").unwrap_err(), NameError::Empty);
        for reserved in RESERVED_NAMES {
            assert_eq!(
                MemberName::parse(reserved).unwrap_err(),
                NameError::Reserved(reserved.to_owned())
            );
        }
        assert_eq!(
            MemberName::parse("a/b").unwrap_err(),
            NameError::Separator('/')
        );
        assert_eq!(
            MemberName::parse("a:b").unwrap_err(),
            NameError::Separator(':')
        );
        assert_eq!(MemberName::parse("lane-17").unwrap().as_str(), "lane-17");
        assert_eq!(MemberName::parse("Root").unwrap().as_str(), "Root");
    }

    #[test]
    fn ids_must_not_be_empty() {
        assert_eq!(FamilyId::new("  ").unwrap_err(), IdError::Empty);
        assert_eq!(AllocationId::new("").unwrap_err(), IdError::Empty);
        assert_eq!(FamilyId::new("fam_1").unwrap().as_str(), "fam_1");
    }

    #[test]
    fn enums_round_trip_their_frozen_spellings() {
        for kind in [MemberKind::Checkout, MemberKind::Bare] {
            assert_eq!(MemberKind::parse(kind.as_str()), Some(kind));
        }
        for state in [
            MemberState::Creating,
            MemberState::Ready,
            MemberState::Disposing,
        ] {
            assert_eq!(MemberState::parse(state.as_str()), Some(state));
        }
        for mode in [CloneMode::Verbatim, CloneMode::Clean, CloneMode::Bare] {
            assert_eq!(CloneMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(MemberState::parse("done"), None);
        assert_eq!(INDEX_SCHEMA, "gwz.local-family/v1");
        assert_eq!(MAX_ENCODED_INDEX_BYTES, 1_048_576);
    }

    #[test]
    fn list_projection_reports_observed_state_without_promotion() {
        let view = fixtures::view();
        let mut observations = BTreeMap::new();
        observations.insert(
            MemberName::parse("A").unwrap(),
            TargetObservation::Present {
                pointer: PointerObservation::Matches,
                marker: MarkerObservation::Matches,
            },
        );
        observations.insert(
            MemberName::parse("B").unwrap(),
            TargetObservation::Present {
                pointer: PointerObservation::Matches,
                marker: MarkerObservation::Matches,
            },
        );
        observations.insert(MemberName::parse("C").unwrap(), TargetObservation::Missing);
        let rows = project_list(&view, &observations);
        assert_eq!(rows[0].name, "root");
        assert_eq!(rows[0].observed, ListState::Ready);
        assert_eq!(rows[1].observed, ListState::Ready);
        assert_eq!(
            rows[2].observed,
            ListState::Incomplete,
            "an apparently complete creating row stays incomplete"
        );
        assert_eq!(rows[3].observed, ListState::Missing);

        let ready = fixtures::row("../ws-A", MemberState::Ready);
        assert_eq!(
            classify_target(
                &ready,
                Some(&TargetObservation::Present {
                    pointer: PointerObservation::OtherFamily,
                    marker: MarkerObservation::Matches
                })
            ),
            ListState::Mismatched
        );
        assert_eq!(
            classify_target(
                &ready,
                Some(&TargetObservation::Malformed { detail: "x".into() })
            ),
            ListState::Malformed
        );
        assert_eq!(classify_target(&ready, None), ListState::Unobserved);
        let disposing = fixtures::row("../ws-C", MemberState::Disposing);
        assert_eq!(
            classify_target(
                &disposing,
                Some(&TargetObservation::Present {
                    pointer: PointerObservation::Matches,
                    marker: MarkerObservation::Matches
                })
            ),
            ListState::InterruptedDisposal
        );
    }
}
