//! No `connection_host` test sleep-polls the stream (Phase 1 Code P2-4, step R2b; contract section 9): a test waits for
//! a stream with the async `MessageStream` calls under a waker, never by retrying a `try_read` or `try_write` after a
//! `thread::sleep` or `yield_now`. This reads the sources of every test file of the directory (through `include_str!`,
//! so a Windows-only file is read on every platform) and fails on a match arm whose pattern ends in `WouldBlock` and
//! whose body sleeps, yields or does nothing (a spin). It does not see the same wait written as a `continue` arm or
//! inside an `if ... == WouldBlock` block; no test of the directory uses either form (R2b check CODE-1).
use std::{fs, path::Path};

/// The test sources the scan covers. A test file of the directory that is not listed fails `every_test_file_is_scanned`.
const SOURCES: &[(&str, &str)] = &[
    (
        "connect_deadline_tests.rs",
        include_str!("connect_deadline_tests.rs"),
    ),
    (
        "domain_ports_tests.rs",
        include_str!("domain_ports_tests.rs"),
    ),
    (
        "factory_profile_tests.rs",
        include_str!("factory_profile_tests.rs"),
    ),
    ("https_auth_tests.rs", include_str!("https_auth_tests.rs")),
    ("https_sspi_tests.rs", include_str!("https_sspi_tests.rs")),
    ("https_tests.rs", include_str!("https_tests.rs")),
    ("stream_wake_tests.rs", include_str!("stream_wake_tests.rs")),
    ("tests.rs", include_str!("tests.rs")),
];
/// What a retry of a not-ready stream call may not do in its arm.
const RETRY_WAITS: &[&str] = &["sleep(", "yield_now", "spin_loop"];

/// The source with its whole-line comments blanked (the line count stays).
fn code_of(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("//") {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of the match arm whose pattern ends in the `WouldBlock` at `at`, or `None` when that is no arm (an
/// assertion or a construction names the kind without matching it).
fn arm_body(code: &str, at: usize) -> Option<&str> {
    let after = &code[at + "WouldBlock".len()..];
    let arrow = after.find("=>")?;
    if after[..arrow].contains([',', ';', ')', '}', '(']) {
        return None;
    }
    let body = after[arrow + 2..].trim_start();
    if !body.starts_with('{') {
        return Some(body.split(',').next().unwrap_or(body));
    }
    let mut depth = 0usize;
    for (index, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[1..index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The one-based lines of `source` whose `WouldBlock` arm waits by retrying: it sleeps, yields or is empty.
fn retry_waits(source: &str) -> Vec<usize> {
    let code = code_of(source);
    let mut found = Vec::new();
    for (at, _) in code.match_indices("WouldBlock") {
        let Some(body) = arm_body(&code, at) else {
            continue;
        };
        if body.trim().is_empty() || RETRY_WAITS.iter().any(|wait| body.contains(wait)) {
            found.push(code[..at].matches('\n').count() + 1);
        }
    }
    found
}

#[test]
fn no_connection_host_test_waits_for_a_stream_by_retrying_would_block() {
    let found: Vec<String> = SOURCES
        .iter()
        .flat_map(|(name, source)| {
            retry_waits(source)
                .into_iter()
                .map(move |line| format!("{name}:{line}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "a WouldBlock arm sleeps, yields or spins (use the async stream under block_on): {found:?}"
    );
}

#[test]
fn the_scan_finds_a_sleeping_a_yielding_and_an_empty_arm_and_passes_the_rest() {
    let sleeping = "match x {\n Err(e) if e.kind() == io::ErrorKind::WouldBlock => {\n  thread::sleep(d);\n }\n}";
    let yielding =
        "match x {\n Err(e) if e.kind() == WouldBlock => { std::thread::yield_now(); }\n}";
    let empty = "match x {\n Err(e) if e.kind() == WouldBlock => {}\n}";
    let inline = "match x {\n Err(e) if e.kind() == WouldBlock => thread::sleep(d),\n}";
    assert_eq!(retry_waits(sleeping), vec![2]);
    assert_eq!(retry_waits(yielding), vec![2]);
    assert_eq!(retry_waits(empty), vec![2]);
    assert_eq!(retry_waits(inline), vec![2]);
    let waiting = "match x {\n Err(e) if e.kind() == WouldBlock => { return Err(e); }\n}";
    let named =
        "assert_eq!(kind, Some(io::ErrorKind::WouldBlock), \"x\");\nlet e = ErrorKind::WouldBlock;";
    let commented = "// WouldBlock => { thread::sleep(d) }\nfn f() {}";
    assert!(retry_waits(waiting).is_empty());
    assert!(retry_waits(named).is_empty());
    assert!(retry_waits(commented).is_empty());
}

#[test]
fn every_test_file_is_scanned() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/transport_host/connection_host");
    let mut files: Vec<String> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with("_tests.rs") || name == "tests.rs")
        .filter(|name| name != "sleep_poll_tests.rs")
        .collect();
    files.sort();
    let scanned: Vec<&str> = SOURCES.iter().map(|(name, _)| *name).collect();
    assert_eq!(files, scanned, "SOURCES lists the directory's test files");
}
