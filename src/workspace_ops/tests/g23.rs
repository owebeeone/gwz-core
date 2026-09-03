use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artifact::read_lock;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::runtime::clock::{FixedClock, TimestampMs};
use crate::runtime::ids::SequentialIdProvider;
use crate::workspace_ops::merge::OperationState;

use super::*;

mod a1_activation;
mod abort_recovery;
mod continue_merge;
mod crash_recovery;
mod engine_parity;
mod pre_014_refusal;
mod finalization;
mod fixtures;
mod gc;
mod m4_matrix;
mod open_operation_gate;
mod preserve;
mod root_abort;
mod root_drift;
mod root_recovery;
mod root_stage;
mod root_start;
mod start;

use fixtures::*;
