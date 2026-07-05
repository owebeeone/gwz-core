//! D4 — workspace-relative patch rendering (the HYBRID strategy).
//!
//! # What this module is
//!
//! D3 plans and executes the workspace diff and produces, per repo, a
//! [`RepoDiffManifest`](crate::diff::RepoDiffManifest) of
//! [`RepoDiffEntry`](crate::diff::RepoDiffEntry)s in libgit2 delta order. D4
//! turns those entries into the exact **bytes** a client writes to a terminal or
//! feeds to `git apply`: patch hunks, name records, and stat/summary output,
//! with every path rewritten to be **workspace-relative** (member-prefixed)
//! under a member prefix.
//!
//! # The strategy (proven by the D4 render spike — `GwzDiffD4RenderSpike.md`)
//!
//! Patch bytes are produced by **libgit2** (`Diff::print(DiffFormat::Patch)`)
//! with `old_prefix`/`new_prefix` set to `a/<member>` / `b/<member>` so the
//! `diff --git`, `---`, and `+++` lines, `/dev/null`, mode/new/deleted headers,
//! binary placeholders, and binary literals all come out workspace-correct for
//! free (spike Q1a, Q2b, Q2c, Q3). On top of that, a **bounded hand post-pass**
//! over the printed bytes rewrites the two things libgit2's prefix options
//! cannot (spike Q1b, refuted against real `git`):
//!
//! - `rename from` / `rename to` (and `copy from` / `copy to`) extended-header
//!   lines — these carry bare repo-relative paths that no prefix flag touches,
//!   so we substitute `<member>/<path>` using the structured entry paths
//!   (spike Q1c);
//! - `--line-prefix`, which `git2 0.21` has no setter for — applied per physical
//!   line (spike Q4a).
//!
//! The post-pass keeps every already-correct line byte-for-byte (crucially the
//! binary literal block, which must be forwarded verbatim — spike Q3, Q4). It
//! splits libgit2's single multi-line `'F'` header chunk on `\n` and edits only
//! the two path-bearing header lines.
//!
//! Name/status/stat output is built **from the manifest**, not by re-diffing or
//! forwarding `DiffFormat::NameOnly` (which is newline-framed and loses the
//! rename old→new pairing — spike Q4b). This also side-steps the libgit2 vs git
//! `similarity index` value divergence for stat-style output (spike Q2a): the
//! renderer forwards libgit2's value verbatim and never promises git parity.
//!
//! # The public seam (what D3 consumes)
//!
//! This module is **pure with respect to the operation layer**: it takes bytes
//! and `RepoDiff*`/`Repository` in and returns bytes out. It never references a
//! Taut log, an operation runtime, a protocol message, or a wire enum. D3 owns
//! all of that and calls:
//!
//! - [`render_entry`] — patch bytes for **one** manifest entry, workspace-
//!   relative, rename/copy headers and line-prefix applied. D3 concatenates
//!   these in manifest order to build the `diff.output` patch stream.
//! - [`render_name_status`] / [`render_name_only`] / [`render_numstat`] /
//!   [`render_stat`] / [`render_shortstat`] / [`render_summary`] — the
//!   manifest-derived record/summary builders, honoring `-z` NUL framing where
//!   the format defines it.
//!
//! A [`ScopeRender`] bundles the member prefix + prefix policy for one repo so a
//! D3 caller resolves it once per target and threads it through every entry.

mod names;
mod options;
mod patch;
mod summary;

#[cfg(test)]
mod tests;

pub use names::{render_name_only, render_name_status, render_numstat};
pub use options::{PrefixPolicy, RenderEntry, RenderOptions, ScopeRender};
pub use patch::{render_entry, render_raw_entry};
pub use summary::{render_shortstat, render_stat, render_summary};
