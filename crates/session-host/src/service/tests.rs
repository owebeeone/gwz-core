#![cfg(test)]
//! The operation service on fake ports: core's half (decoding, classes,
//! handlers, encoding) is a script here, so these rows check the host's own
//! rules (§4.2, §5.1–§5.4, §6, §8).

mod admission;
mod calls;
mod carrier;
mod close;
mod events;
mod host;
mod ports;
mod protocol;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gwz_session_contract::{Frame, Lane, SendError, Tag};

use super::*;

use carrier::*;
use host::*;
use ports::*;
