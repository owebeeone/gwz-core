use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{GitRepositoryState, GitStatus};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::{
    MergeParticipantRecord, OperationDriftKind, ParticipantDriftKind, ParticipantState,
};

use super::*;

mod classification;
mod operation;
mod pending;
mod support;

use classification::{participant, pending_record};
use support::*;
