mod acceptance;
mod action;
mod baseline;
mod common;
mod journal;
mod lifecycle;
mod preservation;
mod publication;

pub(crate) use acceptance::validate_v1_acceptance;
pub(crate) use action::validate_v1_actions;
pub(crate) use baseline::validate_v1_baseline;
#[cfg(test)]
pub(crate) use common::validate_common_v0_view;
pub(crate) use common::validate_common_v1_record;
pub(crate) use journal::validate_v1_journal;
pub(crate) use lifecycle::validate_v1_lifecycle;
pub(crate) use preservation::validate_v1_preservation;
pub(crate) use publication::validate_v1_publication;

use crate::model::ModelResult;

use super::MergeOperationRecordV1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ValidatedV1Record(MergeOperationRecordV1);

impl ValidatedV1Record {
    pub(crate) fn into_record(self) -> MergeOperationRecordV1 {
        self.0
    }
}

pub(crate) fn validate_v1_record(record: MergeOperationRecordV1) -> ModelResult<ValidatedV1Record> {
    validate_common_v1_record(&record)?;
    validate_v1_actions(&record)?;
    validate_v1_lifecycle(&record)?;
    validate_v1_acceptance(&record)?;
    validate_v1_publication(&record)?;
    validate_v1_journal(&record)?;
    Ok(ValidatedV1Record(record))
}

#[cfg(test)]
mod acceptance_tests;
#[cfg(test)]
mod action_tests;
#[cfg(test)]
mod canonical_tests;
#[cfg(test)]
mod common_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod preservation_tests;
#[cfg(test)]
mod publication_tests;
#[cfg(test)]
mod tests;
