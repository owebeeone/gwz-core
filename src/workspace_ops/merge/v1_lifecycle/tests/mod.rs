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

/// True when `text[index..]` begins with `needle`.
fn at(text: &[char], index: usize, needle: &str) -> bool {
    needle.chars().count() <= text.len() - index
        && text[index..]
            .iter()
            .zip(needle.chars())
            .all(|(seen, want)| *seen == want)
}

/// `Some(hash count)` when a raw string literal opens at `index`.
fn raw_string_hashes(text: &[char], index: usize) -> Option<usize> {
    if text[index] != 'r' {
        return None;
    }
    let hashes = text[index + 1..].iter().take_while(|c| **c == '#').count();
    (text.get(index + 1 + hashes) == Some(&'"')).then_some(hashes)
}

/// Blank `start..end` to spaces, retaining newlines.
fn blank(output: &mut [char], start: usize, end: usize) {
    let end = end.min(output.len());
    for slot in &mut output[start..end] {
        if *slot != '\n' {
            *slot = ' ';
        }
    }
}

/// Rust source with COMMENTS AND STRING/CHAR-LITERAL CONTENTS BLANKED to spaces,
/// newlines and offsets retained; CRLF normalized and a trailing newline restored
/// so a region may be its file's last item. THE shared masker for every
/// source-text tripwire in this tree, and a faithful port of the checker's own
/// `mask_non_code` (`check_checked_artifact_boundaries.py:1013-1076`): line
/// comments to end of line; NESTED `/* … */`; raw strings closed by matching hash
/// count; strings with escapes; char literals with the lifetime disambiguation the
/// Python has, so `'a`, `'static`, `<'a>` and `&'a str` stay code — plus
/// `'\u{…}'`, which the Python skips rather than masks.
///
/// **E4.3-B review [P3-3]'s NAMED RESIDUAL is CURED by this port**, and so are the
/// four QUIET shapes E4.4-6-B round 1 [P1-1] drove against its first replacement.
/// The cure is structural: this is a SCANNER that skips each construct whole, not
/// a `"` toggle, so an unbalanced quote inside a literal cannot desynchronise the
/// rest of the file. `the_shared_masker_…` below drives every shape the review
/// named — `'"'`, `b'"'`, `c == '"'`, `r#"a"b"#`, `r"C:\"`, an odd `"` in a block
/// comment, `"https://…"`, `b"…"`, a nested block comment, a `//` inside a raw
/// string, `'\''`, `'\u{…}'` — and the lifetime forms. The belt is fail-LOUD: a
/// mask that runs to end of input inside a literal or comment PANICS with the
/// file's name rather than silently blanking the tail, the direction an absence
/// pin cannot afford.
fn masked_code(subject: &str, source: &str) -> String {
    let text: Vec<char> = source.replace("\r\n", "\n").chars().collect();
    let mut output = text.clone();
    let (mut index, mut unterminated) = (0usize, None);
    while index < text.len() {
        if at(&text, index, "//") {
            let end = text[index..]
                .iter()
                .position(|character| *character == '\n')
                .map_or(text.len(), |offset| index + offset);
            blank(&mut output, index, end);
            index = end;
        } else if at(&text, index, "/*") {
            let mut depth = 1usize;
            let mut end = index + 2;
            while end < text.len() && depth > 0 {
                if at(&text, end, "/*") {
                    depth += 1;
                    end += 2;
                } else if at(&text, end, "*/") {
                    depth -= 1;
                    end += 2;
                } else {
                    end += 1;
                }
            }
            if depth > 0 {
                unterminated = Some("a block comment");
            }
            blank(&mut output, index, end);
            index = end;
        } else if let Some(hashes) = raw_string_hashes(&text, index) {
            let close = format!("\"{}", "#".repeat(hashes));
            let mut end = index + 2 + hashes;
            while end < text.len() && !at(&text, end, &close) {
                end += 1;
            }
            if end >= text.len() {
                unterminated = Some("a raw string literal");
            } else {
                end += close.chars().count();
            }
            blank(&mut output, index, end);
            index = end;
        } else if text[index] == '"' {
            let mut end = index + 1;
            let mut closed = false;
            while end < text.len() {
                if text[end] == '\\' {
                    end += 2;
                } else if text[end] == '"' {
                    end += 1;
                    closed = true;
                    break;
                } else {
                    end += 1;
                }
            }
            if !closed {
                unterminated = Some("a string literal");
            }
            blank(&mut output, index, end);
            index = end;
        } else if text[index] == '\'' && index + 2 < text.len() {
            let mut end = if text[index + 1] == '\\' {
                index + 3
            } else {
                index + 2
            };
            if at(&text, index + 1, "\\u{") {
                end = index + 4;
                while end < text.len() && text[end] != '}' {
                    end += 1;
                }
                end += 1;
            }
            // A lifetime carries no closing quote, so it stays code.
            if text.get(end) == Some(&'\'') {
                blank(&mut output, index, end + 1);
                index = end + 1;
            } else {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    if let Some(what) = unterminated {
        panic!(
            "`{subject}` ends inside {what}: the mask ran to end of input, so every \
             character after the opener is BLANK and an absence asserted over this file \
             would be an artefact of the mask, not a property of the source"
        );
    }
    let mut masked: String = output.into_iter().collect();
    if !masked.ends_with('\n') {
        masked.push('\n');
    }
    masked
}

/// The masker's own rows — every shape E4.4-6-B round 1 [P1-1] drove, plus the
/// lifetime forms. In each source `VISIBLE` sits in CODE after the tricky literal
/// and `HIDDEN` inside a literal or comment: the mask must keep the first, blank
/// the second, and never move the line count.
#[test]
fn the_shared_masker_blanks_every_literal_shape_and_keeps_the_code_visible() {
    for source in [
        "fn q(c: char) -> bool { c != '\"' }\nlet VISIBLE = 1;\n", // M-a char literal holding a quote
        "fn f<'a>(c: char) -> bool { c == '\"' }\nlet VISIBLE: &'a str = \"HIDDEN\";\n", // M-k, live tree shape
        "const C: u8 = b'\"';\nlet VISIBLE = 1;\n", // M-b2 byte-char literal
        "const S: &str = r#\"HIDDEN\"a\"#;\nlet VISIBLE = 1;\n", // M-c raw string, interior quote
        "const P: &str = r\"HIDDEN\\\";\nlet VISIBLE = 1;\n", // M-h raw string ending in a backslash
        "/* HIDDEN don't use \" here */\nlet VISIBLE = 1;\n", // M-i block comment, odd quote
        "fn e() -> &'static str { \"see https://HIDDEN/x\" }\nlet VISIBLE = 1;\n", // M-g, the E4.3-B residual
        "const B: &[u8] = b\"HIDDEN/x\";\nlet VISIBLE = 1;\n",                     // byte string
        "/* outer /* HIDDEN */ still HIDDEN */\nlet VISIBLE = 1;\n", // nested block comment
        "const R: &str = r#\"\n// HIDDEN\n\"#;\nlet VISIBLE = 1;\n", // `//` inside a raw string
        "fn g<'a>(s: &'a str) -> &'a str { s }\nlet VISIBLE = 1;\n", // lifetimes stay code
        "const Q: char = '\\'';\nlet VISIBLE = 1;\n",                // escaped-quote char literal
        "const U: char = '\\u{2014}';\nlet VISIBLE = 1;\n",          // unicode-escape char literal
        "// HIDDEN\nlet VISIBLE = 1;\n",                             // line comment
        "const E: &str = \"\\\"HIDDEN\\\"\";\nlet VISIBLE = 1;\n",   // escaped quotes in a string
    ] {
        let masked = masked_code("probe.rs", source);
        assert!(
            masked.contains("VISIBLE"),
            "code outside the literal was blanked: {source:?}"
        );
        assert!(
            !masked.contains("HIDDEN"),
            "literal or comment content survived: {source:?}"
        );
        assert_eq!(
            masked.lines().count(),
            source.lines().count(),
            "the mask moved the line count: {source:?}"
        );
    }
}

/// The fail-loud belt: a mask that reaches end of input inside a literal names
/// the file instead of blanking the tail in silence.
#[test]
#[should_panic(expected = "ends inside a string literal")]
fn the_shared_masker_panics_when_the_mask_ends_inside_a_literal() {
    masked_code(
        "probe.rs",
        "const S: &str = \"unterminated\nlet VISIBLE = 1;\n",
    );
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
