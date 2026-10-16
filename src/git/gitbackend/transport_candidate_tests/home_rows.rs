//! The SSH home contract through a real server (plan step 3.1, "then the same rows through the fixture"; baseline rows
//! B03 and B04; design `GwzTransportWindowsParityDesign.md` §3). Each case clones from the disposable SSH fixture on
//! the transport, with an environment snapshot that names no process variable: the endpoint trusts the server only if
//! the home the contract chooses holds a `known_hosts` that lists it.
use super::*;
use crate::session_host::EnvironmentSnapshot;
use crate::transport_host::{CallControls, CancellationToken};
use std::ffi::{OsStr, OsString};
use std::sync::Arc;

/// A directory under `base` that holds `.ssh/known_hosts` as the fixture's, or an empty `.ssh` when `trusts` is false.
fn home_named(base: &Path, name: &str, f: &common::SshdFixture, trusts: bool) -> PathBuf {
    let home = base.join(name);
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    if trusts {
        std::fs::copy(&f.known_hosts, home.join(".ssh/known_hosts")).unwrap();
    }
    home
}

/// The cancellation token of a caller that never cancels, and the controls that keep it alive.
fn caller_token() -> (CallControls<()>, CancellationToken) {
    let (controls, gate) = CallControls::new(&Arc::new(()));
    (controls, gate.token().clone())
}

/// The clone's outcome on the transport with `pairs` as the environment: `Ok` when it reached the fixture's commit,
/// else the error's words, whether the runtime refused to start or the trust check refused the server.
fn clone_with(f: &common::SshdFixture, pairs: &[(&str, &OsStr)], name: &str) -> Result<(), String> {
    let environment = EnvironmentSnapshot::from_os_pairs(
        pairs
            .iter()
            .map(|(variable, value)| (OsString::from(variable), value.to_os_string())),
    )
    .unwrap();
    let options = crate::TransportOptions {
        default_identity: Some(f.temp.path().join("client_ed25519").display().to_string()),
        ..Default::default()
    };
    let meta = crate::RequestMeta {
        request_id: name.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(options.clone()),
        ..Default::default()
    };
    let (_controls, token) = caller_token();
    let target = f.temp.path().join(format!("clone-{name}"));
    let (result, cleanup) = crate::transport_host::with_cancellable_local_transport(
        meta,
        "clone".into(),
        &environment,
        &token,
        |backend| {
            let backend = backend
                .with_transport(f.temp.path(), Some(&options))
                .map_err(|error| error.message)?
                .ok_or("no transport scope")?;
            backend
                .clone_repo(&url(f), &target)
                .map(drop)
                .map_err(|error| error.to_string())
        },
    );
    assert_eq!(cleanup.pending_local_work, 0, "{name}");
    result.map_err(|error| error.message)?
}

#[test]
fn the_home_contract_through_a_real_server_b03_and_b04() {
    let f = common::SshdFixture::new();
    commit(
        &git2::Repository::open_bare(&f.repository).unwrap(),
        "first",
    );
    let base = f.temp.path().join("homes");
    let good = home_named(&base, "good", &f, true);
    let bad = home_named(&base, "bad", &f, false);
    let missing = base.join("does not exist");
    let unicode = home_named(&base, "caf\u{e9} \u{65e5}\u{672c}\u{8a9e} dir", &f, true);
    let (good_os, bad_os, missing_os, unicode_os) = (
        good.as_os_str(),
        bad.as_os_str(),
        missing.as_os_str(),
        unicode.as_os_str(),
    );
    let trust_refusal = |result: &Result<(), String>, home: &Path, source: &str| {
        let words = result.as_ref().expect_err("trust was refused");
        assert!(words.contains("cannot trust the host key"), "{words}");
        assert!(
            words.contains(&home.join(".ssh").join("known_hosts").display().to_string()),
            "{words}"
        );
        assert!(
            words.contains(&format!("home came from {source}")),
            "{words}"
        );
    };

    // HOME names the home.
    assert_eq!(clone_with(&f, &[("HOME", good_os)], "home"), Ok(()));
    // B03: HOME unset, USERPROFILE alone.
    assert_eq!(
        clone_with(&f, &[("USERPROFILE", good_os)], "userprofile"),
        Ok(())
    );
    // An explicitly empty HOME refuses before any connection, though USERPROFILE is good.
    let empty = clone_with(
        &f,
        &[("HOME", OsStr::new("")), ("USERPROFILE", good_os)],
        "empty",
    );
    let words = empty.expect_err("an empty HOME was accepted");
    assert!(words.contains("HOME is set but empty"), "{words}");
    // B04: an existing first home without a good known_hosts refuses, though a later candidate is good.
    let first_bad = clone_with(
        &f,
        &[("HOME", bad_os), ("USERPROFILE", good_os)],
        "first-bad",
    );
    trust_refusal(&first_bad, &bad, "HOME");
    // A home with spaces and Unicode in its name works (1.0.17 fails on Windows).
    assert_eq!(
        clone_with(&f, &[("HOME", unicode_os)], "unicode"),
        Ok(()),
        "a Unicode home with spaces"
    );
    // Nothing names a home.
    let none = clone_with(&f, &[("PATH", OsStr::new("x"))], "none");
    let words = none.expect_err("a home was found");
    assert!(words.contains("no SSH home was found"), "{words}");

    // The rules of each platform: Windows skips a home that is not a directory and takes HOMEDRIVE and HOMEPATH; the
    // others take HOME as written, so a missing one is a home with no known_hosts.
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            assert_eq!(
                clone_with(&f, &[("HOME", missing_os), ("USERPROFILE", good_os)], "skipped"),
                Ok(()),
                "B04: a HOME that does not exist is skipped"
            );
            let text = good.to_str().unwrap();
            let (drive, path) = text.split_at(2);
            assert_eq!(
                clone_with(
                    &f,
                    &[("HOMEDRIVE", OsStr::new(drive)), ("HOMEPATH", OsStr::new(path))],
                    "homedrive"
                ),
                Ok(()),
                "B03: HOMEDRIVE and HOMEPATH"
            );
        } else {
            let as_written = clone_with(
                &f,
                &[("HOME", missing_os), ("USERPROFILE", good_os)],
                "as-written",
            );
            trust_refusal(&as_written, &missing, "HOME");
        }
    }
}

// Windows resolves a relative HOME against the current directory its driver captured, and keeps the absolute path: a
// worker that starts after the process directory moved still reads the original `known_hosts` (design section 3, the
// closure test). `with_local_transport` captures the directory beside the environment; the process directory is the
// process's, so the case runs in a child, whose `HOME` is the relative name `rel`.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        const NAME: &str = "candidate_relative_home_survives_a_change_of_the_process_directory";

        #[test]
        fn candidate_relative_home_survives_a_change_of_the_process_directory() {
            if std::env::var_os(host_context::CHILD).is_none() {
                host_context::run_in_child_with(module_path!(), NAME, &[("HOME", "rel".to_owned())]);
                return;
            }
            let f = common::SshdFixture::new();
            commit(&git2::Repository::open_bare(&f.repository).unwrap(), "first");
            let base = f.temp.path().join("relative");
            home_named(&base, "rel", &f, true);
            let decoys = f.temp.path().join("decoys");
            std::fs::create_dir_all(decoys.join("rel/.ssh")).unwrap();
            // The directory the driver captures is `base`, where `rel` is the trusting home.
            std::env::set_current_dir(&base).unwrap();
            let meta = crate::RequestMeta {
                request_id: "relative".into(),
                schema_version: "gwz.protocol/v0".into(),
                transport: Some(crate::TransportOptions {
                    default_identity: Some(f.temp.path().join("client_ed25519").display().to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let target = f.temp.path().join("clone-relative");
            let (result, cleanup) =
                crate::transport_host::with_local_transport(meta.clone(), "relative".into(), |backend| {
                    // The configuration is derived; now the process moves to a directory where `rel` holds no trust.
                    std::env::set_current_dir(&decoys).unwrap();
                    let backend = backend
                        .with_transport(f.temp.path(), meta.transport.as_ref())
                        .unwrap()
                        .unwrap();
                    backend
                        .clone_repo(&url(&f), &target)
                        .map(drop)
                        .map_err(|error| error.to_string())
                })
                .unwrap();
            assert_eq!(cleanup.pending_local_work, 0);
            assert_eq!(result, Ok(()));
        }
    }
}
