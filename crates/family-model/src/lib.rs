//! Pure family model for the GWZ local clone family.
//!
//! A *family* is one original workspace (`root`) plus its named local
//! clones. This crate owns the deterministic values and decisions of that
//! model: names, root-relative member paths, ids, rows, the frozen
//! metadata format (format 1) and its size limit, the observed-state
//! vocabulary, the one remote-token resolver shared by
//! `merge`, `pull` and `push`, and the index transitions. It performs no
//! I/O, holds no lock and repairs nothing; `gwz-family-store` reads and
//! writes the files, and core supplies observations.
//!
//! What the model cannot decide, it does not pretend to: it never resolves
//! a host path, so equivalence that needs canonicalisation, symlink
//! resolution or a filesystem's case rules belongs to the store, as
//! [`validate_member_path`] states. Observations arrive as values;
//! nothing here goes looking.
//!
//! Product contract: gwz-dev `dev-docs/GwzLocalCloneDesign.md` §2, §3, §6
//! (revision 8); boundary: `GwzLocalCloneLibraryBoundaries.md` §3.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

mod name;
mod path;
mod resolve;
mod transition;

pub use name::{
    DisposeTarget, MemberName, NameError, RESERVED_DIRECTORY_NAMES, RESERVED_NAMES,
    classify_dispose_target, validate as validate_member_name,
};
pub use path::{
    MemberPath, PathError, PathRelation, normalize as normalize_member_path,
    relate as relate_member_paths, validate as validate_member_path,
};
pub use resolve::{BoundMember, RemoteToken, Resolution, Verb, resolve_remote_token};
pub use transition::{
    FamilyChange, Refusal, RemovalReason, ValidatedChange, check_allocation_available,
    check_name_available, check_path_available, validate_transition, validate_view,
};

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
/// Format version of the index and pointer files. It is the `/v1` in
/// [`INDEX_SCHEMA`] and [`POINTER_SCHEMA`]; a file carrying any other
/// version is not this format and refuses rather than being upgraded.
pub const INDEX_FORMAT_VERSION: u32 = 1;

/// The original workspace's own name; never a clone name.
pub const ROOT_NAME: &str = "root";
/// The root's own root-relative path.
pub const ROOT_PATH: &str = ".";

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

/// The encoded index is larger than the model admits (design §3, "maximum
/// encoded size 1 MiB; oversize or malformed input refuses before
/// mutation"). The store adds the path it read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexOversize {
    pub bytes: u64,
    pub limit: u64,
}

impl fmt::Display for IndexOversize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "encoded family index is {} bytes, over the {} byte limit",
            self.bytes, self.limit
        )
    }
}

impl std::error::Error for IndexOversize {}

/// Decide whether an encoded index of `bytes` may be read or written. The
/// limit is inclusive: exactly [`MAX_ENCODED_INDEX_BYTES`] is admitted.
pub fn check_encoded_size(bytes: u64) -> Result<(), IndexOversize> {
    if bytes > MAX_ENCODED_INDEX_BYTES {
        return Err(IndexOversize {
            bytes,
            limit: MAX_ENCODED_INDEX_BYTES,
        });
    }
    Ok(())
}

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
    /// Present and still carrying this row's allocation, but its pointer to
    /// the family is gone: an interrupted pointer-only detach or disband.
    /// An explicit repeat may finish removing the remaining pointers and
    /// rows; nothing here removes directory contents (design §3.1).
    PointerRemoved,
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
            // A `creating` row is incomplete whatever stands at its path:
            // its pointer may never have been written, and an apparently
            // complete tree is still not a family endpoint.
            MemberState::Creating => ListState::Incomplete,
            MemberState::Disposing => ListState::InterruptedDisposal,
            MemberState::Ready => match (pointer, marker) {
                (PointerObservation::Matches, MarkerObservation::Matches) => ListState::Ready,
                // Detach removes the pointer and marker of a ready member;
                // interrupted, it leaves the tree, the row and possibly the
                // marker. The pointer is the membership record, so only its
                // absence reads as a detach.
                (
                    PointerObservation::Absent,
                    MarkerObservation::Matches | MarkerObservation::Absent,
                ) => ListState::PointerRemoved,
                _ => ListState::Mismatched,
            },
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

    pub(crate) fn bare_row(path: &str) -> MemberRow {
        MemberRow {
            kind: MemberKind::Bare,
            mode: CloneMode::Bare,
            ..row(path, MemberState::Ready)
        }
    }

    /// A ready `A`, a creating `B`, a disposing `C` and a ready bare `hub`.
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
        view.members
            .insert(MemberName::parse("hub").unwrap(), bare_row("../ws-hub"));
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The format-1 file names and field keys are frozen (design §3): the
    /// store encodes and decodes with exactly these, so a rename here is a
    /// format change, not a refactor.
    #[test]
    fn the_frozen_format_1_files_and_fields_are_pinned() {
        assert_eq!(INDEX_SCHEMA, "gwz.local-family/v1");
        assert_eq!(POINTER_SCHEMA, "gwz.family-root/v1");
        assert_eq!(INDEX_RELATIVE_PATH, ".gwz/local-family.yml");
        assert_eq!(LOCK_RELATIVE_PATH, ".gwz/local-family.lock");
        assert_eq!(POINTER_RELATIVE_PATH, ".gwz/family-root");
        assert_eq!(
            ALLOCATION_MARKER_RELATIVE_PATH,
            ".gwz/local-clone-allocation"
        );
        assert_eq!(ROOT_NAME, "root");
        assert_eq!(ROOT_PATH, ".");
        assert_eq!(
            [
                fields::SCHEMA,
                fields::FAMILY_ID,
                fields::ROOT,
                fields::ROOT_PATH,
                fields::MEMBERS,
            ],
            ["schema", "family_id", "root", "root_path", "members"]
        );
        assert_eq!(
            [
                fields::PATH,
                fields::KIND,
                fields::STATE,
                fields::ALLOCATION_ID,
                fields::SOURCE_PATH,
                fields::MODE,
                fields::LAST_ERROR,
            ],
            [
                "path",
                "kind",
                "state",
                "allocation_id",
                "source_path",
                "mode",
                "last_error"
            ],
            "the conceptual row of design §3"
        );
    }

    #[test]
    fn the_index_size_decision_names_the_limit_it_enforces() {
        assert_eq!(INDEX_FORMAT_VERSION, 1);
        assert!(INDEX_SCHEMA.ends_with("/v1"));
        assert!(POINTER_SCHEMA.ends_with("/v1"));
        assert_eq!(check_encoded_size(0), Ok(()));
        assert_eq!(check_encoded_size(MAX_ENCODED_INDEX_BYTES), Ok(()));
        let refusal = check_encoded_size(MAX_ENCODED_INDEX_BYTES + 1).unwrap_err();
        assert_eq!(
            refusal,
            IndexOversize {
                bytes: MAX_ENCODED_INDEX_BYTES + 1,
                limit: MAX_ENCODED_INDEX_BYTES,
            }
        );
        assert!(refusal.to_string().contains("1048577"));
        assert!(refusal.to_string().contains("1048576"));
    }
}

#[cfg(test)]
mod list_table_tests {
    use super::*;

    fn present(pointer: PointerObservation, marker: MarkerObservation) -> TargetObservation {
        TargetObservation::Present { pointer, marker }
    }

    fn observed(state: MemberState, target: Option<TargetObservation>) -> ListState {
        classify_target(&fixtures::row("../ws-X", state), target.as_ref())
    }

    /// Design §3.1's observed-state table, one case per row, plus the
    /// combinations the table's wording leaves to this model.
    #[test]
    fn every_observed_state_row_has_one_list_state() {
        use MarkerObservation as Marker;
        use MemberState::{Creating, Disposing, Ready};
        use PointerObservation as Pointer;

        // `ready`, valid pointer/path -> use normally.
        assert_eq!(
            observed(Ready, Some(present(Pointer::Matches, Marker::Matches))),
            ListState::Ready
        );

        // `creating`, whether partial or apparently complete -> incomplete.
        for target in [
            present(Pointer::Matches, Marker::Matches),
            present(Pointer::Absent, Marker::Absent),
            present(Pointer::OtherFamily, Marker::Mismatch),
            TargetObservation::Malformed {
                detail: "half written".to_owned(),
            },
        ] {
            let malformed = matches!(target, TargetObservation::Malformed { .. });
            let state = observed(Creating, Some(target));
            assert_eq!(
                state,
                if malformed {
                    ListState::Malformed
                } else {
                    ListState::Incomplete
                },
                "a creating row is never promoted by what is observed at it"
            );
        }

        // `disposing`, target still present -> interrupted deletion.
        assert_eq!(
            observed(Disposing, Some(present(Pointer::Matches, Marker::Matches))),
            ListState::InterruptedDisposal
        );

        // row present, target absent -> missing, in every recorded state.
        for state in [Ready, Creating, Disposing] {
            assert_eq!(
                observed(state, Some(TargetObservation::Missing)),
                ListState::Missing,
                "{state:?}"
            );
        }

        // mismatched id, unexpected path, malformed metadata.
        for (pointer, marker) in [
            (Pointer::OtherFamily, Marker::Matches),
            (Pointer::IsIndex, Marker::Matches),
            (Pointer::Malformed, Marker::Matches),
            (Pointer::Matches, Marker::Mismatch),
            (Pointer::Matches, Marker::Malformed),
            (Pointer::Matches, Marker::Absent),
            (Pointer::Absent, Marker::Mismatch),
        ] {
            assert_eq!(
                observed(Ready, Some(present(pointer, marker))),
                ListState::Mismatched,
                "{pointer:?}/{marker:?}"
            );
        }
        assert_eq!(
            observed(
                Ready,
                Some(TargetObservation::Malformed {
                    detail: "bad yaml".to_owned()
                })
            ),
            ListState::Malformed
        );

        // interrupted pointer-only detach/disband: the tree and the row
        // stand, the pointer to this family does not.
        for marker in [Marker::Matches, Marker::Absent] {
            assert_eq!(
                observed(Ready, Some(present(Pointer::Absent, marker))),
                ListState::PointerRemoved,
                "{marker:?}"
            );
        }

        // No observation supplied is its own answer, never a guess.
        assert_eq!(observed(Ready, None), ListState::Unobserved);
    }

    /// The projection reports; it never promotes, repairs or removes.
    #[test]
    fn the_projection_is_a_pure_reading_of_the_index() {
        let view = fixtures::view();
        let before = view.clone();
        let mut observations = BTreeMap::new();
        observations.insert(
            MemberName::parse("hub").unwrap(),
            present(PointerObservation::Absent, MarkerObservation::Matches),
        );
        observations.insert(
            MemberName::parse("B").unwrap(),
            present(PointerObservation::Matches, MarkerObservation::Matches),
        );
        let rows = project_list(&view, &observations);
        assert_eq!(view, before, "projecting changes nothing");
        assert_eq!(rows, project_list(&view, &observations), "and repeats");

        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, vec!["root", "A", "B", "C", "hub"]);
        assert_eq!(rows[0].path, ROOT_PATH);
        assert_eq!(rows[0].kind, MemberKind::Checkout);
        assert_eq!(rows[0].observed, ListState::Ready);

        let hub = rows.last().unwrap();
        assert_eq!(hub.kind, MemberKind::Bare, "a bare row lists as bare");
        assert_eq!(hub.recorded, MemberState::Ready);
        assert_eq!(
            hub.observed,
            ListState::PointerRemoved,
            "an interrupted detach is reported, not finished"
        );
        assert_eq!(
            rows[2].recorded,
            MemberState::Creating,
            "the recorded state is reported beside the observation"
        );
        assert_eq!(rows[2].observed, ListState::Incomplete);
        assert_eq!(rows[3].observed, ListState::Unobserved);
    }
}
