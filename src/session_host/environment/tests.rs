//! Tests of the endpoint environment snapshot (session plan CS1.5, extended
//! by CS1.9).

use super::*;
use crate::model::ErrorCode;
use crate::session_host::{HostContext, SessionOptions};
use std::collections::BTreeSet;

/// A buffer's whole allocation, once `overwrite` has written all of it. The
/// allocation is still live: no test reads memory after it is freed.
fn whole<T: Copy>(buffer: &mut Vec<T>) -> &[T] {
    let capacity = buffer.capacity();
    // SAFETY: `overwrite` wrote every element up to the capacity, so each
    // one is initialized.
    unsafe { buffer.set_len(capacity) };
    buffer
}

/// A buffer holding `held`, with `spare` in each element of its spare
/// capacity, as a buffer that once held a longer value keeps it.
fn secret_buffer<T: Copy>(held: &[T], spare: T, capacity: usize) -> Vec<T> {
    let mut buffer = Vec::with_capacity(capacity);
    buffer.extend_from_slice(held);
    for slot in buffer.spare_capacity_mut() {
        slot.write(spare);
    }
    buffer
}

/// Compiles only while `T` has no `Clone`: with one, both impls apply and
/// `check` is ambiguous.
trait NotClone<A> {
    fn check() {}
}

impl<T> NotClone<()> for T {}

impl<T: Clone> NotClone<u8> for T {}

#[test]
fn overwriting_covers_a_buffers_whole_allocation_spare_capacity_included() {
    let mut bytes = secret_buffer(b"s3cr3t", b'#', 64);
    let mut units = secret_buffer::<u16>(&[0x73, 0xD800], 0x2323, 16);
    overwrite(&mut bytes);
    overwrite(&mut units);
    assert_eq!((bytes.len(), units.len()), (6, 2), "the length is kept");
    assert!(whole(&mut bytes).iter().all(|&byte| byte == 0));
    assert!(whole(&mut units).iter().all(|&unit| unit == 0));
}

#[test]
fn a_name_or_value_overwrites_the_allocation_that_held_it() {
    let buffer = secret_buffer(b"s3cr3t-token", b'#', 64);
    let held = buffer.as_ptr();
    // SAFETY: the held bytes are ASCII, so valid UTF-8, which every
    // platform's `OsString` encoding accepts.
    let mut value = Wiped(unsafe { OsString::from_encoded_bytes_unchecked(buffer) });
    // What its drop does, stopped before the free.
    let mut bytes = value.overwritten();
    assert_eq!(bytes.as_ptr(), held, "the allocation that held the value");
    assert!(bytes.capacity() >= 64);
    assert!(whole(&mut bytes).iter().all(|&byte| byte == 0));
    assert!(value.0.is_empty(), "nothing is left to overwrite again");
}

#[test]
fn nothing_can_copy_the_snapshot_past_its_session() {
    // The session's end is the snapshot's only drop.
    <EnvironmentSnapshot as NotClone<_>>::check();
    <Wiped as NotClone<_>>::check();
}

fn snapshot(entries: &[(&[u8], &[u8])]) -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_byte_pairs(
        entries
            .iter()
            .map(|(name, value)| (name.to_vec(), value.to_vec())),
    )
    .unwrap()
}

fn value<'a>(snapshot: &'a EnvironmentSnapshot, name: &str) -> Option<&'a OsStr> {
    snapshot.get(name).map(|value| value.0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn line(name: &OsStr, value: &OsStr) -> String {
    format!(
        "{}={}",
        hex(name.as_encoded_bytes()),
        hex(value.as_encoded_bytes())
    )
}

/// A snapshot's entries as `line`s, for comparing whole snapshots.
fn entries(snapshot: &EnvironmentSnapshot) -> BTreeSet<String> {
    snapshot
        .entries
        .iter()
        .map(|(name, value)| line(&name.0, &value.0))
        .collect()
}

/// How two sets of `line`s differ, by counts only: never a name, a value or
/// an encoding of either, so a failing test prints no environment entry.
fn mismatch_report(seen: &BTreeSet<String>, wanted: &BTreeSet<String>) -> String {
    format!(
        "{} entries seen, {} wanted: {} only in seen, {} only in wanted",
        seen.len(),
        wanted.len(),
        seen.difference(wanted).count(),
        wanted.difference(seen).count()
    )
}

#[test]
fn a_mismatch_report_names_no_entry_but_still_counts_the_difference() {
    let shared = line(OsStr::new("HOME"), OsStr::new("/home/gwz"));
    let planted = line(OsStr::new("GH_TOKEN"), OsStr::new("s3cr3t"));
    let seen = BTreeSet::from([shared.clone(), planted]);
    let wanted = BTreeSet::from([shared]);
    let report = mismatch_report(&seen, &wanted);
    for secret in ["GH_TOKEN", "s3cr3t", &hex(b"GH_TOKEN"), &hex(b"s3cr3t")] {
        // The message must not carry the report, which is what is under test.
        assert!(
            !report.contains(secret),
            "the report carries the planted entry"
        );
    }
    assert!(report.contains("1 only in seen"), "{report}");
    assert!(report.contains("0 only in wanted"), "{report}");
}

#[test]
fn lookups_return_values_as_captured() {
    let snapshot = snapshot(&[(b"HOME", b"/home/gwz"), (b"EMPTY", b"")]);
    assert_eq!(snapshot.len(), 2);
    assert_eq!(value(&snapshot, "HOME"), Some(OsStr::new("/home/gwz")));
    assert_eq!(
        value(&snapshot, "EMPTY"),
        Some(OsStr::new("")),
        "an empty value is not an absent one"
    );
    assert_eq!(value(&snapshot, "ABSENT"), None);
}

#[test]
fn a_repeated_name_keeps_its_first_value() {
    let snapshot = snapshot(&[(b"NAME", b"first"), (b"NAME", b"second")]);
    assert_eq!(snapshot.len(), 1);
    assert_eq!(value(&snapshot, "NAME"), Some(OsStr::new("first")));
}

#[test]
fn an_unrepresentable_entry_is_refused_without_its_contents() {
    let secret = b"s3cr3t-value";
    let refused: [(&[u8], &[u8]); 4] = [
        (b"", secret),
        (b"SECRET_NAME\0X", secret),
        (b"SECRET_NAME=X", secret),
        (b"SECRET_NAME", b"s3cr3t\0value"),
    ];
    for (name, value) in refused {
        let pairs = vec![
            (b"KEPT".to_vec(), secret.to_vec()),
            (name.to_vec(), value.to_vec()),
            (b"LATER".to_vec(), secret.to_vec()),
        ];
        let error = EnvironmentSnapshot::from_byte_pairs(pairs).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        let text = format!("{error:?} {error}");
        assert!(!text.contains("s3cr3t"), "{text}");
        assert!(!text.contains("SECRET_NAME"), "{text}");
    }
    // A name may begin with '=', as Windows' per-drive directories do.
    let drive = snapshot(&[(b"=C:", b"C:\\work")]);
    assert_eq!(value(&drive, "=C:"), Some(OsStr::new("C:\\work")));
}

#[test]
fn formatted_output_carries_no_value_and_no_name() {
    let snapshot = snapshot(&[(b"GH_TOKEN", b"s3cr3t")]);
    let debug = format!("{snapshot:?}");
    assert_eq!(debug, "EnvironmentSnapshot { entries: 1, .. }");
    let options = SessionOptions::new(HostContext::new(), snapshot);
    let text = format!("{options:?} {options:#?}");
    assert!(!text.contains("s3cr3t"), "{text}");
    assert!(!text.contains("GH_TOKEN"), "{text}");
}

#[test]
fn a_driver_passes_its_process_environment_through_from_os_pairs() {
    // A driver's edge: the read is the driver's, in its own crate.
    EnvironmentSnapshot::from_os_pairs(std::env::vars_os())
        .expect("the platform yields no entry that is refused");
    // One read feeds both snapshots, so a thread that changes the environment
    // meanwhile (git2's start does, on Linux) cannot make them differ.
    let pairs: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let bytes: Vec<(Vec<u8>, Vec<u8>)> = pairs
        .iter()
        .map(|(name, value)| {
            (
                name.clone().into_encoded_bytes(),
                value.clone().into_encoded_bytes(),
            )
        })
        .collect();
    let from_bytes = EnvironmentSnapshot::from_byte_pairs(bytes).unwrap();
    let from_os = EnvironmentSnapshot::from_os_pairs(pairs).unwrap();
    assert_eq!(from_os.len(), from_bytes.len());
    let (seen, wanted) = (entries(&from_os), entries(&from_bytes));
    assert!(seen == wanted, "{}", mismatch_report(&seen, &wanted));
}

#[test]
fn from_os_pairs_keeps_first_values_and_refuses_what_from_byte_pairs_refuses() {
    let os = OsString::from;
    let pairs = vec![(os("NAME"), os("first")), (os("NAME"), os("second"))];
    let snapshot = EnvironmentSnapshot::from_os_pairs(pairs).unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(value(&snapshot, "NAME"), Some(OsStr::new("first")));
    let secret = "s3cr3t-value";
    let refused = [
        ("", secret),
        ("SECRET_NAME\0X", secret),
        ("SECRET_NAME=X", secret),
        ("SECRET_NAME", "s3cr3t\0value"),
    ];
    for (name, value) in refused {
        let pairs = vec![
            (os("KEPT"), os(secret)),
            (os(name), os(value)),
            (os("LATER"), os(secret)),
        ];
        let error = EnvironmentSnapshot::from_os_pairs(pairs).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        let text = format!("{error:?} {error}");
        assert!(text.contains("entry 1 "), "{text}");
        assert!(!text.contains("s3cr3t"), "{text}");
        assert!(!text.contains("SECRET_NAME"), "{text}");
    }
}

#[test]
fn a_child_spawned_by_the_helper_sees_exactly_the_snapshot() {
    let mut pairs = vec![
        (b"GWZ_SNAPSHOT_PLAIN".to_vec(), b"plain value".to_vec()),
        (b"GWZ_SNAPSHOT_EMPTY".to_vec(), Vec::new()),
        (b"GWZ_SNAPSHOT_LINES".to_vec(), b"one\ntwo=three".to_vec()),
    ];
    pairs.extend(platform_pairs());
    let snapshot = EnvironmentSnapshot::from_byte_pairs(pairs).unwrap();
    let mut command = environment_printer();
    let output = snapshot.apply_to(&mut command).output().unwrap();
    assert!(
        output.status.success(),
        "the child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (seen, wanted) = (printed_environment(&output.stdout), entries(&snapshot));
    assert!(seen == wanted, "{}", mismatch_report(&seen, &wanted));
}

#[test]
fn wtf8_decoding_keeps_unpaired_surrogates_and_refuses_malformed_bytes() {
    let decode = |bytes: &[u8]| {
        let mut units = Vec::new();
        wide::decode_wtf8(bytes, &mut units).then_some(units)
    };
    assert_eq!(decode(b"a\xED\xA0\x80b"), Some(vec![0x61, 0xD800, 0x62]));
    assert_eq!(decode(b"\xED\xB0\x80"), Some(vec![0xDC00]));
    assert_eq!(
        decode("x\u{1F600}".as_bytes()),
        Some(vec![0x78, 0xD83D, 0xDE00])
    );
    // A pair written as two surrogates, as Python's `surrogatepass` writes a
    // split pair, decodes to the same units as the four-byte form.
    assert_eq!(
        decode(b"\xED\xA0\xBD\xED\xB8\x80"),
        Some(vec![0xD83D, 0xDE00])
    );
    assert_eq!(decode(b""), Some(vec![]));
    let malformed: [&[u8]; 8] = [
        b"\xC0\x80",         // overlong
        b"\xE0\x80\x80",     // overlong
        b"\xF0\x80\x80\x80", // overlong
        b"\xF4\x90\x80\x80", // above U+10FFFF
        b"\xF8\x88\x80\x80", // not a lead byte
        b"\x80",             // a lone continuation byte
        b"\xED\xA0",         // truncated
        b"\xE2\x28\xA1",     // a bad continuation byte
    ];
    for bytes in malformed {
        assert_eq!(decode(bytes), None, "{bytes:?}");
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;

        fn platform_pairs() -> Vec<(Vec<u8>, Vec<u8>)> {
            vec![
                (b"GWZ_SNAPSHOT_BYTES".to_vec(), b"\xFF\xFE raw \x80".to_vec()),
                (b"GWZ_SNAPSHOT_\xFF".to_vec(), b"a name that is not UTF-8".to_vec()),
            ]
        }

        /// `env` prints exactly the environment it was started with. The
        /// test binary cannot: on macOS the CoreFoundation it links adds
        /// `__CF_USER_TEXT_ENCODING` to its own environment as it starts.
        fn environment_printer() -> Command {
            let mut command = Command::new("/usr/bin/env");
            command.arg("-0");
            command
        }

        fn printed_environment(stdout: &[u8]) -> BTreeSet<String> {
            stdout
                .split(|byte| *byte == 0)
                .filter(|entry| !entry.is_empty())
                .map(|entry| {
                    let equals = 1 + entry[1..].iter().position(|byte| *byte == b'=').unwrap();
                    line(OsStr::from_bytes(&entry[..equals]), OsStr::from_bytes(&entry[equals + 1..]))
                })
                .collect()
        }

        #[test]
        fn non_utf8_names_and_values_survive_on_posix() {
            let snapshot = snapshot(&[(b"BYTES", b"\xFF\xFE raw \x80"), (b"N\xFF", b"v")]);
            let bytes = value(&snapshot, "BYTES").unwrap();
            assert_eq!(bytes.as_bytes(), b"\xFF\xFE raw \x80");
            let name = OsStr::from_bytes(b"N\xFF");
            assert_eq!(snapshot.get(name).unwrap().as_os_str().as_bytes(), b"v");
        }

        #[test]
        fn names_are_case_sensitive_on_posix() {
            let snapshot = snapshot(&[(b"Path", b"mixed"), (b"PATH", b"upper")]);
            assert_eq!(snapshot.len(), 2);
            assert_eq!(value(&snapshot, "Path"), Some(OsStr::new("mixed")));
            assert_eq!(value(&snapshot, "PATH"), Some(OsStr::new("upper")));
            assert_eq!(value(&snapshot, "path"), None);
        }
    } else if #[cfg(windows)] {
        use std::collections::BTreeMap;
        use std::os::windows::ffi::OsStringExt;

        /// Set in the snapshot the probe's child runs with.
        const PROBE: &str = "GWZ_SESSION_HOST_ENVIRONMENT_PROBE";
        /// Names, separated by `;`, that the probe looks up through the OS.
        const LOOKUP: &str = "GWZ_SESSION_HOST_ENVIRONMENT_LOOKUP";
        const PROBE_TEST: &str = "session_host::environment::tests::child_environment_probe";

        fn platform_pairs() -> Vec<(Vec<u8>, Vec<u8>)> {
            // Windows needs SystemRoot to start some system services; it is
            // part of the snapshot, so the child must see it and nothing else.
            // The child is the test binary, which loads the Python runtime that the embedding tests link
            // (python313.dll) from the directories of `PATH`, so `PATH` is part of the snapshot too.
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
            let path = std::env::var("PATH").unwrap_or_default();
            vec![
                (PROBE.as_bytes().to_vec(), b"1".to_vec()),
                (b"SystemRoot".to_vec(), root.into_bytes()),
                (b"PATH".to_vec(), path.into_bytes()),
                (b"GWZ_SNAPSHOT_LONE".to_vec(), b"a\xED\xA0\x80b".to_vec()),
            ]
        }

        /// Windows adds nothing to a child's environment, so the test binary
        /// reports its own, through the probe.
        fn environment_printer() -> Command {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--ignored", "--exact", PROBE_TEST, "--nocapture", "--test-threads=1"]);
            command
        }

        /// The lines of the probe's output between `begin` and `end`.
        fn section<'a>(stdout: &'a str, begin: &str, end: &str) -> Vec<&'a str> {
            stdout
                .split(begin)
                .nth(1)
                .and_then(|rest| rest.split(end).next())
                .expect("the child reported the section")
                .lines()
                .collect()
        }

        fn printed_environment(stdout: &[u8]) -> BTreeSet<String> {
            let stdout = String::from_utf8(stdout.to_vec()).unwrap();
            section(&stdout, "BEGIN-ENVIRONMENT\n", "END-ENVIRONMENT\n")
                .into_iter()
                .map(str::to_owned)
                .collect()
        }

        #[test]
        #[ignore = "the child process of the tests that spawn the probe"]
        fn child_environment_probe() {
            if std::env::var_os(PROBE).is_none() {
                return;
            }
            let mut report = String::from("\nBEGIN-ENVIRONMENT\n");
            for (name, value) in std::env::vars_os() {
                report.push_str(&line(&name, &value));
                report.push('\n');
            }
            report.push_str("END-ENVIRONMENT\nBEGIN-LOOKUP\n");
            let names = std::env::var_os(LOOKUP).unwrap_or_default();
            for name in names.to_string_lossy().split(';').filter(|name| !name.is_empty()) {
                // The OS's own lookup, which compares names as Windows does.
                let found = std::env::var_os(name)
                    .map_or_else(|| "-".to_owned(), |value| hex(value.as_encoded_bytes()));
                report.push_str(&format!("{}={found}\n", hex(name.as_bytes())));
            }
            report.push_str("END-LOOKUP\n");
            print!("{report}");
        }

        #[test]
        fn case_variants_of_a_name_are_one_entry_for_get_apply_to_and_the_child() {
            let variants = ["Gwz_Case", "GWZ_CASE", "gwz_case", "Gwz_\u{dc}mlaut", "GWZ_\u{fc}MLAUT"];
            let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = variants
                .iter()
                .enumerate()
                .map(|(index, name)| (name.as_bytes().to_vec(), format!("value {index}").into_bytes()))
                .collect();
            pairs.extend(platform_pairs());
            pairs.push((LOOKUP.as_bytes().to_vec(), variants.join(";").into_bytes()));
            let snapshot = EnvironmentSnapshot::from_byte_pairs(pairs).unwrap();
            assert_eq!(value(&snapshot, "GWZ_CASE"), Some(OsStr::new("value 0")));
            assert_eq!(value(&snapshot, "gwz_\u{fc}mlaut"), Some(OsStr::new("value 3")));
            let mut command = environment_printer();
            snapshot.apply_to(&mut command);
            assert_eq!(command.get_envs().count(), snapshot.len(), "Command keeps each entry");
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "the child failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            let found: BTreeMap<&str, &str> = section(&stdout, "BEGIN-LOOKUP\n", "END-LOOKUP\n")
                .into_iter()
                .filter_map(|line| line.split_once('='))
                .collect();
            for name in variants {
                let expected = snapshot
                    .get(name)
                    .map_or_else(|| "-".to_owned(), |value| hex(value.as_os_str().as_encoded_bytes()));
                let seen = found.get(hex(name.as_bytes()).as_str());
                assert!(seen == Some(&expected.as_str()), "the child and get disagree on a variant");
            }
        }

        #[test]
        fn wtf8_pairs_with_an_unpaired_surrogate_decode_to_the_same_os_string() {
            let snapshot = snapshot(&[
                (b"LONE", b"a\xED\xA0\x80b"),
                (b"TRAIL", b"\xED\xB0\x80"),
                (b"PAIR", "x\u{1F600}".as_bytes()),
                (b"N\xED\xA0\x80", b"a name with an unpaired surrogate"),
            ]);
            assert_eq!(value(&snapshot, "LONE").unwrap(), OsString::from_wide(&[0x61, 0xD800, 0x62]));
            assert_eq!(value(&snapshot, "TRAIL").unwrap(), OsString::from_wide(&[0xDC00]));
            assert_eq!(value(&snapshot, "PAIR").unwrap(), OsString::from_wide(&[0x78, 0xD83D, 0xDE00]));
            let name = OsString::from_wide(&[0x4E, 0xD800]);
            assert!(snapshot.get(&name).is_some());
            let error = EnvironmentSnapshot::from_byte_pairs(vec![(b"NAME".to_vec(), b"\xFF".to_vec())]).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidRequest, "a value that is not WTF-8");
        }

        #[test]
        fn names_are_case_insensitive_on_windows() {
            let snapshot = snapshot(&[(b"Path", b"first"), (b"PATH", b"second")]);
            assert_eq!(snapshot.len(), 1);
            assert_eq!(value(&snapshot, "PATH"), Some(OsStr::new("first")));
            assert_eq!(value(&snapshot, "path"), Some(OsStr::new("first")));
        }

        #[test]
        fn a_decoded_string_is_built_at_its_final_size_and_matches_std() {
            // Each input's WTF-8 is longer than its UTF-16, which is what made
            // std's `from_wide` grow, freeing a buffer that held part of it.
            let inputs: [&[u8]; 7] = [
                "Grüße, 東京".as_bytes(),
                "x\u{1F600}y".as_bytes(),
                b"\xED\xA0\xBD\xED\xB8\x80", // a pair written as two surrogates
                b"a\xED\xA0\x80b",           // an unpaired lead
                b"\xED\xB0\x80z",            // an unpaired trail
                b"\xED\xA0\x80\xED\xA0\x80", // two leads
                b"\xED\xA0\x80\xF0\x9F\x98\x80", // a lead before a pair
            ];
            for bytes in inputs {
                let mut units = Vec::new();
                assert!(wide::decode_wtf8(bytes, &mut units));
                let text = platform::from_bytes(bytes.to_vec()).unwrap();
                assert!(text == OsString::from_wide(&units), "{bytes:?}");
                let unchanged = OsString::with_capacity(bytes.len()).capacity();
                assert_eq!(text.capacity(), unchanged, "it never grew: {bytes:?}");
            }
        }
    }
}
