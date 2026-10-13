//! What an endpoint knows of how a member's open ended that the wire has no
//! field for, handed to the driver that words the failure, in process only
//! (adaptive concurrency design §5.3; the 5.6 remediation round 1). A failure
//! crosses to the driver as a code with a few closed fields, so the endpoint
//! leaves this beside it in the runtime's `Handoff`, as it leaves a refused
//! host key (decision D12), and the driver words the failure from both. A
//! driver whose endpoint is in another process has none, and words the
//! failure from the wire alone.
use super::capacity::LocalWait;

/// How one member's open ended, beyond its failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndNote {
    /// The member made no attempt: its key is Down after two failed retests
    /// and finished it with the failure the key recorded (§5.5).
    NotAttempted,
    /// The member shared the failure of a retest of its Down key (§5.5).
    StillFailing,
    /// A throttled member's attempts that the server refused (a 429, or a 503
    /// or 403 with `Retry-After`) of those it made.
    Refused { count: u32 },
    /// The local wait that ran out the member's allocation, and how long it
    /// lasted.
    LocalWait(LocalWait),
}

/// A note with the site it words: `github.com:22`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ended {
    pub(crate) note: EndNote,
    site: String,
}

impl Ended {
    /// The note for a failure of `host` and `port`; an IPv6 literal is
    /// bracketed, as the notes of §9 name it.
    pub(crate) fn new(note: EndNote, host: &str, port: u16) -> Self {
        let site = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        Self { note, site }
    }

    /// `reason`, the words of the failure as it stands, put as the note says
    /// when the note is about a Down key (§5.3): `not attempted: <host:port>
    /// failed earlier in this run (<reason>)` for a member that made no
    /// attempt, and `<reason>; still failing after a retry about 30 s later`
    /// for one that shares a retest's failure. Any other note leaves the
    /// reason as it is.
    pub(crate) fn down_words(&self, reason: String) -> String {
        match self.note {
            EndNote::NotAttempted => format!(
                "not attempted: {} failed earlier in this run ({reason})",
                self.site
            ),
            EndNote::StillFailing => {
                format!("{reason}; still failing after a retry about 30 s later")
            }
            EndNote::Refused { .. } | EndNote::LocalWait(_) => reason,
        }
    }

    /// Whether the note is about a Down key.
    pub(crate) fn is_down(&self) -> bool {
        matches!(self.note, EndNote::NotAttempted | EndNote::StillFailing)
    }
}
