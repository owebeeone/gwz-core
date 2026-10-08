//! OpenSSL's default verify paths as the trust of a clone, from the disposable
//! HTTPS fixture (whose CA is in no system store): the fixture verifies when
//! the file or the directory the snapshot names holds its CA, and when they do
//! not it is refused, whatever else the machine trusts. The parity with
//! gwz 1.0.17, which the same variables narrow alike, is measured against
//! GitHub in the parity run's evidence.

use super::ca_bundle_tests::{Served, unrelated_ca};
use super::ca_trust_tests::snapshot;
use super::https_tests;
use super::*;
use crate::git::GitBackend;
use std::{
    ffi::OsStr,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
};

/// Set in the child of the CLI test, to the directory holding its case.
const CHILD: &str = "GWZ_CA_TRUST_CLI_CHILD";

/// The default verify paths of a clone: `file` and `dir` as the snapshot's
/// `SSL_CERT_FILE` and `SSL_CERT_DIR`, and the extra pairs beside them.
struct Trust<'a> {
    file: Option<&'a Path>,
    dir: Option<&'a Path>,
    extra: Vec<(&'a str, &'a OsStr)>,
}

fn trust<'a>(file: Option<&'a Path>, dir: Option<&'a Path>) -> Trust<'a> {
    Trust {
        file,
        dir,
        extra: Vec::new(),
    }
}

/// Clones `url` through the cancellable entry. The outer result is whether the
/// entry ran the clone, which a refusal of the configuration is not.
fn clone_with(root: &Path, trust: Trust, url: &str, name: &str) -> ModelResult<ModelResult<()>> {
    let home = https_tests::endpoint_home(root).into_os_string();
    let mut pairs = vec![("HOME", home.as_os_str())];
    if let Some(file) = trust.file {
        pairs.push(("SSL_CERT_FILE", file.as_os_str()));
    }
    if let Some(dir) = trust.dir {
        pairs.push(("SSL_CERT_DIR", dir.as_os_str()));
    }
    pairs.extend(trust.extra);
    let environment = snapshot(&pairs);
    let (_controls, gate) = CallControls::new(&Arc::new(()));
    let target = root.join(name);
    with_cancellable_local_transport(
        https_tests::meta(name),
        "clone".into(),
        &environment,
        gate.token(),
        |backend| backend.clone_repo(url, &target).map(|_| ()),
    )
    .0
}

fn write(path: PathBuf, content: &[u8]) -> PathBuf {
    std::fs::write(&path, content).unwrap();
    path
}

fn empty_dir(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir(&dir).unwrap();
    dir
}

/// A directory OpenSSL looks in, holding `pem` as a certificate file under a
/// hash name, by a link as `c_rehash` makes them.
fn store_with(root: &Path, name: &str, pem: &[u8]) -> PathBuf {
    let dir = empty_dir(root, name);
    let file = write(root.join(format!("{name}.crt")), pem);
    symlink(file, dir.join("0123abcd.0")).unwrap();
    dir
}

#[test]
fn the_file_of_ssl_cert_file_is_the_trust_and_another_cas_file_is_not() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let none = empty_dir(root.path(), "none");
    let right = write(root.path().join("right.pem"), &server.ca);
    let wrong = write(
        root.path().join("wrong.pem"),
        &unrelated_ca(root.path(), "wrong"),
    );
    clone_with(
        root.path(),
        trust(Some(&right), Some(&none)),
        &server.url,
        "right",
    )
    .expect("the entry ran the clone")
    .expect("the fixture's CA, the only trust, verifies the fixture");
    assert!(
        clone_with(
            root.path(),
            trust(Some(&wrong), Some(&none)),
            &server.url,
            "wrong"
        )
        .expect("the entry ran the clone")
        .is_err(),
        "an unrelated CA, the only trust, verifies the fixture"
    );
}

#[test]
fn the_directory_of_ssl_cert_dir_is_the_trust_beside_a_file_that_holds_no_ca() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let store = store_with(root.path(), "store", &server.ca);
    let wrong = write(
        root.path().join("wrong.pem"),
        &unrelated_ca(root.path(), "wrong"),
    );
    clone_with(
        root.path(),
        trust(Some(&wrong), Some(&store)),
        &server.url,
        "dir",
    )
    .expect("the entry ran the clone")
    .expect("a certificate in the directory verifies the fixture");
}

/// 1.0.17 refuses a file that holds no certificate only when it verifies
/// (OpenSSL reads it, finds nothing, and the verification fails); gwz does not
/// refuse the operation before any connection, and does not trust more.
#[test]
fn a_file_that_holds_no_certificate_is_a_failed_verification_not_a_refused_operation() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let none = empty_dir(root.path(), "none");
    for (case, content) in [
        ("empty", &b""[..]),
        ("garbage", &b"not a certificate\n"[..]),
        (
            "malformed",
            &b"-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n"
                [..],
        ),
    ] {
        let file = write(root.path().join(format!("{case}.pem")), content);
        let result = clone_with(
            root.path(),
            trust(Some(&file), Some(&none)),
            &server.url,
            case,
        )
        .unwrap_or_else(|refused| panic!("{case}: refused before any connection: {refused:?}"));
        assert!(result.is_err(), "{case}: verified with no root at all");
    }
    assert!(
        server.connections() >= 3,
        "each of them reached the fixture"
    );
}

#[test]
fn a_trusted_certificate_block_in_the_file_is_a_root_as_in_openssl() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let none = empty_dir(root.path(), "none");
    let trusted = String::from_utf8(server.ca.clone())
        .unwrap()
        .replace("CERTIFICATE-----", "TRUSTED CERTIFICATE-----");
    let file = write(root.path().join("trusted.pem"), trusted.as_bytes());
    clone_with(
        root.path(),
        trust(Some(&file), Some(&none)),
        &server.url,
        "trusted",
    )
    .expect("the entry ran the clone")
    .expect("a trusted certificate block verifies the fixture");
}

/// `GIT_SSL_CAINFO` is the endpoint's CA file beside the platform's roots
/// (TR2.7; 1.0.17 ignores it, a difference the operator decides): the fixture's
/// CA in it verifies the fixture while the default verify paths hold another.
#[test]
fn git_ssl_cainfo_adds_a_root_to_the_default_verify_paths() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let none = empty_dir(root.path(), "none");
    let wrong = write(
        root.path().join("wrong.pem"),
        &unrelated_ca(root.path(), "wrong"),
    );
    let cainfo = write(root.path().join("cainfo.pem"), &server.ca);
    let mut with_cainfo = trust(Some(&wrong), Some(&none));
    with_cainfo
        .extra
        .push(("GIT_SSL_CAINFO", cainfo.as_os_str()));
    clone_with(root.path(), with_cainfo, &server.url, "cainfo")
        .expect("the entry ran the clone")
        .expect("the CA file adds its root to the default verify paths");
}

/// The CLI's path, in children whose process environment holds the trust: the
/// command's environment is the snapshot. The default verify paths are the
/// process's, as in 1.0.17, and the children show the same trust as the
/// snapshots above.
#[test]
fn the_clis_trust_is_the_default_verify_paths_of_its_environment() {
    if let Some(case) = std::env::var_os(CHILD) {
        let case = PathBuf::from(case);
        let url = std::fs::read_to_string(case.join("url")).unwrap();
        let expect = std::fs::read_to_string(case.join("expect")).unwrap();
        let target = case.join("cli-clone");
        let _ = std::fs::remove_dir_all(&target);
        let (result, _) =
            with_local_transport(https_tests::meta("cli-clone"), "clone".into(), |backend| {
                backend.clone_repo(&url, &target).map(|_| ())
            })
            .expect("the entry ran the clone, no configuration refusal");
        assert_eq!(result.is_ok(), expect == "clone", "{expect}: {result:?}");
        return;
    }
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    std::fs::write(root.path().join("url"), &server.url).unwrap();
    let none = empty_dir(root.path(), "none");
    let right = write(root.path().join("right.pem"), &server.ca);
    let wrong = write(
        root.path().join("wrong.pem"),
        &unrelated_ca(root.path(), "wrong"),
    );
    let garbage = write(root.path().join("garbage.pem"), b"not a certificate\n");
    let store = store_with(root.path(), "store", &server.ca);
    let name = format!(
        "{}::the_clis_trust_is_the_default_verify_paths_of_its_environment",
        module_path!().split_once("::").unwrap().1
    );
    for (case, expect, file, dir) in [
        ("fixture's CA as the file", "clone", &right, &none),
        ("another CA as the file", "refuse", &wrong, &none),
        (
            "garbage as the file, the fixture's CA in the directory",
            "clone",
            &garbage,
            &store,
        ),
        ("garbage as the file", "refuse", &garbage, &none),
    ] {
        std::fs::write(root.path().join("expect"), expect).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture", "--test-threads", "1"])
            .env(CHILD, root.path())
            .env("SSL_CERT_FILE", file)
            .env("SSL_CERT_DIR", dir)
            .env("HOME", https_tests::endpoint_home(root.path()))
            .env_remove("GIT_SSL_CAINFO")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{case}: the CLI test's child failed:\n{stdout}\n{stderr}"
        );
        assert_eq!(
            stdout.matches(&format!("test {name} ... ok")).count(),
            1,
            "{case}: the child ran no single test:\n{stdout}"
        );
    }
}
