//! Diff operand parsing: comparison-form lowering and snapshot-operand
//! extraction (D0 §7.1/§7.2, plan §"Comparison forms"/§"GWZ snapshot operands").
//!
//! The client keeps raw positional operands (before `--`) plus the parsed
//! `--cached` / `--merge-base` flags. This module lowers those into a single
//! workspace-level [`ParsedComparison`]: the comparison kind and its two revision
//! *endpoints*, where each endpoint is either a plain revision token (a branch /
//! tag / commit-ish resolved per repo in D3) or a GWZ snapshot reference
//! (`+<snapshot_id>`, resolved to each member's recorded commit here in D2).
//!
//! Only the shape common to every repo is settled here; the per-repo rev/path
//! *classification* of ambiguous operands (AD9) is D3's job. D2 needs this much
//! to know (a) which comparison kind each target gets and (b) whether a snapshot
//! operand is present, because a snapshot operand narrows the candidate set.

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::RepoDiffComparisonKind;

/// One resolved endpoint of a comparison: a plain revision token, or a GWZ
/// snapshot reference. A snapshot endpoint keeps its bare id (no leading `+`);
/// D2 resolves it to each member's recorded commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    /// A revision token interpreted per repo (`HEAD`, `main`, a sha, …).
    Revision(String),
    /// A `+<snapshot_id>` operand; the stored id without the leading `+`.
    Snapshot(String),
}

impl Endpoint {
    /// Classify a raw operand token: a leading `+` marks a snapshot reference,
    /// everything else is a revision token. (After `--` a `+name` is always a
    /// literal path and never reaches here.)
    fn parse(token: &str) -> Self {
        match token.strip_prefix('+') {
            Some(id) => Endpoint::Snapshot(id.to_owned()),
            None => Endpoint::Revision(token.to_owned()),
        }
    }

    /// The snapshot id, if this endpoint is a snapshot reference.
    pub fn snapshot_id(&self) -> Option<&str> {
        match self {
            Endpoint::Snapshot(id) => Some(id),
            Endpoint::Revision(_) => None,
        }
    }
}

/// The workspace-level comparison after operand lowering: the kind plus its
/// resolved endpoints. `left`/`right` are `None` for the sides a kind fills in
/// per repo (the worktree/index side, or a defaulted `HEAD`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedComparison {
    pub kind: RepoDiffComparisonKind,
    pub left: Option<Endpoint>,
    pub right: Option<Endpoint>,
    /// Use `merge-base(left, right-or-HEAD)` as the old side (`--merge-base` and
    /// the `A...B` range form).
    pub merge_base: bool,
}

impl ParsedComparison {
    /// Every snapshot id referenced by this comparison (0, 1, or 2), in
    /// left-then-right order.
    pub fn snapshot_ids(&self) -> Vec<&str> {
        [self.left.as_ref(), self.right.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(Endpoint::snapshot_id)
            .collect()
    }

    /// True when any endpoint is a snapshot reference. A snapshot operand
    /// narrows the candidate set (root is omitted; snapshotless members are
    /// excluded).
    pub fn has_snapshot(&self) -> bool {
        !self.snapshot_ids().is_empty()
    }
}

/// Lower parsed CLI operands + flags into a [`ParsedComparison`] (D0 §7.1).
///
/// `operands` are the raw positional tokens before `--`. `cached` is
/// `--cached`/`--staged`; `merge_base` is `--merge-base`. Range forms (`A..B`,
/// `A...B`) are split *before* endpoint classification so `+base..+tip` becomes
/// two snapshot endpoints, never one id containing dots.
///
/// Rejects operand shapes v0 does not accept (more than two operands, a range
/// combined with a second operand, `--merge-base` without a commit) as
/// [`ErrorCode::InvalidRequest`].
pub fn parse_comparison(
    operands: &[String],
    cached: bool,
    merge_base: bool,
) -> ModelResult<ParsedComparison> {
    if operands.len() > 2 {
        return Err(invalid("diff accepts at most two revision operands"));
    }

    // A range operand (A..B / A...B) is a single token that expands to two
    // endpoints; it cannot be combined with another operand.
    if let Some(first) = operands.first()
        && let Some(range) = split_range(first)
    {
        if operands.len() != 1 {
            return Err(invalid(
                "a range operand cannot be combined with another revision",
            ));
        }
        return lower_range(range, cached, merge_base);
    }

    match operands.len() {
        0 => Ok(lower_no_operand(cached, merge_base)),
        1 => lower_one_operand(&operands[0], cached, merge_base),
        2 => lower_two_operands(&operands[0], &operands[1], cached, merge_base),
        _ => unreachable!("operand count checked above"),
    }
}

/// True when an operand is rev-only syntax that can never be a pathspec: a range
/// (`A..B` / `A...B`) or a `+snapshot` reference. Used by operand classification
/// (`super::classify`) to short-circuit these tokens as revisions without a
/// filesystem stat — git treats `..`/`...` as rev syntax and `+snap` is our
/// snapshot sigil.
pub(crate) fn is_never_path_operand(token: &str) -> bool {
    token.starts_with('+') || split_range(token).is_some()
}

/// Split `A..B` / `A...B` into (left, right, three_dot). Endpoints may be empty
/// (`..B`, `A..`), which the caller defaults to `HEAD`. `...` is detected before
/// `..`. Returns `None` when the token is not a range.
fn split_range(token: &str) -> Option<(String, String, bool)> {
    // Guard against a stray `....` producing a leading-dot right side.
    if let Some((left, right)) = token.split_once("...")
        && !right.starts_with('.')
    {
        return Some((left.to_owned(), right.to_owned(), true));
    }
    if let Some((left, right)) = token.split_once("..")
        && !right.starts_with('.')
        && !left.ends_with('.')
    {
        return Some((left.to_owned(), right.to_owned(), false));
    }
    None
}

fn lower_range(
    (left, right, three_dot): (String, String, bool),
    cached: bool,
    merge_base: bool,
) -> ModelResult<ParsedComparison> {
    if cached {
        return Err(invalid("--cached does not accept a range operand"));
    }
    let left = default_endpoint(&left);
    let right = default_endpoint(&right);
    Ok(ParsedComparison {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some(left),
        right: Some(right),
        // `A...B` uses merge-base(A, B); `--merge-base A..B` would too.
        merge_base: three_dot || merge_base,
    })
}

/// An endpoint token, defaulting an empty range side (`..B`) to `HEAD`.
fn default_endpoint(token: &str) -> Endpoint {
    if token.is_empty() {
        Endpoint::Revision("HEAD".to_owned())
    } else {
        Endpoint::parse(token)
    }
}

fn lower_no_operand(cached: bool, merge_base: bool) -> ParsedComparison {
    if cached {
        // `--cached`: index vs HEAD tree (or empty tree when unborn).
        ParsedComparison {
            kind: RepoDiffComparisonKind::IndexVsTree,
            left: None,
            right: None,
            merge_base,
        }
    } else {
        // Plain `gwz diff`: worktree vs index. `--merge-base` with no commit is
        // meaningless but harmless; it is dropped by the worktree-vs-index kind.
        ParsedComparison {
            kind: RepoDiffComparisonKind::WorktreeVsIndex,
            left: None,
            right: None,
            merge_base: false,
        }
    }
}

fn lower_one_operand(
    operand: &str,
    cached: bool,
    merge_base: bool,
) -> ModelResult<ParsedComparison> {
    let left = Endpoint::parse(operand);
    let kind = if cached {
        // `--cached <commit>`: index vs <commit> tree.
        RepoDiffComparisonKind::IndexVsTree
    } else {
        // `<commit>` / `+snap`: worktree vs <commit> tree.
        RepoDiffComparisonKind::WorktreeVsTree
    };
    Ok(ParsedComparison {
        kind,
        left: Some(left),
        right: None,
        merge_base,
    })
}

fn lower_two_operands(
    left: &str,
    right: &str,
    cached: bool,
    merge_base: bool,
) -> ModelResult<ParsedComparison> {
    if cached {
        return Err(invalid("--cached accepts at most one revision operand"));
    }
    Ok(ParsedComparison {
        kind: RepoDiffComparisonKind::TreeVsTree,
        left: Some(Endpoint::parse(left)),
        right: Some(Endpoint::parse(right)),
        merge_base,
    })
}

fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}
