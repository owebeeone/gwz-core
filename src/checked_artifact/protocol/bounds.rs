pub(in crate::checked_artifact) const MAX_ACTIVE_ACTION_DIRS: usize = 64;
pub(in crate::checked_artifact) const MAX_RETIRED_ACTION_DIRS: usize = 64;
pub(in crate::checked_artifact) const MAX_MANAGED_PARENT_BOOTSTRAPS: usize = 8;
pub(in crate::checked_artifact) const MAX_MANAGED_PARENT_COMPONENTS: usize = 8;
pub(in crate::checked_artifact) const MAX_BARRIER_INVOCATIONS_PER_ACTION: usize = 64;
pub(in crate::checked_artifact) const MAX_CLEANUP_ROWS: usize = 3;
pub(in crate::checked_artifact) const MAX_BOOTSTRAP_INTENT_GENERATIONS: usize =
    MAX_MANAGED_PARENT_BOOTSTRAPS + 2 * MAX_MANAGED_PARENT_COMPONENTS;

pub(in crate::checked_artifact) const BASE_ACTION_SLOTS: usize = 13;
pub(in crate::checked_artifact) const BARRIER_ACTION_SLOTS: usize =
    3 * MAX_BARRIER_INVOCATIONS_PER_ACTION;
pub(in crate::checked_artifact) const BOOTSTRAP_INTENT_ACTION_SLOTS: usize =
    2 * MAX_BOOTSTRAP_INTENT_GENERATIONS;
pub(in crate::checked_artifact) const BOOTSTRAP_MARKER_RETIREMENT_SLOTS: usize =
    MAX_MANAGED_PARENT_COMPONENTS;
pub(in crate::checked_artifact) const MAX_ACTION_SLOTS: usize = BASE_ACTION_SLOTS
    + BARRIER_ACTION_SLOTS
    + BOOTSTRAP_INTENT_ACTION_SLOTS
    + BOOTSTRAP_MARKER_RETIREMENT_SLOTS;

pub(in crate::checked_artifact) const MAX_INFRASTRUCTURE_ENTRIES: usize = 10;
pub(in crate::checked_artifact) const MAX_ROOT_ENTRIES: usize =
    MAX_ACTIVE_ACTION_DIRS + MAX_INFRASTRUCTURE_ENTRIES;
pub(in crate::checked_artifact) const MAX_NAME_BYTES: usize = 255;
pub(in crate::checked_artifact) const METADATA_ACCOUNTING_BYTES_PER_ENTRY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogBudgetV1 {
    entries: usize,
    name_bytes: usize,
    metadata_bytes: usize,
}

impl CatalogBudgetV1 {
    pub(in crate::checked_artifact) const fn new(
        entries: usize,
        name_bytes: usize,
        metadata_bytes: usize,
    ) -> Self {
        Self {
            entries,
            name_bytes,
            metadata_bytes,
        }
    }

    pub(in crate::checked_artifact) const fn tuple(self) -> (usize, usize, usize) {
        (self.entries, self.name_bytes, self.metadata_bytes)
    }
}

pub(in crate::checked_artifact) const ROOT_BUDGET_V1: CatalogBudgetV1 = CatalogBudgetV1::new(
    MAX_ROOT_ENTRIES,
    MAX_ROOT_ENTRIES * MAX_NAME_BYTES,
    MAX_ROOT_ENTRIES * METADATA_ACCOUNTING_BYTES_PER_ENTRY,
);
pub(in crate::checked_artifact) const ACTION_BUDGET_V1: CatalogBudgetV1 = CatalogBudgetV1::new(
    MAX_ACTION_SLOTS,
    MAX_ACTION_SLOTS * MAX_NAME_BYTES,
    MAX_ACTION_SLOTS * METADATA_ACCOUNTING_BYTES_PER_ENTRY,
);
pub(in crate::checked_artifact) const RETIRED_ROOT_BUDGET_V1: CatalogBudgetV1 =
    CatalogBudgetV1::new(
        MAX_RETIRED_ACTION_DIRS,
        MAX_RETIRED_ACTION_DIRS * MAX_NAME_BYTES,
        MAX_RETIRED_ACTION_DIRS * METADATA_ACCOUNTING_BYTES_PER_ENTRY,
    );

const _: () = assert!(MAX_BOOTSTRAP_INTENT_GENERATIONS == 24);
const _: () = assert!(MAX_ACTION_SLOTS == 261);
const _: () = assert!(MAX_ROOT_ENTRIES == 74);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogOccupancyV1 {
    active_action_dirs: usize,
    retired_action_dirs: usize,
    admission: CatalogAdmissionOccupancyV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogAdmissionOccupancyV1 {
    Idle,
    PreparingWithoutFinal,
    PreparingWithFinal,
}

#[allow(
    clippy::enum_variant_names,
    reason = "stable names distinguish the three independently enforced catalog bounds"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogOccupancyErrorV1 {
    ActiveLimitExceeded,
    RetiredLimitExceeded,
    RetirementCreditsExceeded,
    PreparingFinalMissing,
}

impl CatalogOccupancyV1 {
    #[cfg(not(test))]
    pub(in crate::checked_artifact) fn new(
        active_action_dirs: usize,
        retired_action_dirs: usize,
        admission: CatalogAdmissionOccupancyV1,
    ) -> Result<Self, CatalogOccupancyErrorV1> {
        Self::validate(active_action_dirs, retired_action_dirs, admission)
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn new(
        active_action_dirs: usize,
        retired_action_dirs: usize,
        admission: impl Into<CatalogAdmissionOccupancyV1>,
    ) -> Result<Self, CatalogOccupancyErrorV1> {
        Self::validate(active_action_dirs, retired_action_dirs, admission.into())
    }

    fn validate(
        active_action_dirs: usize,
        retired_action_dirs: usize,
        admission: CatalogAdmissionOccupancyV1,
    ) -> Result<Self, CatalogOccupancyErrorV1> {
        if active_action_dirs > MAX_ACTIVE_ACTION_DIRS {
            return Err(CatalogOccupancyErrorV1::ActiveLimitExceeded);
        }
        if retired_action_dirs > MAX_RETIRED_ACTION_DIRS {
            return Err(CatalogOccupancyErrorV1::RetiredLimitExceeded);
        }
        if admission == CatalogAdmissionOccupancyV1::PreparingWithFinal && active_action_dirs == 0 {
            return Err(CatalogOccupancyErrorV1::PreparingFinalMissing);
        }
        let outstanding = active_action_dirs
            + usize::from(admission == CatalogAdmissionOccupancyV1::PreparingWithoutFinal);
        if retired_action_dirs + outstanding > MAX_RETIRED_ACTION_DIRS {
            return Err(CatalogOccupancyErrorV1::RetirementCreditsExceeded);
        }
        Ok(Self {
            active_action_dirs,
            retired_action_dirs,
            admission,
        })
    }

    pub(in crate::checked_artifact) fn can_admit_new(self) -> bool {
        self.admission == CatalogAdmissionOccupancyV1::Idle
            && self.active_action_dirs < MAX_ACTIVE_ACTION_DIRS
            && self.retired_action_dirs + self.active_action_dirs < MAX_RETIRED_ACTION_DIRS
    }

    pub(in crate::checked_artifact) fn can_resume(self) -> bool {
        self.retired_action_dirs
            + self.active_action_dirs
            + usize::from(self.admission == CatalogAdmissionOccupancyV1::PreparingWithoutFinal)
            <= MAX_RETIRED_ACTION_DIRS
    }
}

#[cfg(test)]
impl From<bool> for CatalogAdmissionOccupancyV1 {
    fn from(preparing_without_final: bool) -> Self {
        if preparing_without_final {
            Self::PreparingWithoutFinal
        } else {
            Self::Idle
        }
    }
}
