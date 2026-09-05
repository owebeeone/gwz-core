//! The one remote-token resolver shared by `merge`, `pull` and `push`
//! (design §6; boundary document "One resolver, with verb-specific results").
//!
//! The resolver decides from an observed family view only. Whether a Git
//! remote exists is resolved by the caller's existing per-repository lookup;
//! this function reports a candidate, never a fact.

use crate::{FamilyView, MemberKind, MemberState, ROOT_NAME, ROOT_PATH};

/// The verb whose `--remote` / `local_source_name` token is being resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// `gwz merge --remote <name>`: family-only, no Git fallback.
    Merge,
    /// `gwz pull --head --remote <name>`: ready family member, else Git remote.
    Pull,
    /// `gwz push --remote <name>`: ready family member, else Git remote.
    Push,
}

/// The raw token as the driver supplied it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteToken(String);

impl RemoteToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A ready family member the token binds to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundMember {
    /// `root` or the clone name.
    pub name: String,
    /// Root-relative path (`.` for the root).
    pub path: String,
    pub kind: MemberKind,
    pub is_root: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A ready row, including an explicitly selected `root`.
    Bound(BoundMember),
    /// Merge only: the token is not a ready family member. `state` carries the
    /// lifecycle state of a row that exists but is not ready.
    UnknownLocal {
        token: String,
        state: Option<MemberState>,
    },
    /// Pull/push only: a row exists but is not ready. Never falls through to a
    /// same-named Git remote.
    LifecycleRefusal { name: String, state: MemberState },
    /// Pull/push only: no family row has this name. The caller's existing Git
    /// remote lookup decides between the remote and `missing_remote`.
    GitRemoteCandidate { name: String },
    /// No token: merge takes the ordinary Git-ref path; pull/push keep their
    /// existing defaults and request-over-policy precedence.
    NoToken,
}

/// Resolve `token` for `verb` against the observed family.
///
/// `view` is `None` when the workspace is observed to belong to no family
/// (no index and no pointer). A malformed or unreadable family never reaches
/// this function: the store refuses it, so nothing here pretends no family
/// exists.
pub fn resolve_remote_token(
    view: Option<&FamilyView>,
    token: Option<&RemoteToken>,
    verb: Verb,
) -> Resolution {
    let Some(token) = token else {
        return Resolution::NoToken;
    };
    let token = token.as_str();
    let Some(view) = view else {
        return not_a_member(token, None, verb);
    };
    if token == ROOT_NAME {
        return Resolution::Bound(BoundMember {
            name: ROOT_NAME.to_owned(),
            path: ROOT_PATH.to_owned(),
            kind: MemberKind::Checkout,
            is_root: true,
        });
    }
    match view.member(token) {
        Some((name, row)) if row.state == MemberState::Ready => Resolution::Bound(BoundMember {
            name: name.as_str().to_owned(),
            path: row.path.clone(),
            kind: row.kind,
            is_root: false,
        }),
        Some((_, row)) => not_a_member(token, Some(row.state), verb),
        None => not_a_member(token, None, verb),
    }
}

fn not_a_member(token: &str, state: Option<MemberState>, verb: Verb) -> Resolution {
    match (verb, state) {
        (Verb::Merge, state) => Resolution::UnknownLocal {
            token: token.to_owned(),
            state,
        },
        (Verb::Pull | Verb::Push, Some(state)) => Resolution::LifecycleRefusal {
            name: token.to_owned(),
            state,
        },
        (Verb::Pull | Verb::Push, None) => Resolution::GitRemoteCandidate {
            name: token.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn token(value: &str) -> RemoteToken {
        RemoteToken::new(value)
    }

    /// Expected outcome for one (verb, token) cell of the table.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Cell {
        BoundRoot,
        Bound(&'static str, MemberKind),
        UnknownLocal(Option<MemberState>),
        Lifecycle(MemberState),
        GitRemote,
        NoToken,
    }

    fn check(view: Option<&FamilyView>, verb: Verb, raw: Option<&str>, expected: Cell) {
        let owned = raw.map(token);
        let actual = resolve_remote_token(view, owned.as_ref(), verb);
        let cell = match &actual {
            Resolution::Bound(member) if member.is_root => {
                assert_eq!(member.path, ROOT_PATH);
                assert_eq!(member.kind, MemberKind::Checkout, "the root is a checkout");
                Cell::BoundRoot
            }
            Resolution::Bound(member) => {
                assert_eq!(member.path, format!("../ws-{}", member.name));
                Cell::Bound(
                    match member.name.as_str() {
                        "A" => "A",
                        "hub" => "hub",
                        other => panic!("unexpected bound member {other}"),
                    },
                    member.kind,
                )
            }
            Resolution::UnknownLocal { token, state } => {
                assert_eq!(Some(token.as_str()), raw);
                Cell::UnknownLocal(*state)
            }
            Resolution::LifecycleRefusal { name, state } => {
                assert_eq!(Some(name.as_str()), raw);
                Cell::Lifecycle(*state)
            }
            Resolution::GitRemoteCandidate { name } => {
                assert_eq!(Some(name.as_str()), raw);
                Cell::GitRemote
            }
            Resolution::NoToken => Cell::NoToken,
        };
        assert_eq!(
            cell,
            expected,
            "verb={verb:?} token={raw:?} family={}",
            view.is_some()
        );
    }

    /// The boundary document's table, every verb, every state, both
    /// fallbacks and the no-family case, in one place (F51 P2-4 closure
    /// evidence). It carries every design §7 CLI row that reaches this
    /// function: `pull --head --remote A|origin|root`, `merge --remote
    /// A|C|origin`, `push --remote hub|origin`, and the absent token behind
    /// `gwz merge feature/x` / `gwz merge A`, which are git refs and never
    /// arrive as a family selector.
    #[test]
    fn one_table_covers_every_verb_state_and_fallback() {
        let view = fixtures::view();
        let family = Some(&view);
        use Cell::*;
        use MemberState::*;
        let rows: &[(Option<&str>, Cell, Cell)] = &[
            // token,          merge,                               pull/push
            (None, NoToken, NoToken),
            (Some("root"), BoundRoot, BoundRoot),
            (
                Some("A"),
                Bound("A", MemberKind::Checkout),
                Bound("A", MemberKind::Checkout),
            ),
            // A ready bare hub binds like any other member, and its kind
            // reaches the caller: `gwz push --remote hub` (design §8.5).
            (
                Some("hub"),
                Bound("hub", MemberKind::Bare),
                Bound("hub", MemberKind::Bare),
            ),
            (Some("B"), UnknownLocal(Some(Creating)), Lifecycle(Creating)),
            (
                Some("C"),
                UnknownLocal(Some(Disposing)),
                Lifecycle(Disposing),
            ),
            (Some("Z"), UnknownLocal(None), GitRemote),
            (Some("origin"), UnknownLocal(None), GitRemote),
            (Some("HEAD"), UnknownLocal(None), GitRemote),
            (Some("upstream"), UnknownLocal(None), GitRemote),
        ];
        for (raw, merge, pull_push) in rows {
            check(family, Verb::Merge, *raw, *merge);
            check(family, Verb::Pull, *raw, *pull_push);
            check(family, Verb::Push, *raw, *pull_push);
        }
    }

    /// A workspace in no family: merge selectors are unknown, pull/push
    /// tokens are Git-remote candidates, and no token is still no token.
    #[test]
    fn no_family_never_binds_and_never_refuses_on_lifecycle() {
        // `hub` after `gwz local disband`: no family, so pull/push reach
        // the ordinary remote lookup and report missing_remote (design §8.6).
        for raw in ["A", "root", "origin", "hub"] {
            check(None, Verb::Merge, Some(raw), Cell::UnknownLocal(None));
            check(None, Verb::Pull, Some(raw), Cell::GitRemote);
            check(None, Verb::Push, Some(raw), Cell::GitRemote);
        }
        check(None, Verb::Merge, None, Cell::NoToken);
        check(None, Verb::Pull, None, Cell::NoToken);
    }

    /// Case-sensitive: `a` is not `A`, and a non-ready row never falls through
    /// to a same-named Git remote for pull/push.
    #[test]
    fn names_are_exact_and_non_ready_rows_never_fall_through() {
        let view = fixtures::view();
        check(Some(&view), Verb::Pull, Some("a"), Cell::GitRemote);
        check(
            Some(&view),
            Verb::Push,
            Some("B"),
            Cell::Lifecycle(MemberState::Creating),
        );
        assert!(matches!(
            resolve_remote_token(Some(&view), Some(&token("C")), Verb::Push),
            Resolution::LifecycleRefusal { .. }
        ));
    }
}
