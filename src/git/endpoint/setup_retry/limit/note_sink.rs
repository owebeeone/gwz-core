//! Where the notes of §9 go: a reader the caller injects, given each note as
//! its line of text. The machines make the notes and the governor keeps them;
//! this turns them into words and hands them over, outside the governor's
//! lock. Whether a note is printed, and where, is the reader's.
use super::governor::Governor;
use std::sync::Arc;

/// A reader of the `note: ` lines, which the CLI prints on stderr in human
/// mode. Cloned to every endpoint of a command; there is no global.
#[derive(Clone)]
pub struct NoteSink {
    read: Arc<dyn Fn(&str) + Send + Sync>,
    /// Whether the reader also wants the detail lines, which the CLI prints for `--verbose` (U9: a followed
    /// discovery redirect). Off unless the reader asks.
    detail: bool,
    /// A reader of the detail lines of its own, for a caller that gives them a level of their own (gwz-py logs
    /// them at DEBUG). Without one the detail lines go to `read`.
    detail_read: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}

impl NoteSink {
    pub fn new(read: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
            detail: false,
            detail_read: None,
        }
    }
    /// The same reader, with the detail lines on or off.
    pub fn with_detail(mut self, detail: bool) -> Self {
        self.detail = detail;
        self
    }
    /// The same reader, with the detail lines on and read by `read` instead.
    pub fn with_detail_reader(mut self, read: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.detail = true;
        self.detail_read = Some(Arc::new(read));
        self
    }
    /// Gives the reader one line, as the endpoints do.
    pub fn tell(&self, line: &str) {
        (self.read)(line);
    }
    /// Gives the reader one detail line, if it asked for detail.
    pub fn tell_detail(&self, line: &str) {
        if self.detail {
            match &self.detail_read {
                Some(read) => read(line),
                None => (self.read)(line),
            }
        }
    }
    /// Gives the reader every note `governor` has made since the last call,
    /// rendered, in the order made.
    pub(crate) fn deliver(&self, governor: &Governor) {
        for (site, note) in governor.drain_all_notes() {
            self.tell(&note.render(&site));
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn a_detail_reader_of_its_own_gets_the_detail_lines_and_the_notes_reader_only_the_notes() {
            use std::sync::Mutex;
            let notes = Arc::new(Mutex::new(Vec::<String>::new()));
            let details = Arc::new(Mutex::new(Vec::<String>::new()));
            let (note_log, detail_log) = (notes.clone(), details.clone());
            let sink = NoteSink::new(move |line| note_log.lock().unwrap().push(line.to_owned()))
                .with_detail_reader(move |line| detail_log.lock().unwrap().push(line.to_owned()));
            sink.tell("note: a");
            sink.tell_detail("note: b");
            assert_eq!(*notes.lock().unwrap(), ["note: a"]);
            assert_eq!(*details.lock().unwrap(), ["note: b"]);
            // Without one the detail lines go to the notes' reader when asked for, and nowhere else.
            let plain = Arc::new(Mutex::new(Vec::<String>::new()));
            let log = plain.clone();
            let sink = NoteSink::new(move |line| log.lock().unwrap().push(line.to_owned()));
            sink.tell_detail("note: hidden");
            let log = plain.clone();
            let sink = NoteSink::new(move |line| log.lock().unwrap().push(line.to_owned())).with_detail(true);
            sink.tell_detail("note: shown");
            assert_eq!(*plain.lock().unwrap(), ["note: shown"]);
        }
    }
}
