//! TR2.7, CA bundles (gwz-core `dev-docs/GwzTransportReleasePlanAmendment.md`
//! §3.5): every certificate in the CA file that the snapshot's
//! `GIT_SSL_CAINFO` or `SSL_CERT_FILE` names is a root, beside the platform's
//! built-in roots, and a malformed block refuses the operation before any
//! connection opens. Each test clones through the production entry from the
//! disposable HTTPS fixture.

use super::https_tests;
use super::*;
use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::session_host::EnvironmentSnapshot;
use std::{
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

/// A certificate block whose base64 holds no certificate.
const MALFORMED: &[u8] =
    b"-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n";

#[test]
fn a_bundle_whose_second_certificate_issued_the_server_certificate_connects() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let bundle = [unrelated_ca(root.path(), "unrelated-ca"), server.ca.clone()].concat();
    let (result, cleanup) = clone(root.path(), &bundle, &server.url, "two-certificates");
    result
        .expect("the entry ran the clone")
        .expect("the bundle's second certificate verifies the fixture");
    assert!(server.connections() >= 1);
    assert_eq!(cleanup.pending_local_work, 0);
}

#[test]
fn a_bundle_with_one_malformed_block_is_refused_before_any_connection_opens() {
    let root = tempfile::TempDir::new().unwrap();
    let server = Served::start(root.path());
    let bundle = [server.ca.clone(), MALFORMED.to_vec()].concat();
    let (result, _) = clone(root.path(), &bundle, &server.url, "malformed-block");
    let refused = result.expect_err("the CA file refuses the operation");
    assert_eq!(refused.code, ErrorCode::InvalidRequest);
    assert_eq!(
        refused.message,
        "endpoint CA file has a malformed certificate block"
    );
    assert_eq!(server.connections(), 0, "no connection opened");
}

/// The reader alone: every certificate block, in the file's order, with the
/// text and other blocks around them ignored. A block that does not end, is
/// not base64 or holds no certificate refuses the whole file, and so does a
/// file without a certificate block.
#[test]
fn the_ca_file_yields_every_certificate_block_and_refuses_a_malformed_one() {
    use crate::git::endpoint::ca_bundle::{Refusal, certificates};
    let root = tempfile::TempDir::new().unwrap();
    let text = |name| String::from_utf8(unrelated_ca(root.path(), name)).unwrap();
    let (first, second) = (text("first-ca"), text("second-ca"));
    let der = |pem: &str| -> Vec<Vec<u8>> {
        let roots = certificates(pem.as_bytes()).unwrap();
        roots.iter().map(|root| root.to_der().unwrap()).collect()
    };
    let file = [
        "a comment before the first block\n",
        &first,
        "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\ntext between\n",
        &second.replace('\n', "\r\n"),
        "text after the last block",
    ]
    .concat();
    assert_eq!(der(&file), [der(&first), der(&second)].concat());

    let malformed = std::str::from_utf8(MALFORMED).unwrap();
    for (case, file, refusal) in [
        (
            "a block with no end",
            second.replace("-----END", "-----"),
            Refusal::Malformed,
        ),
        (
            "a body that is not base64",
            "-----BEGIN CERTIFICATE-----\n#!\n-----END CERTIFICATE-----\n".into(),
            Refusal::Malformed,
        ),
        (
            "a block with no certificate",
            malformed.into(),
            Refusal::Malformed,
        ),
        (
            "a good block, then a malformed one",
            first.clone() + malformed,
            Refusal::Malformed,
        ),
        ("an empty file", String::new(), Refusal::NoCertificate),
        (
            "text alone",
            "no certificate here\n".into(),
            Refusal::NoCertificate,
        ),
        (
            "a trusted certificate block, which is not a certificate block",
            first.replace("CERTIFICATE-----", "TRUSTED CERTIFICATE-----"),
            Refusal::NoCertificate,
        ),
    ] {
        assert_eq!(certificates(file.as_bytes()).err(), Some(refusal), "{case}");
    }
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        /// Set in the child of the roots test, to the directory holding the
        /// fixture's URL and the bundle.
        const ROOTS_CHILD: &str = "GWZ_TR2_7_ROOTS_CHILD";

        /// The CA file's certificates add to the platform's built-in roots,
        /// and never replace them. OpenSSL's default paths come from the
        /// process environment, so the clone that shows it runs in a child of
        /// this test binary that starts with `SSL_CERT_FILE` naming a
        /// temporary store which holds the fixture's CA: there the entry
        /// clones the fixture while the snapshot's `GIT_SSL_CAINFO` names a
        /// bundle without that CA. In this process, whose default paths hold
        /// no such CA, the same clone fails.
        #[test]
        fn the_bundle_adds_to_the_platform_roots() {
            if let Some(case) = std::env::var_os(ROOTS_CHILD) {
                let case = std::path::PathBuf::from(case);
                let url = std::fs::read_to_string(case.join("url")).unwrap();
                let bundle = std::fs::read(case.join("bundle.pem")).unwrap();
                let (result, _) = clone(&case, &bundle, &url, "default-paths-hold-the-ca");
                result
                    .expect("the entry ran the clone")
                    .expect("the default paths' CA verifies the fixture beside the bundle");
                return;
            }
            let root = tempfile::TempDir::new().unwrap();
            let server = Served::start(root.path());
            let bundle = unrelated_ca(root.path(), "bundle");
            std::fs::write(root.path().join("url"), &server.url).unwrap();
            std::fs::write(root.path().join("store.pem"), &server.ca).unwrap();
            let (result, _) = clone(root.path(), &bundle, &server.url, "default-paths-alone");
            assert!(
                result.expect("the entry ran the clone").is_err(),
                "the bundle alone does not verify the fixture"
            );
            let name = format!(
                "{}::the_bundle_adds_to_the_platform_roots",
                module_path!().split_once("::").unwrap().1
            );
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture", "--test-threads", "1"])
                .env(ROOTS_CHILD, root.path())
                .env("SSL_CERT_FILE", root.path().join("store.pem"))
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "the roots test's child failed:\n{stdout}\n{stderr}"
            );
            assert_eq!(
                stdout.matches(&format!("test {name} ... ok")).count(),
                1,
                "the child ran no single test:\n{stdout}"
            );
        }
    }
}

/// Clones `url` through the production entry, with a snapshot whose `HOME`
/// is an endpoint home under `root` and whose `GIT_SSL_CAINFO` names a file
/// that holds `bundle`.
pub(super) fn clone(
    root: &Path,
    bundle: &[u8],
    url: &str,
    name: &str,
) -> (ModelResult<ModelResult<()>>, CleanupReport) {
    let ca = root.join(format!("{name}.pem"));
    std::fs::write(&ca, bundle).unwrap();
    let environment = EnvironmentSnapshot::from_os_pairs([
        (
            "HOME".into(),
            https_tests::endpoint_home(root).into_os_string(),
        ),
        ("GIT_SSL_CAINFO".into(), ca.into_os_string()),
    ])
    .unwrap();
    let (_controls, gate) = CallControls::new(&Arc::new(()));
    let target = root.join(name);
    with_cancellable_local_transport(
        https_tests::meta(name),
        "clone".into(),
        &environment,
        gate.token(),
        |backend| backend.clone_repo(url, &target).map(|_| ()),
    )
}

/// A CA certificate, in PEM, that issued nothing the fixture serves; `name`
/// names its files in `dir` and its subject.
pub(super) fn unrelated_ca(dir: &Path, name: &str) -> Vec<u8> {
    let path = dir.join(format!("{name}.pem"));
    let output = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout"])
        .arg(dir.join(format!("{name}-key.pem")))
        .arg("-out")
        .arg(&path)
        .args(["-days", "2", "-subj", &format!("/CN=GWZ {name}")])
        .args(["-addext", "basicConstraints=critical,CA:TRUE"])
        .args(["-addext", "keyUsage=critical,keyCertSign,cRLSign"])
        .output()
        .unwrap();
    assert!(output.status.success(), "openssl made no CA certificate");
    std::fs::read(path).unwrap()
}

/// The HTTPS fixture serving a repository through `git http-backend`, on a
/// runtime and thread of its own, since the entry runs its own executor.
pub(super) struct Served {
    pub(super) url: String,
    /// The fixture's CA certificate, in PEM, which issued the server's.
    pub(super) ca: Vec<u8>,
    connections: Arc<AtomicUsize>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Served {
    pub(super) fn start(root: &Path) -> Self {
        let (repository, _) = https_tests::repository(root);
        let repository = Arc::new(repository);
        let (started, receiver) = mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let server = https_tests::fixture::Server::start(Arc::new(move |request| {
                    let repository = repository.clone();
                    Box::pin(
                        async move { https_tests::git_http_backend(repository, request).await },
                    )
                }))
                .await;
                started
                    .send((
                        server.url.clone(),
                        server.ca.clone(),
                        server.connections.clone(),
                    ))
                    .unwrap();
                let _ = stopped.await;
            });
        });
        let (url, ca, connections) = receiver.recv().unwrap();
        Self {
            url,
            ca,
            connections,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    /// The connections the fixture has accepted.
    pub(super) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
