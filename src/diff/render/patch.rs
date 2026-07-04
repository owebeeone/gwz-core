//! `render_entry` — workspace-relative patch bytes for one manifest entry.
//!
//! This is the hybrid strategy in one place: libgit2 prints the patch (prefixes
//! carrying the member path into `diff --git`/`---`/`+++`/mode/binary lines),
//! and a bounded hand post-pass rewrites the `rename from`/`rename to` (and
//! `copy from`/`copy to`) header lines and applies `--line-prefix`. Everything
//! else — hunk bodies, `/dev/null`, binary literals — is forwarded byte-for-byte
//! (spike Q1c, Q3, Q4a).

use git2::{DiffFormat, DiffOptions as Git2DiffOptions, Repository};

use crate::diff::{RepoDiffComparison, RepoDiffEntry, RepoDiffOptions};
use crate::git::git_error;
use crate::model::ModelResult;

use super::options::RenderOptions;

/// Render the patch bytes for **one** manifest entry, workspace-relative.
///
/// D3 supplies the already-resolved per-repo [`RepoDiffComparison`] and the
/// [`RepoDiffOptions`] it built the manifest with, plus the entry and the
/// per-repo [`RenderOptions`] (member prefix, prefix policy, line-prefix, `-z`
/// is irrelevant here, and `show_binary`). Rendering narrows a fresh libgit2
/// diff to this entry's path(s) so per-file work stays bounded (plan §"output
/// rendering may narrow internal work to the relevant path"); for a rename it
/// narrows to **both** sides so similarity detection still pairs them and the
/// rename headers are emitted (plan §"rename entries … must include both paths").
///
/// Pure: `RepoDiff*` + `Repository` in, bytes out. No log/operation/protocol
/// types cross this boundary.
pub fn render_entry(
    repo: &Repository,
    comparison: &RepoDiffComparison,
    options: &RepoDiffOptions,
    entry: &RepoDiffEntry,
    render: &RenderOptions,
) -> ModelResult<Vec<u8>> {
    // Narrow to just this entry's path(s). Both sides are included so a rename's
    // source blob is present for similarity detection and its `rename from`
    // header is emitted; add/modify/delete collapse to the one present side.
    let mut narrowed = options.clone();
    narrowed.pathspecs = entry_pathspecs(entry);

    let scope = &render.scope;
    let show_binary = render.show_binary;
    let old_prefix = scope.old_prefix();
    let new_prefix = scope.new_prefix();

    let diff =
        crate::diff::build_repo_diff(repo, comparison, &narrowed, |opts: &mut Git2DiffOptions| {
            opts.old_prefix(old_prefix);
            opts.new_prefix(new_prefix);
            opts.show_binary(show_binary);
        })?;

    // The bounded post-pass. libgit2 delivers the whole extended-header block as
    // one multi-line `'F'` chunk (spike cross-cutting finding); we split it on
    // `\n`, rewrite only the two path-bearing header lines, and forward every
    // other physical line unchanged. The line-prefix, if any, is applied per
    // physical line here so header, hunk, and binary lines all carry it.
    let member = scope.member_prefix().to_owned();
    let line_prefix = render.line_prefix.clone();
    let mut out: Vec<u8> = Vec::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        emit_line(&mut out, line, &member, line_prefix.as_deref());
        true
    })
    .map_err(git_error)?;

    Ok(out)
}

/// The repo-relative pathspecs that narrow the diff to one entry: both sides for
/// a rename (so the pair is detectable), else whichever side is present.
fn entry_pathspecs(entry: &RepoDiffEntry) -> Vec<String> {
    let mut specs: Vec<String> = Vec::new();
    if let Some(old) = &entry.old_path {
        specs.push(old.clone());
    }
    if let Some(new) = &entry.new_path
        && entry.old_path.as_deref() != Some(new.as_str())
    {
        specs.push(new.clone());
    }
    specs
}

/// Emit one libgit2 `DiffLine` into `out`, splitting the header (`'F'`) block on
/// physical newlines, rewriting rename/copy header lines to be workspace-
/// relative, and prefixing every physical line with `line_prefix` if set.
fn emit_line(out: &mut Vec<u8>, line: git2::DiffLine<'_>, member: &str, line_prefix: Option<&str>) {
    let origin = line.origin();
    let content = line.content();

    match origin {
        'F' => {
            // Multi-line header chunk: act per physical line so rename/copy
            // header rewriting and the line-prefix land on the right lines.
            for physical in split_inclusive_nl(content) {
                if let Some(prefix) = line_prefix {
                    out.extend_from_slice(prefix.as_bytes());
                }
                emit_header_line(out, physical, member);
            }
        }
        // Content/context lines carry a leading origin byte (`+`/`-`/space).
        // libgit2 splits these one physical line per callback, so a single
        // line-prefix + origin here is correct. Binary/other chunks (`B`, `H`,
        // …) carry no origin byte and must be forwarded verbatim.
        _ => {
            if let Some(prefix) = line_prefix {
                out.extend_from_slice(prefix.as_bytes());
            }
            if matches!(origin, '+' | '-' | ' ') {
                out.push(origin as u8);
            }
            out.extend_from_slice(content);
        }
    }
}

/// Write one physical header line, rewriting `rename from`/`rename to`/`copy
/// from`/`copy to` to prepend the member prefix (spike Q1b/Q1c). For the root
/// scope (`member` empty) the lines are already correct and pass through.
fn emit_header_line(out: &mut Vec<u8>, physical: &[u8], member: &str) {
    if member.is_empty() {
        out.extend_from_slice(physical);
        return;
    }
    // Header lines are ASCII keywords over a git path; a path can be non-UTF-8,
    // so match the keyword on bytes and splice the member prefix in before the
    // path bytes to stay byte-exact.
    for keyword in [
        b"rename from ".as_slice(),
        b"rename to ",
        b"copy from ",
        b"copy to ",
    ] {
        if let Some(rest) = physical.strip_prefix(keyword) {
            out.extend_from_slice(keyword);
            out.extend_from_slice(member.as_bytes());
            out.push(b'/');
            out.extend_from_slice(rest);
            return;
        }
    }
    out.extend_from_slice(physical);
}

/// Split bytes into physical lines, keeping the trailing `\n` on each (the last
/// segment may have none). Byte-level so non-UTF-8 paths survive.
fn split_inclusive_nl(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = Vec::new();
    let mut start = 0usize;
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push(&bytes[start..=idx]);
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        out.push(&bytes[start..]);
    }
    out
}
