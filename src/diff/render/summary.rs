//! Manifest-derived stat / shortstat / summary builders.
//!
//! Built from [`RenderEntry`]s (workspace paths, manifest order), never by
//! forwarding repo-local libgit2 stat output. This keeps paths workspace-
//! relative and side-steps the libgit2-vs-git `similarity index` divergence
//! (spike Q2a): `--summary`'s rename percentage is libgit2's value, forwarded
//! verbatim, and is documented as such — not promised to equal git's.
//!
//! `--shortstat`'s arithmetic (`N files changed, M insertions(+), K
//! deletions(-)`) is exact and matches git byte-for-byte. `--stat`'s histogram
//! graph is width-scaled like git's but its per-file line is asserted
//! structurally (path, counts, order), not byte-identical, because git's column
//! alignment depends on terminal width heuristics outside this module's scope.

use crate::diff::RepoDiffStatus;

use super::options::{RenderEntry, RenderOptions};

/// Default `--stat` render width, matching git's fallback when no terminal width
/// is known (`GIT_DEFAULT_STAT_WIDTH`-adjacent). The graph column is scaled into
/// what remains after the name and count columns.
const STAT_WIDTH: usize = 80;

/// `--shortstat`: ` N files changed, M insertions(+), K deletions(-)`.
/// Byte-identical to git: leading space, singular/plural agreement, and the
/// insertions/deletions clauses omitted when zero.
pub fn render_shortstat(entries: &[RenderEntry<'_>], _render: &RenderOptions) -> Vec<u8> {
    let files = entries.len();
    let (ins, del) = totals(entries);
    if files == 0 {
        return Vec::new();
    }
    let mut s = format!(
        " {files} {}",
        plural(files, "file changed", "files changed")
    );
    if ins > 0 {
        s.push_str(&format!(
            ", {ins} {}",
            plural(ins, "insertion(+)", "insertions(+)")
        ));
    }
    if del > 0 {
        s.push_str(&format!(
            ", {del} {}",
            plural(del, "deletion(-)", "deletions(-)")
        ));
    }
    s.push('\n');
    s.into_bytes()
}

/// `--stat`: per-file `<path> | <n> <graph>` lines followed by the shortstat
/// line. Paths are workspace-relative and in manifest order. Binary files render
/// `Bin` in place of the count/graph.
pub fn render_stat(entries: &[RenderEntry<'_>], render: &RenderOptions) -> Vec<u8> {
    if entries.is_empty() {
        return Vec::new();
    }

    // Column 1: the display name (rename shows `old => new`).
    let names: Vec<String> = entries.iter().map(stat_name).collect();
    let name_width = names.iter().map(|n| n.len()).max().unwrap_or(0);

    // Column 2: the total-changed count (ins+del), right-aligned; binary is Bin.
    let counts: Vec<Option<usize>> = entries
        .iter()
        .map(|re| {
            if re.entry.is_binary {
                None
            } else {
                Some(re.entry.insertions.unwrap_or(0) + re.entry.deletions.unwrap_or(0))
            }
        })
        .collect();
    let count_width = counts
        .iter()
        .map(|c| c.map(digits).unwrap_or(3)) // "Bin" is 3 wide
        .max()
        .unwrap_or(1);

    // The graph column gets whatever width remains after name, separators, count.
    // Layout: " <name> | <count> <graph>". git uses a similar budget.
    let fixed = 1 + name_width + 3 + count_width + 1; // leading sp + name + " | " + count + sp
    let graph_budget = STAT_WIDTH.saturating_sub(fixed).max(1);
    let max_change = counts.iter().flatten().copied().max().unwrap_or(0);

    let mut out = String::new();
    for ((re, name), count) in entries.iter().zip(&names).zip(&counts) {
        out.push(' ');
        out.push_str(name);
        for _ in name.len()..name_width {
            out.push(' ');
        }
        out.push_str(" | ");
        match count {
            None => {
                out.push_str(&format!("{:>width$}", "Bin", width = count_width));
            }
            Some(total) => {
                out.push_str(&format!("{total:>count_width$} "));
                out.push_str(&graph(re.entry, *total, max_change, graph_budget));
            }
        }
        out.push('\n');
    }

    out.push_str(&String::from_utf8_lossy(&render_shortstat(entries, render)));
    out.into_bytes()
}

/// `--summary`: extended change lines (create/delete/rename/mode change).
/// The rename percentage is libgit2's `similarity`, forwarded verbatim.
pub fn render_summary(entries: &[RenderEntry<'_>], _render: &RenderOptions) -> Vec<u8> {
    let mut out = String::new();
    for re in entries {
        let entry = re.entry;
        match entry.status {
            RepoDiffStatus::Added => {
                if let (Some(mode), Some(path)) = (entry.new_mode, re.new_ws_path()) {
                    out.push_str(&format!(" create mode {mode:06o} {path}\n"));
                }
            }
            RepoDiffStatus::Deleted => {
                if let (Some(mode), Some(path)) = (entry.old_mode, re.old_ws_path()) {
                    out.push_str(&format!(" delete mode {mode:06o} {path}\n"));
                }
            }
            RepoDiffStatus::Renamed => {
                if let (Some(old), Some(new)) = (re.old_ws_path(), re.new_ws_path()) {
                    let sim = entry.similarity.unwrap_or(100).min(100);
                    out.push_str(&format!(" rename {old} => {new} ({sim}%)\n"));
                }
            }
            RepoDiffStatus::TypeChanged => {
                if let (Some(old), Some(new), Some(path)) =
                    (entry.old_mode, entry.new_mode, re.primary_ws_path())
                    && old != new
                {
                    out.push_str(&format!(" mode change {old:06o} => {new:06o} {path}\n"));
                }
            }
            RepoDiffStatus::Modified | RepoDiffStatus::Unmerged => {}
        }
    }
    out.into_bytes()
}

/// Aggregate insertion/deletion totals across textual entries.
fn totals(entries: &[RenderEntry<'_>]) -> (usize, usize) {
    let mut ins = 0;
    let mut del = 0;
    for re in entries {
        ins += re.entry.insertions.unwrap_or(0);
        del += re.entry.deletions.unwrap_or(0);
    }
    (ins, del)
}

/// The `--stat` first-column name: `old => new` for a rename, else the primary
/// workspace path.
fn stat_name(re: &RenderEntry<'_>) -> String {
    if matches!(re.entry.status, RepoDiffStatus::Renamed)
        && let (Some(old), Some(new)) = (re.old_ws_path(), re.new_ws_path())
    {
        return format!("{old} => {new}");
    }
    re.primary_ws_path().unwrap_or_default()
}

/// The `+`/`-` graph, scaled so the largest-changed file fills `budget`.
fn graph(
    entry: &crate::diff::RepoDiffEntry,
    total: usize,
    max_change: usize,
    budget: usize,
) -> String {
    if total == 0 || max_change == 0 {
        return String::new();
    }
    let ins = entry.insertions.unwrap_or(0);
    // Scale total into the budget; keep the +/- split proportional. `minus` is
    // the remainder of the scaled width after the insertions share, so the
    // deletions count is reflected implicitly (total = ins + del).
    let scaled = ((total * budget) as f64 / max_change as f64).round() as usize;
    let scaled = scaled.clamp(1, budget);
    let plus = ((ins * scaled) as f64 / total as f64).round() as usize;
    let plus = plus.min(scaled);
    let minus = scaled - plus;
    let mut g = String::with_capacity(scaled);
    g.extend(std::iter::repeat_n('+', plus));
    g.extend(std::iter::repeat_n('-', minus));
    g
}

fn digits(mut n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut d = 0;
    while n > 0 {
        d += 1;
        n /= 10;
    }
    d
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { one } else { many }.to_owned()
}
