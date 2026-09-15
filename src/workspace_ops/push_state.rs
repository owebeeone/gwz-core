//! Last-known-state classification for a push (gwz-dev
//! `dev-docs/GwzUrlSchemePushPlan.md` §3.5 rule 2). Before any read, a selected
//! repository's source object is compared with its last-known ref to decide
//! whether the push contacts that destination at all. The classification is
//! pure; [`planned_push`] gathers its inputs from the repository's own
//! configuration and refs, never from the remote.
use std::path::Path;

use crate::git::{GitBackend, GitHeadState, same_repository};

/// How a push source compares with its last-known ref.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LastKnownState {
    /// The same object id.
    Equal,
    /// The last-known object descends from the source.
    Behind,
    /// The source descends from the last-known object.
    Ahead,
    /// Neither descends from the other.
    Diverged,
    /// No last-known ref, an ancestry error, or answers that contradict.
    Unknown,
}

/// A last-known ref's object, with the two ancestry answers between it and the
/// source as `GitBackend::is_ancestor` gives them. An `Err` is an ancestry error
/// (missing objects, shallow history).
pub(super) struct LastKnownRef<'a, E> {
    pub(super) object: &'a str,
    /// `is_ancestor(object, source)`: the source's history holds the object.
    pub(super) ancestor_of_source: Result<bool, E>,
    /// `is_ancestor(source, object)`: the object's history holds the source.
    pub(super) descendant_of_source: Result<bool, E>,
}

/// The classification, with the contact decision of §3.5 rule 2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PushContact {
    pub(super) state: LastKnownState,
    /// False only for an ordinary push equal to or behind its last-known ref
    /// (D6): that repository is `Noop` and not contacted.
    pub(super) contacted: bool,
}

/// Classifies `source` against its last-known ref, `None` when §3.5's
/// conditions give none. `forced` is whether the captured refspec starts with
/// `+`; it changes the decision, never the state. Equality compares object
/// ids, never ancestry. Different ids need both answers: an error in either, or
/// two that contradict, leaves the state unknown, so the destination is
/// contacted.
pub(super) fn classify_push_state<E>(
    source: &str,
    last_known: Option<LastKnownRef<'_, E>>,
    forced: bool,
) -> PushContact {
    let state = match last_known {
        None => LastKnownState::Unknown,
        Some(known) if known.object == source => LastKnownState::Equal,
        Some(known) => match (known.ancestor_of_source, known.descendant_of_source) {
            (Ok(true), Ok(false)) => LastKnownState::Ahead,
            (Ok(false), Ok(true)) => LastKnownState::Behind,
            (Ok(false), Ok(false)) => LastKnownState::Diverged,
            _ => LastKnownState::Unknown,
        },
    };
    let contacted = forced || !matches!(state, LastKnownState::Equal | LastKnownState::Behind);
    PushContact { state, contacted }
}

/// A selected repository's row before any read: `Noop`, with its reason, when
/// §3.5 rule 2 finds its branch unchanged since the last fetch or push, and
/// otherwise a planned push of `refspec` through `remote`. A request that
/// checks every remote skips the classification (rule 3).
pub(super) fn planned_push<B: GitBackend>(
    backend: &B,
    path: &Path,
    request: &crate::PushRequest,
    remote: &str,
    head: &GitHeadState,
    refspec: String,
) -> (crate::MemberStatus, crate::PlannedChange) {
    let checks_every_remote = matches!(request.remote_check, Some(crate::RemoteCheck::Always));
    let unchanged = if checks_every_remote {
        None
    } else {
        unchanged_reason(backend, path, remote, &refspec)
    };
    let (status, action, message) = match unchanged {
        Some(reason) => (
            crate::MemberStatus::Noop,
            crate::PlannedAction::Noop,
            reason,
        ),
        None => (
            crate::MemberStatus::Planned,
            crate::PlannedAction::Push,
            format!("push to {remote}"),
        ),
    };
    let planned = crate::PlannedChange {
        action,
        from_ref: head.commit.clone(),
        to_ref: Some(refspec),
        message: Some(message),
    };
    (status, planned)
}

/// The `Noop` reason when a push of `refspec` from the repository at `path`
/// through `remote` need not contact its destination, or `None` when it must.
/// Only an ordinary transfer of one source this repository resolves, to
/// `refs/heads/<branch>`, can go uncontacted: through a remote whose push URL
/// is absent or reaches its fetch URL's repository, and only when the backend
/// has a last-known ref for that branch that equals the source or descends
/// from it (D6). A forced or deleting transfer, a pattern or shorthand, and
/// every failed or unknown answer are contacted.
fn unchanged_reason<B: GitBackend>(
    backend: &B,
    path: &Path,
    remote: &str,
    refspec: &str,
) -> Option<String> {
    let forced = refspec.starts_with('+');
    let plain = refspec.strip_prefix('+').unwrap_or(refspec);
    let (source, destination) = plain.split_once(':')?;
    let branch = destination.strip_prefix("refs/heads/")?;
    if [source, branch]
        .iter()
        .any(|part| part.is_empty() || part.contains('*'))
    {
        return None;
    }
    let object = backend.read_ref(path, source).ok().flatten()?;
    // A forced transfer is contacted whatever its last-known ref says.
    let known = if forced || !push_url_reaches_fetch_repository(backend, path, remote) {
        None
    } else {
        backend
            .last_known_ref(path, remote, destination)
            .ok()
            .flatten()
    };
    let last_known = known.as_deref().map(|known| {
        // Equality compares object ids and never asks for ancestry.
        let ask = |ancestor: &str, descendant: &str| {
            if known == object.as_str() {
                Ok(false)
            } else {
                backend.is_ancestor(path, ancestor, descendant)
            }
        };
        LastKnownRef {
            object: known,
            ancestor_of_source: ask(known, &object),
            descendant_of_source: ask(&object, known),
        }
    });
    let contact = classify_push_state(&object, last_known, forced);
    let relation = match (contact.contacted, contact.state) {
        (false, LastKnownState::Equal) => "up to date with",
        (false, LastKnownState::Behind) => "behind",
        _ => return None,
    };
    Some(format!(
        "{relation} {remote}/{branch} as of the last fetch or push"
    ))
}

/// §3.5: a last-known ref may stand for a destination only when the remote's
/// push URL is absent or reaches the same repository as its fetch URL: the two
/// are equal, or differ only by scheme in either direction. The backend checks
/// its own conditions as well; this one holds whatever the backend.
fn push_url_reaches_fetch_repository<B: GitBackend>(
    backend: &B,
    path: &Path,
    remote: &str,
) -> bool {
    let Ok(remotes) = backend.remotes(path) else {
        return false;
    };
    remotes
        .into_iter()
        .find(|configured| configured.name == remote)
        .is_some_and(|configured| match (configured.url, configured.push_url) {
            (Some(_), None) => true,
            (Some(url), Some(push_url)) => {
                same_repository(&url, &push_url) || same_repository(&push_url, &url)
            }
            (None, _) => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use LastKnownState::{Ahead, Behind, Diverged, Equal, Unknown};

    const SOURCE: &str = "1111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222";

    /// One `is_ancestor` answer: true, false or an ancestry error.
    type Answer = Result<bool, &'static str>;
    const YES: Answer = Ok(true);
    const NO: Answer = Ok(false);
    const ERR: Answer = Err("missing objects");

    /// A last-known object, then its `ancestor_of_source` ("ahead?") and
    /// `descendant_of_source` ("behind?") answers.
    type Known = Option<(&'static str, Answer, Answer)>;

    const SKIPPED: bool = false;
    const CONTACTED: bool = true;

    /// The §3.5 rule 2 table, one input shape per line: the case, the last-known
    /// ref, then the state and whether an ordinary and a forced push contact.
    #[rustfmt::skip]
    const ROWS: [(&str, Known, LastKnownState, bool, bool); 13] = [
        ("equal",                                       Some((SOURCE, NO,  NO)),  Equal,    SKIPPED,   CONTACTED),
        ("equal despite ancestry errors",               Some((SOURCE, ERR, ERR)), Equal,    SKIPPED,   CONTACTED),
        ("equal despite contradictory ancestry",        Some((SOURCE, YES, YES)), Equal,    SKIPPED,   CONTACTED),
        ("behind",                                      Some((OTHER,  NO,  YES)), Behind,   SKIPPED,   CONTACTED),
        ("ahead",                                       Some((OTHER,  YES, NO)),  Ahead,    CONTACTED, CONTACTED),
        ("diverged",                                    Some((OTHER,  NO,  NO)),  Diverged, CONTACTED, CONTACTED),
        ("no last-known ref",                           None,                     Unknown,  CONTACTED, CONTACTED),
        ("ancestry error asking if ahead",              Some((OTHER,  ERR, NO)),  Unknown,  CONTACTED, CONTACTED),
        ("ancestry error asking if ahead, behind true", Some((OTHER,  ERR, YES)), Unknown,  CONTACTED, CONTACTED),
        ("ancestry error asking if behind",             Some((OTHER,  NO,  ERR)), Unknown,  CONTACTED, CONTACTED),
        ("ancestry error asking if behind, ahead true", Some((OTHER,  YES, ERR)), Unknown,  CONTACTED, CONTACTED),
        ("ancestry errors both ways",                   Some((OTHER,  ERR, ERR)), Unknown,  CONTACTED, CONTACTED),
        ("contradictory ancestry",                      Some((OTHER,  YES, YES)), Unknown,  CONTACTED, CONTACTED),
    ];

    /// The classifier's input for one table row.
    fn last_known(known: Known) -> Option<LastKnownRef<'static, &'static str>> {
        let (object, ancestor_of_source, descendant_of_source) = known?;
        Some(LastKnownRef {
            object,
            ancestor_of_source,
            descendant_of_source,
        })
    }

    #[test]
    fn every_cell_of_the_contact_table_holds() {
        for (case, known, state, ordinary, forced) in ROWS {
            for (force, contacted) in [(false, ordinary), (true, forced)] {
                assert_eq!(
                    classify_push_state(SOURCE, last_known(known), force),
                    PushContact { state, contacted },
                    "{case}, forced: {force}"
                );
            }
        }
    }
}
