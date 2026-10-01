//! The transport scope's pins (gwz-py `dev-docs/GwzPyPerOperationTransportDesign.md`
//! §2.1 and §3, "The predicate"): `Operation` names exactly the operations
//! whose gwz-core handler calls `with_transport`, each by the protocol method
//! that carries its request, and a tag is in scope only when it reaches a
//! remote.
//!
//! The source test reads gwz-core's production sources as text, so it also
//! covers handlers that only some builds compile. gwz-cli's
//! `src/tests/transport_scope.rs` pins its driver's arms to the same call
//! sites.

use super::{Operation, in_scope};
use crate::{TagOp, TagRequest};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[test]
fn operations_equal_the_handlers_that_call_with_transport() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut kinds = BTreeSet::new();
    for file in production_files(&source) {
        let text = without_test_code(code_only(&std::fs::read_to_string(&file).unwrap()));
        for (call, _) in text.match_indices(".with_transport(") {
            // A handler takes its operation's `request: crate::<Kind>Request`.
            let named: BTreeSet<String> =
                names_after(&text[enclosing_fn(&text, call)..call], "request: crate::")
                    .iter()
                    .filter_map(|name| name.strip_suffix("Request"))
                    .map(str::to_owned)
                    .collect();
            assert_eq!(
                named.len(),
                1,
                "{}: a with_transport call outside one operation's handler: {named:?}",
                file.display()
            );
            kinds.extend(named);
        }
    }
    let operations: BTreeSet<String> = Operation::ALL
        .iter()
        .map(|operation| format!("{operation:?}"))
        .collect();
    assert!(
        !kinds.is_empty(),
        "no with_transport call under {}",
        source.display()
    );
    assert_eq!(
        operations, kinds,
        "transport_scope::Operation (left) differs from the operations whose gwz-core handler \
         calls with_transport (right)"
    );
}

/// Each operation's method is its request type's kind in snake case, and the
/// method names it back; a method of no such operation, such as
/// `attach_repo_member` or `repo_sync`, whose handlers open no connection,
/// names none.
#[test]
fn each_operation_is_named_by_its_protocol_method() {
    for operation in Operation::ALL {
        assert_eq!(operation.method(), snake_case(&format!("{operation:?}")));
        assert_eq!(Operation::from_method(operation.method()), Some(operation));
    }
    for method in [
        "attach_repo_member",
        "repo_sync",
        "status",
        "merge",
        "Fetch",
        "",
    ] {
        assert_eq!(Operation::from_method(method), None, "{method}");
    }
}

#[test]
fn a_tag_is_in_scope_only_when_it_reaches_a_remote() {
    let tag = |op, remote: Option<&str>| TagRequest {
        op,
        remote: remote.map(str::to_owned),
        ..TagRequest::default()
    };
    for (op, remote, expected) in [
        (TagOp::Push, None, true),
        (TagOp::Fetch, None, true),
        (TagOp::List, Some("origin"), true),
        (TagOp::Delete, Some("origin"), true),
        (TagOp::List, None, false),
        (TagOp::Delete, None, false),
        (TagOp::Create, None, false),
        (TagOp::Create, Some("origin"), false),
    ] {
        let request = tag(op, remote);
        assert_eq!(
            in_scope(Operation::Tag, Some(&request)),
            expected,
            "{op:?} {remote:?}"
        );
        // Only a tag reads the tag request.
        assert!(in_scope(Operation::Fetch, Some(&request)));
    }
    assert!(!in_scope(Operation::Tag, None));
    for operation in Operation::ALL {
        assert_eq!(in_scope(operation, None), operation != Operation::Tag);
    }
}

fn snake_case(name: &str) -> String {
    let mut snake = String::new();
    for character in name.chars() {
        if character.is_ascii_uppercase() && !snake.is_empty() {
            snake.push('_');
        }
        snake.push(character.to_ascii_lowercase());
    }
    snake
}

/// Each identifier that directly follows `prefix` in `text`.
fn names_after(text: &str, prefix: &str) -> BTreeSet<String> {
    text.match_indices(prefix)
        .map(|(at, _)| {
            text[at + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        })
        .collect()
}

/// Where the last function item that starts before `at` begins.
fn enclosing_fn(text: &str, at: usize) -> usize {
    text[..at]
        .match_indices("fn ")
        .map(|(index, _)| index)
        .filter(|index| {
            *index == 0 || text[..*index].ends_with(|c: char| c.is_whitespace() || c == ')')
        })
        .last()
        .expect("a with_transport call inside a function")
}

/// The Rust sources under `dir`, less those named as tests: any file or
/// directory whose name contains `test`.
fn production_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.contains("test") {
            continue;
        }
        if path.is_dir() {
            files.extend(production_files(&path));
        } else if name.ends_with(".rs") {
            files.push(path);
        }
    }
    files
}

/// The index of the brace that closes the block opened at `open`.
fn block_end(text: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (index, byte) in text.bytes().enumerate().skip(open) {
        if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return index;
            }
        }
    }
    panic!("unbalanced braces after byte {open}");
}

/// `code` without the blocks that `#[cfg(test)]` or `#[cfg(all(test, ...))]`
/// gates, `cfg_if!` branches among them. A gated item that ends at `;` before
/// any block keeps its text.
fn without_test_code(mut code: String) -> String {
    for marker in ["#[cfg(test)]", "#[cfg(all(test,"] {
        while let Some(at) = code.find(marker) {
            let rest = &code[at..];
            match (rest.find('{'), rest.find(';')) {
                (Some(brace), semicolon) if semicolon.is_none_or(|end| brace < end) => {
                    let end = block_end(&code, at + brace);
                    code.replace_range(at..=end, "");
                }
                _ => {
                    code.replace_range(at..at + marker.len(), "");
                }
            }
        }
    }
    code
}

/// `text` with every comment and every string, byte string or character
/// literal blanked to spaces, byte for byte, so that offsets, braces and
/// searches see only code.
fn code_only(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut code = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &bytes[at..];
        let blanked = if rest.starts_with(b"//") {
            Some(at + rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len()))
        } else if rest.starts_with(b"/*") {
            let close = rest[2..].windows(2).position(|w| w == b"*/");
            Some(at + 2 + close.map_or(rest.len() - 2, |end| end + 2))
        } else if bytes[at] == b'"' {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            Some(end + 1)
        } else if bytes[at] == b'\'' {
            char_literal_end(text, at)
        } else {
            raw_string_end(bytes, at)
        };
        match blanked {
            Some(end) => {
                let end = end.min(bytes.len());
                for byte in &mut code[at..end] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
                at = end;
            }
            None => {
                at += 1;
            }
        }
    }
    String::from_utf8(code).expect("literals blank whole characters")
}

/// The end of a raw string literal (`r"…"`, `r#"…"#`, `br"…"`) that starts at
/// `at`, if one does.
fn raw_string_end(bytes: &[u8], at: usize) -> Option<usize> {
    if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
        return None;
    }
    let start = at + usize::from(bytes[at] == b'b');
    if bytes.get(start) != Some(&b'r') {
        return None;
    }
    let hashes = bytes[start + 1..]
        .iter()
        .take_while(|&&b| b == b'#')
        .count();
    if bytes.get(start + 1 + hashes) != Some(&b'"') {
        return None;
    }
    let mut close = vec![b'"'];
    close.extend(std::iter::repeat_n(b'#', hashes));
    let body = start + 2 + hashes;
    let found = bytes[body..]
        .windows(close.len())
        .position(|w| w == close.as_slice());
    Some(found.map_or(bytes.len(), |end| body + end + close.len()))
}

/// The end of a character literal that starts at `at`, or `None` for a
/// lifetime or a label.
fn char_literal_end(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(at + 1) == Some(&b'\\') {
        let close = bytes.get(at + 3..)?.iter().position(|&b| b == b'\'')?;
        return Some(at + 3 + close + 1);
    }
    let width = text[at + 1..].chars().next()?.len_utf8();
    (bytes.get(at + 1 + width) == Some(&b'\'')).then_some(at + 2 + width)
}
