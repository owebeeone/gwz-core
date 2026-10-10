//! Endpoint-local selected authority. Secret owners deliberately have no Debug.
use super::{
    agent_job::{Control, Job, Supervisor},
    ssh_key_container,
};
use crate::git::regular_file;
use gwz_transport::pool::{Identity, Key};
use std::{
    io::{self, Read},
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
const MAX_KEY: usize = 1 << 20;
const SCRATCH: usize = 256;
#[derive(Clone)]
pub(crate) struct Registry(Arc<Mutex<State>>);
struct State {
    slots: usize,
    bytes: usize,
    max_slots: usize,
    max_bytes: usize,
    key_cap: usize,
    next: u64,
    entries: Vec<Weak<Entry>>,
    /// The host's job budget, which this registry's key reads draw on.
    supervisor: Supervisor,
}
pub(crate) struct Reservation {
    registry: Registry,
    charge: usize,
    cap: usize,
}
// Declaration order releases secret allocation before its reservation.
pub(crate) struct Loaded {
    text: String,
    permit: Reservation,
    key: Key,
}
pub(crate) struct Entry {
    text: String,
    /// Held for its drop, which returns the snapshot's charge to the registry.
    _permit: Reservation,
    key: Key,
    token: String,
}
impl Registry {
    cfg_if::cfg_if! { if #[cfg(test)] {
        /// A registry with a job budget of its own.
        pub(crate) fn new() -> Self {
            Self::with_supervisor(Supervisor::new())
        }
    } }
    /// A registry whose key reads, and the endpoint that holds it, share
    /// `supervisor`'s job budget.
    pub(crate) fn with_supervisor(supervisor: Supervisor) -> Self {
        Self::limits(64, 16 << 20, MAX_KEY, supervisor)
    }
    pub(crate) fn supervisor(&self) -> Supervisor {
        self.0.lock().unwrap().supervisor.clone()
    }
    fn limits(max_slots: usize, max_bytes: usize, key_cap: usize, supervisor: Supervisor) -> Self {
        Self(Arc::new(Mutex::new(State {
            slots: 0,
            bytes: 0,
            max_slots,
            max_bytes,
            key_cap,
            next: 0,
            entries: Vec::new(),
            supervisor,
        })))
    }
    pub(crate) fn lookup(&self, key: &Key, identity: &Identity) -> io::Result<Arc<Entry>> {
        let mut pins = Vec::new();
        let mut state = self.0.lock().unwrap();
        state.entries.retain(|entry| entry.strong_count() != 0);
        pins.extend(state.entries.iter().filter_map(Weak::upgrade));
        drop(state);
        pins.into_iter()
            .find(|entry| entry.key() == key && entry.identity() == *identity)
            .ok_or_else(|| io::ErrorKind::PermissionDenied.into())
    }
    pub(crate) fn reserve(&self) -> io::Result<Reservation> {
        let mut state = self.0.lock().unwrap();
        let cap = state.key_cap + 1;
        let charge = cap + SCRATCH;
        if state.slots >= state.max_slots || charge > state.max_bytes.saturating_sub(state.bytes) {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        state.slots += 1;
        state.bytes += charge;
        Ok(Reservation {
            registry: self.clone(),
            charge,
            cap,
        })
    }
    pub(crate) fn start(
        &self,
        key: Key,
        path: PathBuf,
        deadline: Option<Instant>,
        cleanup: Duration,
    ) -> io::Result<Job<Loaded>> {
        let permit = self.reserve()?;
        Job::start(&self.supervisor(), deadline, cleanup, move |control| {
            permit.read(key, path, &control)
        })
    }
    /// Call only with a joined admission result. `live` checks the original
    /// request's cancellation/deadline, independently of the consumed Job.
    pub(crate) fn intern(
        &self,
        loaded: Loaded,
        live: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<Arc<Entry>> {
        if !Arc::ptr_eq(&self.0, &loaded.permit.registry.0) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        live()?;
        // Pins are declared before the guard, so unwind also unlocks before
        // dropping a last Entry (whose reservation locks this same ledger).
        let mut pins = Vec::new();
        let mut state = self.0.lock().unwrap();
        state.entries.retain(|entry| entry.strong_count() != 0);
        pins.extend(state.entries.iter().filter_map(Weak::upgrade));
        if let Some(entry) = pins
            .iter()
            .find(|entry| entry.key == loaded.key && entry.text == loaded.text)
        {
            let entry = entry.clone();
            drop(state);
            drop(loaded);
            return Ok(entry);
        }
        state.next = state.next.checked_add(1).ok_or(io::ErrorKind::Other)?;
        let entry = Arc::new(Entry {
            text: loaded.text,
            _permit: loaded.permit,
            key: loaded.key,
            token: format!("selected-{}", state.next),
        });
        state.entries.push(Arc::downgrade(&entry));
        drop(state);
        Ok(entry)
    }
}
impl Reservation {
    fn read(self, key: Key, path: PathBuf, control: &Control) -> io::Result<Loaded> {
        self.read_with(key, control, |buffer, control| {
            control.check()?;
            let mut file = regular_file::open(&path).map_err(clean)?;
            control.check()?;
            let mut used = 0;
            while used < buffer.len() {
                control.check()?;
                let end = (used + 8192).min(buffer.len());
                let n = file.read(&mut buffer[used..end]).map_err(clean)?;
                control.check()?;
                if n == 0 {
                    break;
                }
                used += n;
            }
            Ok(used)
        })
    }
    // Fixed allocation avoids uncharged growth/copy overlap. Keep its full
    // capacity charged for the snapshot lifetime, even for a short key file.
    fn read_with(
        self,
        key: Key,
        control: &Control,
        read: impl FnOnce(&mut [u8], &Control) -> io::Result<usize>,
    ) -> io::Result<Loaded> {
        control.check()?;
        let mut bytes = vec![0; self.cap].into_boxed_slice().into_vec();
        let used = read(&mut bytes, control).map_err(clean)?;
        control.check()?;
        if used == 0 || used >= self.cap {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        bytes.truncate(used);
        let text = String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidInput)?;
        ssh_key_container::check(&text, control)?;
        control.check()?;
        Ok(Loaded {
            text,
            permit: self,
            key,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.registry.0.lock().unwrap();
        state.slots -= 1;
        state.bytes -= self.charge;
    }
}
impl Entry {
    pub(crate) fn identity(&self) -> Identity {
        Identity::Explicit(self.token.clone())
    }
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}
fn clean(error: io::Error) -> io::Error {
    error.kind().into()
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Registry {
            pub(crate) fn with_limits(slots: usize, bytes: usize, cap: usize) -> Self { Self::limits(slots, bytes, cap, Supervisor::new()) }
            pub(crate) fn usage(&self) -> (usize, usize) { let state = self.0.lock().unwrap(); (state.slots, state.bytes) }
        }
        impl Reservation {
            pub(crate) fn test_read(self, key: Key, control: &Control, read: impl FnOnce(&mut [u8], &Control) -> io::Result<usize>) -> io::Result<Loaded> { self.read_with(key, control, read) }
        }
    }
}
