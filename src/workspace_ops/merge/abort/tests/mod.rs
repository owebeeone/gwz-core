use super::*;
use crate::artifact;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, EventSink, NullSink};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::{
    MergeOperationRecord, MergeParticipantObservation, MergeParticipantRecord, MergeStatusSnapshot,
    MergeStore, OperationState, ParticipantDrift, ParticipantDriftKind, ParticipantState,
    PendingActionObservation, PendingActionObservationState, PendingMergeAction,
    PendingMergeActionKind, RollbackEligibility, status::PendingActionReconciliation,
};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

mod lifecycle;
mod reconciliation;
mod recovery;
mod support;

use support::*;
