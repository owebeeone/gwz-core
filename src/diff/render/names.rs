//! Manifest-derived name / status / numstat record builders.
//!
//! These never re-diff or forward `DiffFormat::NameOnly` (which is
//! newline-framed and drops the rename old→new pair — spike Q4b). They build the
//! records from [`RenderEntry`]s so paths are workspace-relative, rename pairing
//! is preserved, and `-z` NUL framing is produced exactly where git defines it.
//!
//! Framing (matching `git diff [-z] --name-only|--name-status|--numstat`):
//!
//! - **name-only**: one path per entry (new side, old side for a delete). Text:
//!   `<path>\n`; `-z`: `<path>\0`.
//! - **name-status**: `<X>\t<path>\n` for non-renames, `R<sim>\t<old>\t<new>\n`
//!   for renames. `-z` reframes to `<X>\0<path>\0` and `R<sim>\0<old>\0<new>\0`
//!   (status and the two paths become three NUL-terminated fields).
//! - **numstat**: `<ins>\t<del>\t<path>\n`, binary as `-\t-\t<path>`, rename as
//!   `<ins>\t<del>\t<old> => <new>\n`. `-z` reframes a rename to
//!   `<ins>\t<del>\0<old>\0<new>\0` and others to `<ins>\t<del>\t<path>\0`.

use crate::diff::RepoDiffStatus;

use super::options::{RenderEntry, RenderOptions};

/// `--name-only`: the changed paths in manifest order, workspace-relative.
pub fn render_name_only(entries: &[RenderEntry<'_>], render: &RenderOptions) -> Vec<u8> {
    let sep = record_sep(render);
    let mut out = Vec::new();
    for re in entries {
        if let Some(path) = re.primary_ws_path() {
            out.extend_from_slice(path.as_bytes());
            out.push(sep);
        }
    }
    out
}

/// `--name-status`: status letter + path(s) per entry, rename pairs preserved.
pub fn render_name_status(entries: &[RenderEntry<'_>], render: &RenderOptions) -> Vec<u8> {
    let z = render.null_terminated;
    let mut out = Vec::new();
    for re in entries {
        let entry = re.entry;
        if matches!(entry.status, RepoDiffStatus::Renamed) {
            let (Some(old), Some(new)) = (re.old_ws_path(), re.new_ws_path()) else {
                continue;
            };
            // `R<sim>` status field; git pads to three digits (`R100`, `R097`).
            let status = format!("R{:03}", entry.similarity.unwrap_or(100).min(100));
            if z {
                push_z_fields(
                    &mut out,
                    &[status.as_bytes(), old.as_bytes(), new.as_bytes()],
                );
            } else {
                out.extend_from_slice(status.as_bytes());
                out.push(b'\t');
                out.extend_from_slice(old.as_bytes());
                out.push(b'\t');
                out.extend_from_slice(new.as_bytes());
                out.push(b'\n');
            }
        } else {
            let Some(path) = re.primary_ws_path() else {
                continue;
            };
            let status = entry.status.status_char();
            if z {
                let mut field = [0u8; 1];
                field[0] = status as u8;
                push_z_fields(&mut out, &[&field, path.as_bytes()]);
            } else {
                out.push(status as u8);
                out.push(b'\t');
                out.extend_from_slice(path.as_bytes());
                out.push(b'\n');
            }
        }
    }
    out
}

/// `--numstat`: per-file added/removed line counts + path, `-` for binary.
pub fn render_numstat(entries: &[RenderEntry<'_>], render: &RenderOptions) -> Vec<u8> {
    let z = render.null_terminated;
    let mut out = Vec::new();
    for re in entries {
        let entry = re.entry;
        let (ins, del) = if entry.is_binary {
            ("-".to_owned(), "-".to_owned())
        } else {
            (
                entry.insertions.unwrap_or(0).to_string(),
                entry.deletions.unwrap_or(0).to_string(),
            )
        };
        out.extend_from_slice(ins.as_bytes());
        out.push(b'\t');
        out.extend_from_slice(del.as_bytes());
        out.push(b'\t');

        if matches!(entry.status, RepoDiffStatus::Renamed) {
            let (Some(old), Some(new)) = (re.old_ws_path(), re.new_ws_path()) else {
                continue;
            };
            if z {
                // `<ins>\t<del>\t` already written; git's -z rename numstat is
                // `<ins>\t<del>\0<old>\0<new>\0` — but the trailing tab we wrote
                // must instead be a NUL. Roll it back to a NUL and push fields.
                *out.last_mut().unwrap() = 0;
                out.extend_from_slice(old.as_bytes());
                out.push(0);
                out.extend_from_slice(new.as_bytes());
                out.push(0);
            } else {
                out.extend_from_slice(old.as_bytes());
                out.extend_from_slice(b" => ");
                out.extend_from_slice(new.as_bytes());
                out.push(b'\n');
            }
        } else {
            let Some(path) = re.primary_ws_path() else {
                continue;
            };
            out.extend_from_slice(path.as_bytes());
            out.push(if z { 0 } else { b'\n' });
        }
    }
    out
}

/// The record separator for the simple one-path-per-line formats.
fn record_sep(render: &RenderOptions) -> u8 {
    if render.null_terminated { 0 } else { b'\n' }
}

/// Append `field\0` for each field (the `-z` NUL-terminated record shape).
fn push_z_fields(out: &mut Vec<u8>, fields: &[&[u8]]) {
    for field in fields {
        out.extend_from_slice(field);
        out.push(0);
    }
}
