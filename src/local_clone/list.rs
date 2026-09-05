//! The `gwz local list` projection (design §3.1, §7, §8.1, §11 item 12;
//! operator ruling 2026-09-05): the pure model's `ListRow`s onto the
//! `LocalFamilyResponse.members` wire payload.
//!
//! Observation-only, like the model it mirrors: nothing here writes, repairs,
//! promotes or deletes. Each wire enum mirrors its model enum one-for-one in
//! declaration order (`LocalMemberKind` <- `MemberKind`, `LocalMemberState`
//! <- `MemberState`, `LocalObservedState` <- `ListState`), and each match
//! below is exhaustive with no wildcard, so a model variant that gains no
//! wire value fails to compile here instead of being folded into a
//! neighbour.

use std::collections::BTreeMap;
use std::path::Path;

use gwz_family_model::{
    FamilyView, ListRow, ListState, MemberKind, MemberName, MemberState, TargetObservation,
    project_list,
};
use gwz_family_store_contract::FamilyObservation;

use super::errors::unsupported;
use crate::model::ModelResult;

pub fn member_kind(kind: MemberKind) -> crate::LocalMemberKind {
    match kind {
        MemberKind::Checkout => crate::LocalMemberKind::Checkout,
        MemberKind::Bare => crate::LocalMemberKind::Bare,
    }
}

pub fn member_state(state: MemberState) -> crate::LocalMemberState {
    match state {
        MemberState::Creating => crate::LocalMemberState::Creating,
        MemberState::Ready => crate::LocalMemberState::Ready,
        MemberState::Disposing => crate::LocalMemberState::Disposing,
    }
}

pub fn observed_state(state: ListState) -> crate::LocalObservedState {
    match state {
        ListState::Ready => crate::LocalObservedState::Ready,
        ListState::Incomplete => crate::LocalObservedState::Incomplete,
        ListState::InterruptedDisposal => crate::LocalObservedState::InterruptedDisposal,
        ListState::Missing => crate::LocalObservedState::Missing,
        ListState::PointerRemoved => crate::LocalObservedState::PointerRemoved,
        ListState::Mismatched => crate::LocalObservedState::Mismatched,
        ListState::Malformed => crate::LocalObservedState::Malformed,
        ListState::Unobserved => crate::LocalObservedState::Unobserved,
    }
}

/// One projected row, field for field.
pub fn entry(row: &ListRow) -> crate::LocalFamilyMemberEntry {
    crate::LocalFamilyMemberEntry {
        name: row.name.clone(),
        kind: member_kind(row.kind),
        recorded_state: member_state(row.recorded),
        observed_state: observed_state(row.observed),
        path: row.path.clone(),
        last_error: row.last_error.clone(),
    }
}

/// The whole listing: the root first, then every member in name order,
/// exactly as `gwz_family_model::project_list` orders it.
pub fn members(
    view: &FamilyView,
    observations: &BTreeMap<MemberName, TargetObservation>,
) -> Vec<crate::LocalFamilyMemberEntry> {
    project_list(view, observations).iter().map(entry).collect()
}

/// `LocalFamilyResponse.root_path` (design §7, §8.1; operator ruling
/// 2026-09-06): the registering root as the store observed it -- the
/// directory holding the index, reached through the pointer when the
/// addressed workspace is a clone -- spelled as a wire string the way
/// `handle_ls` spells `abspath`. Members' `path`s stay root-relative; a
/// driver joins the two. A workspace in no family has no root to name.
pub fn root_path(observation: &FamilyObservation) -> Option<String> {
    match observation {
        FamilyObservation::NoFamily => None,
        FamilyObservation::Family { root, .. } => Some(root.to_string_lossy().into_owned()),
    }
}

/// Observe every recorded member's target for the listing -- presence, the
/// pointer and the allocation marker at `root.join(row.path)` -- as
/// `gwz_family_model::TargetObservation` values for `classify_target`.
///
/// Not implemented at this checkpoint: the pointer and marker reads belong
/// to the store (lane S) and directory presence to the inspector (lane I).
/// Until they land this refuses `unsupported_operation` rather than hand
/// the projection an empty map, which would list every member as
/// `unobserved` without saying why. Since W2 the store returns a real view,
/// so this is reached: a `list` in a workspace that *is* a family member
/// refuses here. A workspace in no family never arrives -- the caller
/// answers `Ok` with an empty member list before this point.
pub(crate) fn observe_members(
    _root: &Path,
    _view: &FamilyView,
) -> ModelResult<BTreeMap<MemberName, TargetObservation>> {
    Err(unsupported("local family list: member target observation"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ErrorCode;
    use gwz_family_model::{
        AllocationId, CloneMode, FamilyId, MarkerObservation, MemberRow, PointerObservation,
    };

    fn row(path: &str, kind: MemberKind, state: MemberState) -> MemberRow {
        MemberRow {
            path: path.to_owned(),
            kind,
            state,
            allocation_id: AllocationId::new(format!("alloc-{path}")).unwrap(),
            source_path: ".".to_owned(),
            mode: if kind == MemberKind::Bare {
                CloneMode::Bare
            } else {
                CloneMode::Verbatim
            },
            last_error: None,
        }
    }

    fn name(value: &str) -> MemberName {
        MemberName::parse(value).unwrap()
    }

    fn present(pointer: PointerObservation, marker: MarkerObservation) -> TargetObservation {
        TargetObservation::Present { pointer, marker }
    }

    /// One listed row as (name, kind, recorded, observed, path, last_error),
    /// the enums by wire value.
    type Summary<'a> = (&'a str, i64, i64, i64, &'a str, Option<&'a str>);

    /// Design §7: the wire enums mirror the model's declaration order, and
    /// no two model variants share a wire value.
    #[test]
    fn every_model_variant_has_exactly_one_wire_value_in_declaration_order() {
        assert_eq!(
            [MemberKind::Checkout, MemberKind::Bare].map(|kind| member_kind(kind).wire()),
            [0, 1]
        );
        assert_eq!(
            [
                MemberState::Creating,
                MemberState::Ready,
                MemberState::Disposing
            ]
            .map(|state| member_state(state).wire()),
            [0, 1, 2]
        );
        let states = [
            ListState::Ready,
            ListState::Incomplete,
            ListState::InterruptedDisposal,
            ListState::Missing,
            ListState::PointerRemoved,
            ListState::Mismatched,
            ListState::Malformed,
            ListState::Unobserved,
        ];
        assert_eq!(
            states.map(|state| observed_state(state).wire()),
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(
            observed_state(ListState::Ready),
            crate::LocalObservedState::Ready
        );
        assert_eq!(
            observed_state(ListState::PointerRemoved),
            crate::LocalObservedState::PointerRemoved
        );
        assert_eq!(
            observed_state(ListState::Unobserved),
            crate::LocalObservedState::Unobserved
        );
    }

    /// The listing is the model's projection, row for row: the root first,
    /// then members in name order, each with its recorded state, the
    /// observed state the model classified, its root-relative path and its
    /// recorded diagnostic. Producing it changes nothing.
    #[test]
    fn the_listing_projects_the_fake_view_root_first_then_members_in_name_order() {
        let mut view = FamilyView::founded(
            FamilyId::new("fam_test").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        );
        view.members.insert(
            name("A"),
            row("../ws-A", MemberKind::Checkout, MemberState::Ready),
        );
        let mut incomplete = row("../ws-B", MemberKind::Checkout, MemberState::Creating);
        incomplete.last_error = Some("copy interrupted at src/".to_owned());
        view.members.insert(name("B"), incomplete);
        view.members.insert(
            name("C"),
            row("../ws-C", MemberKind::Checkout, MemberState::Disposing),
        );
        view.members.insert(
            name("hub"),
            row("../ws-hub", MemberKind::Bare, MemberState::Ready),
        );
        let before = view.clone();

        let mut observations = BTreeMap::new();
        observations.insert(
            name("A"),
            present(PointerObservation::Matches, MarkerObservation::Matches),
        );
        observations.insert(
            name("B"),
            present(PointerObservation::Matches, MarkerObservation::Matches),
        );
        observations.insert(name("C"), TargetObservation::Missing);
        observations.insert(
            name("hub"),
            present(PointerObservation::Absent, MarkerObservation::Matches),
        );

        let listed = members(&view, &observations);
        assert_eq!(view, before, "projecting changes nothing");

        let summary: Vec<Summary<'_>> = listed
            .iter()
            .map(|entry| {
                (
                    entry.name.as_str(),
                    entry.kind.wire(),
                    entry.recorded_state.wire(),
                    entry.observed_state.wire(),
                    entry.path.as_str(),
                    entry.last_error.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("root", 0, 1, 0, ".", None),
                ("A", 0, 1, 0, "../ws-A", None),
                ("B", 0, 0, 1, "../ws-B", Some("copy interrupted at src/")),
                ("C", 0, 2, 3, "../ws-C", None),
                ("hub", 1, 1, 4, "../ws-hub", None),
            ]
        );
        assert_eq!(
            listed[2].observed_state,
            crate::LocalObservedState::Incomplete
        );
        assert_eq!(
            listed[4].observed_state,
            crate::LocalObservedState::PointerRemoved
        );

        // A row core supplied no observation for says so, and is not an
        // endpoint; nothing guesses.
        observations.remove(&name("A"));
        let relisted = members(&view, &observations);
        assert_eq!(
            relisted[1].observed_state,
            crate::LocalObservedState::Unobserved
        );
    }

    /// Design §7/§8.1 (operator ruling 2026-09-06): `root_path` is the
    /// registering root as the store observed it, so a driver joins it with
    /// each member's root-relative `path`; outside a family there is no
    /// root to name and the field is absent.
    #[test]
    fn the_root_path_is_the_observed_root_and_absent_outside_a_family() {
        use gwz_family_store_contract::{FamilyObservation, FamilySource};
        let view = FamilyView::founded(
            FamilyId::new("fam_test").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        );
        let root = Path::new("/somewhere/gwz-dev");
        for source in [FamilySource::Index, FamilySource::Pointer] {
            let observation = FamilyObservation::Family {
                root: root.to_path_buf(),
                source,
                view: view.clone(),
            };
            assert_eq!(
                root_path(&observation).as_deref(),
                Some("/somewhere/gwz-dev"),
                "{source:?}: the root is named whether reached by index or pointer"
            );
        }
        assert_eq!(root_path(&FamilyObservation::NoFamily), None);
    }

    #[test]
    fn observing_member_targets_is_refused_unsupported_at_this_checkpoint() {
        let view = FamilyView::founded(
            FamilyId::new("fam_test").unwrap(),
            AllocationId::new("alloc-root").unwrap(),
        );
        let error = observe_members(Path::new("/nowhere"), &view).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedOperation);
        assert!(error.message.contains("member target observation"));
    }
}
