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
/// `root` is the family root as the store observed it (the index holder,
/// reached through the pointer when the addressed workspace is a clone);
/// the rows' recorded paths are relative to it. Observation-only (design
/// §3.1): the store reads the metadata files and never writes, repairs,
/// promotes or removes; a path that no longer exists is `Missing`, metadata
/// that cannot be read is `Malformed` with the reason, and every row gets
/// an observation, so nothing lists as `unobserved`. Infallible today; the
/// `ModelResult` stays for the dispatch slot's `?`.
pub(crate) fn observe_members(
    root: &Path,
    view: &FamilyView,
) -> ModelResult<BTreeMap<MemberName, TargetObservation>> {
    Ok(super::adapters::store::member_targets(
        &super::family_merge::family_store(),
        root,
        view,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// LCM1.1: every recorded row gets a real observation through the
    /// store -- a present member with its pointer and marker, a path that
    /// is gone -- and observing writes nothing.
    #[test]
    fn observing_member_targets_reads_each_recorded_path_through_the_store() {
        use gwz_family_model::{FamilyChange, MarkerObservation, PointerObservation};
        use gwz_family_store_contract::{FamilyLocation, FamilySession, FamilyStore};

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let store = gwz_family_store::YamlFamilyStore::new();
        let mut session = store.try_lock(&FamilyLocation::new(&root)).unwrap();
        session
            .found(
                FamilyId::new("fam_list").unwrap(),
                AllocationId::new("alloc-root").unwrap(),
            )
            .unwrap();
        for member in ["A", "gone"] {
            session
                .apply(&FamilyChange::Allocate {
                    name: name(member),
                    row: row(
                        &format!("../ws-{member}"),
                        MemberKind::Checkout,
                        MemberState::Creating,
                    ),
                })
                .unwrap();
        }
        let destination = root.join("../ws-A");
        std::fs::create_dir_all(&destination).unwrap();
        session.install_pointer(&name("A"), &destination).unwrap();
        let view = session.reread().unwrap().unwrap();
        drop(session);

        let before = std::fs::read(root.join(gwz_family_model::INDEX_RELATIVE_PATH)).unwrap();
        let observed = observe_members(&root, &view).unwrap();
        assert_eq!(
            observed.get(&name("A")),
            Some(&present(
                PointerObservation::Matches,
                MarkerObservation::Matches
            ))
        );
        assert_eq!(
            observed.get(&name("gone")),
            Some(&TargetObservation::Missing)
        );
        assert_eq!(
            std::fs::read(root.join(gwz_family_model::INDEX_RELATIVE_PATH)).unwrap(),
            before,
            "observing writes nothing"
        );
        let listed = members(&view, &observed);
        assert_eq!(
            listed
                .iter()
                .map(|entry| (entry.name.as_str(), entry.observed_state))
                .collect::<Vec<_>>(),
            vec![
                ("root", crate::LocalObservedState::Ready),
                ("A", crate::LocalObservedState::Incomplete),
                ("gone", crate::LocalObservedState::Missing),
            ],
            "a creating row stays incomplete whatever stands at its path"
        );
    }
}
