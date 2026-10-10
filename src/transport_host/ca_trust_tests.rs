//! The platform's roots where the TLS backend is OpenSSL: OpenSSL's default
//! verify paths, as gwz 1.0.17's libgit2 loads them, and nothing else. A user
//! who narrows trust through `SSL_CERT_FILE` or `SSL_CERT_DIR` gets exactly
//! that narrower trust, never the system's roots again. Everywhere else the
//! platform's roots are the TLS backend's, and `SSL_CERT_FILE` is a CA file of
//! the endpoint like `GIT_SSL_CAINFO` (TR2.7).
//!
//! These tests read the files and directories (`verify_paths`), and the
//! endpoint configuration that names them. The clones that show the trust
//! are in `ca_trust_clone_tests`.

use super::endpoint_environment::endpoint_config;
use super::https_tests::unrelated_ca;
use crate::git::endpoint::verify_paths::Paths;
use crate::session_host::EnvironmentSnapshot;
use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

/// A snapshot of `pairs`, with a `HOME`, which an endpoint needs, unless they
/// name one.
pub(super) fn snapshot(pairs: &[(&str, &OsStr)]) -> EnvironmentSnapshot {
    let home = std::env::temp_dir().into_os_string();
    let default_home = pairs.iter().all(|(name, _)| *name != "HOME");
    EnvironmentSnapshot::from_os_pairs(
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), value.to_os_string()))
            .chain(default_home.then(|| (OsString::from("HOME"), home))),
    )
    .unwrap()
}

fn der(roots: &[native_tls::Certificate]) -> Vec<Vec<u8>> {
    roots.iter().map(|root| root.to_der().unwrap()).collect()
}

fn certificates_of(pem: &[u8]) -> Vec<Vec<u8>> {
    der(&crate::git::endpoint::ca_bundle::certificates(pem).unwrap())
}

/// A CA certificate in PEM, made under `dir`.
fn pem(dir: &Path, name: &str) -> String {
    String::from_utf8(unrelated_ca(dir, name)).unwrap()
}

fn write(path: &Path, content: impl AsRef<[u8]>) -> PathBuf {
    std::fs::write(path, content).unwrap();
    path.to_owned()
}

/// What `paths` hold: the file's and the directory's, as the DER of each root.
fn roots_of(file: Option<&Path>, dir: Option<&Path>) -> Vec<Vec<u8>> {
    let paths = Paths {
        file: file.map(Path::to_owned),
        dir: dir.map(Path::to_owned),
    };
    der(&paths.roots())
}

#[test]
fn a_named_path_that_exists_stands_and_the_standard_one_is_never_looked_for() {
    let root = tempfile::TempDir::new().unwrap();
    let file = write(&root.path().join("ca.pem"), "");
    let looked = Cell::new(0);
    let standard = || {
        looked.set(looked.get() + 1);
        Paths::default()
    };
    let paths = Paths::resolve(
        Some(file.as_os_str()),
        Some(root.path().as_os_str()),
        standard,
    );
    assert_eq!(paths.file, Some(file));
    assert_eq!(paths.dir, Some(root.path().to_owned()));
    assert_eq!(looked.get(), 0, "the standard locations were looked for");
}

/// 1.0.17's host replaces a variable that names nothing with the first
/// standard location, and leaves one that names a file alone, whatever the
/// file holds.
#[test]
fn a_named_path_that_does_not_exist_or_is_not_named_gives_the_standard_one() {
    let root = tempfile::TempDir::new().unwrap();
    let garbage = write(&root.path().join("garbage.pem"), "not a certificate");
    let missing = root.path().join("missing");
    let standard = || Paths {
        file: Some("/standard/file".into()),
        dir: Some("/standard/dir".into()),
    };
    let paths = Paths::resolve(Some(missing.as_os_str()), None, standard);
    assert_eq!(paths.file, Some("/standard/file".into()));
    assert_eq!(paths.dir, Some("/standard/dir".into()));
    let paths = Paths::resolve(
        Some(garbage.as_os_str()),
        Some(root.path().as_os_str()),
        standard,
    );
    assert_eq!(
        (paths.file, paths.dir),
        (Some(garbage.clone()), Some(root.path().to_owned())),
        "a file that holds garbage is still the named file"
    );
    let paths = Paths::resolve(Some(garbage.as_os_str()), None, standard);
    assert_eq!(
        (paths.file, paths.dir),
        (Some(garbage), Some("/standard/dir".into())),
        "only the path that is missing comes from the standard locations"
    );
}

#[test]
fn a_file_yields_every_certificate_in_order_and_each_once() {
    let root = tempfile::TempDir::new().unwrap();
    let (a, b, c) = (
        pem(root.path(), "a"),
        pem(root.path(), "b"),
        pem(root.path(), "c"),
    );
    let bundle = write(
        &root.path().join("bundle.pem"),
        format!("# comment\n{a}\ntext between\n{b}{a}{c}trailing text\n"),
    );
    let expected: Vec<_> = [&a, &b, &c]
        .iter()
        .flat_map(|pem| certificates_of(pem.as_bytes()))
        .collect();
    assert_eq!(roots_of(Some(&bundle), None), expected);
}

/// `openssl x509 -trustout` writes a `TRUSTED CERTIFICATE`, which OpenSSL
/// loads as a trusted certificate; the old `X509 CERTIFICATE` label is read too.
#[test]
fn trusted_and_old_style_certificate_blocks_are_certificates_here_as_in_openssl() {
    let root = tempfile::TempDir::new().unwrap();
    let a = pem(root.path(), "a");
    let b = pem(root.path(), "b");
    let file = write(
        &root.path().join("labels.pem"),
        format!(
            "{}{}",
            a.replace("CERTIFICATE-----", "TRUSTED CERTIFICATE-----"),
            b.replace("CERTIFICATE-----", "X509 CERTIFICATE-----"),
        ),
    );
    let expected = [certificates_of(a.as_bytes()), certificates_of(b.as_bytes())].concat();
    assert_eq!(roots_of(Some(&file), None), expected);
}

/// A file OpenSSL cannot read certificates from has no roots, and is no error.
#[test]
fn a_file_without_certificates_holds_no_roots_and_is_no_error() {
    let root = tempfile::TempDir::new().unwrap();
    let key = pem(root.path(), "key-only");
    let private_key = std::fs::read_to_string(root.path().join("key-only-key.pem")).unwrap();
    for (case, content) in [
        ("empty", String::new()),
        ("garbage", "\u{0}\u{1}not pem at all\n".into()),
        ("a private key", private_key),
        (
            "a block of base64 that is no certificate",
            "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n"
                .into(),
        ),
        (
            "a block that never ends",
            key.replace("-----END CERTIFICATE-----", ""),
        ),
    ] {
        let file = write(&root.path().join("none.pem"), content);
        assert!(roots_of(Some(&file), None).is_empty(), "{case}");
    }
    assert!(roots_of(Some(&root.path().join("missing")), None).is_empty());
    assert!(roots_of(Some(root.path()), None).is_empty(), "a directory");
}

/// OpenSSL keeps what it loaded before the block it could not.
#[test]
fn a_damaged_block_ends_the_file_and_keeps_what_came_before_it() {
    let root = tempfile::TempDir::new().unwrap();
    let (a, b) = (pem(root.path(), "a"), pem(root.path(), "b"));
    let damaged =
        "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n";
    let file = write(&root.path().join("damaged.pem"), format!("{a}{damaged}{b}"));
    assert_eq!(roots_of(Some(&file), None), certificates_of(a.as_bytes()));
    let unterminated = write(
        &root.path().join("unterminated.pem"),
        format!("{a}{}", b.replace("-----END CERTIFICATE-----", "")),
    );
    assert_eq!(
        roots_of(Some(&unterminated), None),
        certificates_of(a.as_bytes())
    );
}

#[test]
fn a_directory_yields_the_certificates_of_the_files_named_as_a_hash_and_a_number() {
    let root = tempfile::TempDir::new().unwrap();
    let store = root.path().join("store");
    std::fs::create_dir(&store).unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let certificate = |name: &str| pem(&source, name);
    let (first, second, third, linked) = (
        certificate("first"),
        certificate("second"),
        certificate("third"),
        certificate("linked"),
    );
    write(&store.join("0123abcd.0"), &first);
    write(&store.join("0123abcd.1"), &second);
    write(&store.join("deadbeef.12"), &third);
    symlink(
        write(&source.join("linked.crt"), &linked),
        store.join("00ff00ff.0"),
    )
    .unwrap();
    // Not names OpenSSL looks a certificate up by.
    for ignored in [
        "bundle.pem",
        "ca-certificates.crt",
        "0123abc.0",
        "0123abcde.0",
        "0123abcd.r0",
        "0123abcd.",
        "0123abcd",
        "0123abcg.0",
        "0123abcd.0.bak",
    ] {
        write(
            &store.join(ignored),
            certificate(&format!("ignored-{ignored}")),
        );
    }
    std::fs::create_dir(store.join("11111111.0")).unwrap();
    let expected = [first, second, third, linked]
        .iter()
        .flat_map(|pem| certificates_of(pem.as_bytes()))
        .collect::<Vec<_>>();
    let mut found = roots_of(None, Some(&store));
    found.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(found, expected);
}

#[test]
fn an_empty_or_missing_directory_holds_no_roots() {
    let root = tempfile::TempDir::new().unwrap();
    assert!(roots_of(None, Some(root.path())).is_empty());
    assert!(roots_of(None, Some(&root.path().join("missing"))).is_empty());
    assert!(roots_of(None, None).is_empty());
}

#[test]
fn the_file_comes_first_and_a_certificate_in_both_is_one_root() {
    let root = tempfile::TempDir::new().unwrap();
    let store = root.path().join("store");
    std::fs::create_dir(&store).unwrap();
    let (a, b) = (pem(root.path(), "a"), pem(root.path(), "b"));
    let file = write(&root.path().join("file.pem"), &a);
    write(&store.join("0123abcd.0"), format!("{b}{a}"));
    assert_eq!(
        roots_of(Some(&file), Some(&store)),
        [certificates_of(a.as_bytes()), certificates_of(b.as_bytes())].concat()
    );
}

/// On Debian the hash directory's files are links to the certificates the
/// bundle holds, so each root would be read twice. A directory file whose
/// certificate text is that of a block already read is not read again, and the
/// trust is what it was.
#[test]
fn a_directory_file_holding_a_block_already_read_is_skipped_unread() {
    let root = tempfile::TempDir::new().unwrap();
    let store = root.path().join("store");
    let source = root.path().join("source");
    std::fs::create_dir(&store).unwrap();
    std::fs::create_dir(&source).unwrap();
    let names = ["a", "b", "c"];
    let pems: Vec<_> = names.iter().map(|name| pem(&source, name)).collect();
    // The bundle is the files joined, and the hash links point at the files.
    let bundle = write(&root.path().join("bundle.pem"), pems.concat());
    for (index, pem) in pems.iter().enumerate() {
        let file = write(&source.join(format!("{}.crt", names[index])), pem);
        symlink(file, store.join(format!("0000000{index}.0"))).unwrap();
    }
    // A certificate the bundle lacks, which must still be a root.
    let only = pem(&source, "only");
    write(&store.join("0000000a.0"), &only);
    // The same certificate under a comment, and again as a second block
    // in a file of its own: read, and add nothing.
    write(&store.join("0000000b.0"), format!("# a note\n{}", pems[0]));
    write(&store.join("0000000c.0"), format!("{}{}", pems[1], pems[2]));
    let paths = Paths {
        file: Some(bundle),
        dir: Some(store),
    };
    let (roots, read) = paths.read();
    let expected = [pems.concat(), only]
        .iter()
        .flat_map(|pem| certificates_of(pem.as_bytes()))
        .collect::<Vec<_>>();
    assert_eq!(der(&roots), expected, "the trust is exactly what it was");
    // The bundle, the certificate it lacks and the two files that are not a
    // block of their own are read; the three links are not.
    assert_eq!((read.read, read.skipped), (4, 3));
}

#[test]
fn a_certificate_file_of_a_megabyte_or_more_is_read() {
    let root = tempfile::TempDir::new().unwrap();
    let a = pem(root.path(), "a");
    let padding = "# padding\n".repeat(150_000);
    assert!(padding.len() > 1024 * 1024);
    let file = write(&root.path().join("big.pem"), format!("{padding}{a}"));
    assert_eq!(roots_of(Some(&file), None), certificates_of(a.as_bytes()));
}

cfg_if::cfg_if! {
    if #[cfg(not(any(windows, target_vendor = "apple")))] {
        /// Where the backend is OpenSSL, `SSL_CERT_FILE` and `SSL_CERT_DIR` in
        /// the snapshot are the default verify paths, the platform's roots:
        /// the endpoint's CA roots hold none of them, and a file that is no
        /// CA file at all is not refused before any connection, as in 1.0.17.
        #[test]
        fn ssl_cert_file_and_dir_are_the_platform_roots_and_never_the_endpoints_ca_file() {
            let root = tempfile::TempDir::new().unwrap();
            let store = root.path().join("store");
            std::fs::create_dir(&store).unwrap();
            let a = pem(root.path(), "a");
            write(&store.join("0123abcd.0"), pem(root.path(), "in-dir"));
            let file = write(&root.path().join("file.pem"), &a);
            let environment = snapshot(&[
                ("SSL_CERT_FILE", file.as_os_str()),
                ("SSL_CERT_DIR", store.as_os_str()),
            ]);
            let (_, https) = endpoint_config(&environment).unwrap();
            assert!(https.tls.ca_roots.is_empty(), "decoded as a CA file");
            let platform = https.tls.platform_roots.expect("OpenSSL's default verify paths");
            let expected = [
                certificates_of(a.as_bytes()),
                certificates_of(&std::fs::read(store.join("0123abcd.0")).unwrap()),
            ]
            .concat();
            assert_eq!(der(&platform()), expected);

            let garbage = write(&root.path().join("garbage.pem"), "not pem");
            let environment = snapshot(&[("SSL_CERT_FILE", garbage.as_os_str())]);
            let (_, https) = endpoint_config(&environment).expect("a garbage file is no refusal");
            assert!(https.tls.ca_roots.is_empty());
        }

        /// The CA file of the endpoint stays `GIT_SSL_CAINFO`, read strictly and
        /// added to the platform's roots (TR2.7, an accepted decision whose
        /// difference from 1.0.17, which ignores the variable, is the
        /// operator's to revisit).
        #[test]
        fn git_ssl_cainfo_is_still_the_endpoints_ca_file_beside_the_platform_roots() {
            let root = tempfile::TempDir::new().unwrap();
            let a = pem(root.path(), "a");
            let cainfo = write(&root.path().join("cainfo.pem"), &a);
            let empty = root.path().join("empty-dir");
            std::fs::create_dir(&empty).unwrap();
            let environment = snapshot(&[
                ("GIT_SSL_CAINFO", cainfo.as_os_str()),
                ("SSL_CERT_FILE", cainfo.as_os_str()),
                ("SSL_CERT_DIR", empty.as_os_str()),
            ]);
            let (_, https) = endpoint_config(&environment).unwrap();
            assert_eq!(der(&https.tls.ca_roots), certificates_of(a.as_bytes()));
            assert!(https.tls.platform_roots.is_some());
            let malformed = write(
                &root.path().join("malformed.pem"),
                "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n",
            );
            let environment = snapshot(&[("GIT_SSL_CAINFO", malformed.as_os_str())]);
            assert_eq!(
                endpoint_config(&environment).err().unwrap().message,
                "endpoint CA file has a malformed certificate block"
            );
        }
    } else {
        /// Where the backend is Security.framework or Schannel, `SSL_CERT_FILE`
        /// is a CA file of the endpoint as `GIT_SSL_CAINFO` is, added to the
        /// platform's roots, and the endpoint names no other roots (TR2.7).
        #[test]
        fn ssl_cert_file_is_a_ca_file_of_the_endpoint_beside_the_backends_own_roots() {
            let root = tempfile::TempDir::new().unwrap();
            let a = pem(root.path(), "a");
            let file = write(&root.path().join("file.pem"), &a);
            let environment = snapshot(&[("SSL_CERT_FILE", file.as_os_str())]);
            let (_, https) = endpoint_config(&environment).unwrap();
            assert_eq!(der(&https.tls.ca_roots), certificates_of(a.as_bytes()));
            assert!(https.tls.platform_roots.is_none());
            let garbage = write(&root.path().join("garbage.pem"), "not pem");
            let environment = snapshot(&[("SSL_CERT_FILE", garbage.as_os_str())]);
            assert_eq!(
                endpoint_config(&environment).err().unwrap().message,
                "endpoint CA file has no certificate"
            );
        }
    }
}
