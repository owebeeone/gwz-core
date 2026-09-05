//! The store-side observations core composes: what stands at each recorded
//! member path, read through `gwz-family-store` (which owns the format) and
//! folded for the model's `classify_target`.

use std::collections::BTreeMap;
use std::path::Path;

use gwz_family_model::{FamilyView, MemberName, TargetObservation};
use gwz_family_store::YamlFamilyStore;

/// One observation per recorded member (design §3.1, `gwz local list`):
/// presence, pointer and marker at `root.join(row.path)`, never written to.
pub fn member_targets(
    store: &YamlFamilyStore,
    root: &Path,
    view: &FamilyView,
) -> BTreeMap<MemberName, TargetObservation> {
    view.members
        .iter()
        .map(|(name, row)| (name.clone(), store.observe_member_target(root, view, row)))
        .collect()
}
