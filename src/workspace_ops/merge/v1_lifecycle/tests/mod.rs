mod acceptance;
mod archive_result;
mod authority;
pub(in crate::workspace_ops::merge::v1_lifecycle) mod c7_matrix;
mod capability_free_exception;
mod checked;
mod dispatcher;
mod dispatcher_attempt_matrix;
mod dispatcher_matrix;
mod dispatcher_reconciliation;
mod effect;
mod effect_retry;
pub(in crate::workspace_ops::merge::v1_lifecycle) mod fixtures;
mod forward;
mod journal_vocabulary;
mod no_ff_determinism;
mod no_ff_wire;
pub(super) mod predecessor_matrix;
mod prefixed_preservation;
mod publication_attempt_sequence;
mod reducer;
mod retirement;
mod reverse_entry;
mod reverse_no_ff;
mod reverse_router;
mod store;
mod vocabulary;

/// Rust source with STRING LITERAL CONTENTS BLANKED before `//` is stripped to end
/// of line — CRLF normalized, lines preserved, trailing newline restored so a
/// region may be its file's last item. THE shared masker for every source-text
/// tripwire here (E4.4-6-B, 2026-09-02): stripping `//` without string awareness
/// hides a door on any line whose earlier `//` sits inside a literal
/// (`"https://…"`), which the E4.3-B round-2 reviewer drove to a false `2 passed`.
/// `check_checked_artifact_boundaries.py::mask_non_code`'s idiom in Rust — only
/// `"` toggles, which covers raw strings (`r#"…"#`), and `//` cannot occur inside
/// a char literal.
fn masked_code(source: &str) -> String {
    let (mut kept, mut quoted) = (Vec::new(), false);
    for line in source.replace("\r\n", "\n").lines() {
        let (mut out, mut escaped) = (String::new(), false);
        let mut characters = line.chars().peekable();
        while let Some(character) = characters.next() {
            if escaped {
                (escaped, _) = (false, out.push(' '));
            } else if quoted && character == '\\' {
                (escaped, _) = (true, out.push(' '));
            } else if character == '"' {
                (quoted, _) = (!quoted, out.push('"'));
            } else if !quoted && character == '/' && characters.peek() == Some(&'/') {
                break;
            } else {
                out.push(if quoted { ' ' } else { character });
            }
        }
        kept.push(out);
    }
    kept.join("\n") + "\n"
}

/// One top-level item's text, from its signature to the first column-zero `}`.
fn item_body<'a>(source: &'a str, subject: &str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("`{subject}` no longer declares `{signature}`"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`{signature}` has no column-zero close; scan unbounded"));
    &rest[..end]
}
