//! The Pageant a runtime found, and the rules for talking to it (GwzTransportWindowsParityPlan.md, step 3.6; TR1.8 §4
//! and §5).
//!
//! A runtime looks for a Pageant window once, when it is built, and keeps what it found: the window's handle, the
//! process that owns it and, on Windows, a handle to that process, which keeps the process object (and so its id) from
//! being reused while the runtime lives. That is the window's identity. Before each request the owner is checked
//! against it, and after the answer arrives it is checked again, so that a Pageant that went away, or a window that
//! another program now owns, is a refusal and never another agent.
//!
//! [`PageantWindow`] is the seam between those rules and the operating system: the Windows implementation sends
//! through the exchange of step 3.5; the tests' implementations are scripted.
use super::super::pageant_exchange::PageantError;
use gwz_ids::IdSource;
use std::{fmt, sync::Arc, time::Duration};

/// What keeps Pageant's process object open while a pin lives, so that its id is not given to another process. On
/// Windows it is a handle to the process; the pins of other platforms' tests have none.
pub(crate) trait Holder: Send + Sync {
    /// Whether the held process has not ended.
    fn running(&self) -> bool;
}

/// The Pageant window a runtime found, with its owner. Cloning shares the one pin.
#[derive(Clone)]
pub(crate) struct PageantPin(Arc<Pinned>);

struct Pinned {
    window: usize,
    process: u32,
    /// The process's creation time as the system counts it (100 ns since 1601), or 0 where it could not be read.
    created: u64,
    /// The mapping names of this runtime's requests: one source, so no name is drawn twice.
    ids: IdSource,
    /// Keeps the process object, so its id is not reused for another process while this pin lives. `None` where the
    /// process could not be opened (another user's Pageant is not refused for that, TD11 B).
    holder: Option<Box<dyn Holder>>,
}

impl PageantPin {
    pub(crate) fn new(
        window: usize,
        process: u32,
        created: u64,
        holder: Option<Box<dyn Holder>>,
    ) -> Self {
        Self(Arc::new(Pinned {
            window,
            process,
            created,
            ids: crate::operation_context::new_id_source(),
            holder,
        }))
    }

    pub(crate) fn window(&self) -> usize {
        self.0.window
    }

    pub(crate) fn process(&self) -> u32 {
        self.0.process
    }

    pub(crate) fn ids(&self) -> &IdSource {
        &self.0.ids
    }

    pub(crate) fn holder(&self) -> Option<&dyn Holder> {
        self.0.holder.as_deref()
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl PageantPin {
            /// A pin that names a window and a process by their numbers alone, for the rows that pick the numbers.
            pub(crate) fn numbered(window: usize, process: u32) -> Self {
                Self::new(window, process, 0, None)
            }
        }
    }
}

/// Two pins are equal when they name the same window of the same process, created at the same time.
impl PartialEq for PageantPin {
    fn eq(&self, other: &Self) -> bool {
        (self.0.window, self.0.process, self.0.created)
            == (other.0.window, other.0.process, other.0.created)
    }
}
impl Eq for PageantPin {}

impl fmt::Debug for PageantPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PageantPin {{ window: {:#x}, process: {} }}",
            self.0.window, self.0.process
        )
    }
}

/// What a request needs of Pageant's window.
pub(crate) trait PageantWindow: Send + Sync + 'static {
    /// Whether the window still belongs to the process the runtime found, and that process still runs.
    fn owner_pinned(&self) -> bool;

    /// One request and its reply: the exchange of step 3.5, bounded by `bound`, which is never zero.
    fn send(&self, request: &[u8], bound: Duration) -> Result<Vec<u8>, PageantError>;
}

/// One request to `window`, with the identity checked on both sides of the send. The check before the send is not
/// atomic with it: a window can change owner between the two, and the check after the send cannot undo what the new
/// owner then read. It rejects the result, so that a reply from a replaced Pageant is never used.
pub(crate) fn pinned_exchange(
    window: &dyn PageantWindow,
    request: &[u8],
    bound: Duration,
) -> Result<Vec<u8>, PageantError> {
    if !window.owner_pinned() {
        return Err(PageantError::Replaced);
    }
    let mut reply = window.send(request, bound)?;
    if !window.owner_pinned() {
        reply.fill(0);
        return Err(PageantError::Replaced);
    }
    Ok(reply)
}
