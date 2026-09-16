//! Unit tests for the disposal service, grouped by the behaviour they pin.

pub(crate) use std::path::{Path, PathBuf};

pub(crate) use gwz_family_model::{
    FamilyChange, MemberName, MemberRow, MemberState, Refusal, TargetObservation,
};
pub(crate) use gwz_family_store_contract::{FamilySession, MetadataEffect, StoreError};
pub(crate) use gwz_repo_contract::{
    Observation, ProtectedRoots, RepoKey, RepositoryInfo, UnknownKind, UnknownReason,
    WorkObservation,
};
pub(crate) use gwz_work_detector::GwzEvidence;

pub(crate) use super::*;
pub(crate) use crate::test_support::{DisposalCall, RecordingDisposalPorts};
pub(crate) use gwz_family_model::FamilyView;
pub(crate) use gwz_family_model::{
    AllocationId, CloneMode, FamilyId, MarkerObservation, MemberKind, PointerObservation,
};
pub(crate) use gwz_family_store_contract::contract_tests::InMemoryFamilyStore;
pub(crate) use gwz_family_store_contract::{
    AppliedChange, FamilyLocation, FamilyStore, StoreOperation,
};
pub(crate) use gwz_repo_contract::{HeadState, ObjectFormat, ObjectId, WorkEntry, WorkKind};
pub(crate) use gwz_work_detector::EvidenceState;

mod disposal;
mod evidence;
mod hazards;
mod history;
mod paths;
mod refusal_sweep;
mod store_failures;
mod support;

pub(crate) use support::*;
