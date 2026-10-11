//! Where the notes of §9 go: a reader the caller injects, given each note as
//! its line of text. The machines make the notes and the governor keeps them;
//! this turns them into words and hands them over, outside the governor's
//! lock. Whether a note is printed, and where, is the reader's.
use super::governor::Governor;
use std::sync::Arc;

/// A reader of the `note: ` lines, which the CLI prints on stderr in human
/// mode. Cloned to every endpoint of a command; there is no global.
#[derive(Clone)]
pub struct NoteSink(Arc<dyn Fn(&str) + Send + Sync>);

impl NoteSink {
    pub fn new(read: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self(Arc::new(read))
    }
    /// Gives the reader one line, as the endpoints do.
    pub fn tell(&self, line: &str) {
        (self.0)(line);
    }
    /// Gives the reader every note `governor` has made since the last call,
    /// rendered, in the order made.
    pub(crate) fn deliver(&self, governor: &Governor) {
        for (site, note) in governor.drain_all_notes() {
            self.tell(&note.render(&site));
        }
    }
}
