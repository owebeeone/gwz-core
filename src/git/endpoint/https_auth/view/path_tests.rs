//! The view's paths and the file reads behind them (plan step 4.3): the same rows on every platform, with the
//! Windows variants of what a path is. Git is the real one: the tests run `git config` and `git credential fill`.
use super::tests::{Parts, git, lookup, platform_environment};
use super::*;
use std::fs;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// A directory below the platform's root, as Git writes it.
        fn rooted(text: &str) -> String {
            format!("/{text}")
        }
        /// The spelling of `HOME` a Windows program usually gets is not Unix's.
        const HOME: &str = "HOME";
        const NO_SYSTEM: &str = "GIT_CONFIG_NOSYSTEM";
        /// `PATH` as a Unix Git is given it.
        const PATH: &str = "PATH";
    } else if #[cfg(windows)] {
        fn rooted(text: &str) -> String {
            format!("C:/{text}")
        }
        /// The capitalizations Windows programs are given: a name that Git reads whatever its case.
        const HOME: &str = "Home";
        const NO_SYSTEM: &str = "Git_Config_NoSystem";
        const PATH: &str = "Path";
    }
}

/// The path of `path` as the view carries it.
fn carried(path: &Path) -> Vec<u8> {
    paths::bytes(path).unwrap().to_vec()
}

/// What `file_worker::read` makes of `path`.
async fn read(path: &[u8]) -> Result<Vec<u8>, AuthError> {
    let config = Config {
        executable: git(),
        environment: Vec::new(),
    };
    let parts = Parts::new().await;
    let executable = git();
    let runner = parts.runner(&config, &executable, 5);
    super::super::file_worker::read(&runner, SecretBuffer(path.to_vec()))
        .await
        .map(|bytes| bytes.0.clone())
}

#[tokio::test]
async fn a_regular_file_is_read_whatever_its_directories_are_called() {
    let root = tempfile::tempdir().unwrap();
    for name in [
        "with spaces",
        "caf\u{e9} \u{65e5}\u{672c}\u{8a9e}",
        " padded",
    ] {
        let directory = root.path().join(name);
        fs::create_dir(&directory).unwrap();
        let file = directory.join("conf ig \u{fc}");
        fs::write(&file, name.as_bytes()).unwrap();
        assert_eq!(read(&carried(&file)).await.unwrap(), name.as_bytes());
    }
}

#[tokio::test]
async fn an_absent_file_reads_as_empty() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        read(&carried(&root.path().join("absent")))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_directory_a_device_and_a_relative_path_are_refused() {
    let root = tempfile::tempdir().unwrap();
    for refused in [
        carried(root.path()),
        super::tests::DEVICE.as_bytes().to_vec(),
        b"relative/config".to_vec(),
    ] {
        assert_eq!(
            read(&refused).await.err(),
            Some(AuthError::ConfigurationRefused),
            "{}",
            String::from_utf8_lossy(&refused)
        );
    }
}

#[tokio::test]
async fn a_file_over_the_source_limit_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("large");
    fs::write(&file, vec![b'#'; SOURCE_LIMIT + 1]).unwrap();
    assert_eq!(
        read(&carried(&file)).await.err(),
        Some(AuthError::ConfigurationRefused)
    );
    fs::write(&file, vec![b'#'; SOURCE_LIMIT]).unwrap();
    assert_eq!(read(&carried(&file)).await.unwrap().len(), SOURCE_LIMIT);
}

/// A path longer than the 260 characters of the classic Windows limit is read: the file is opened by a wide path.
#[tokio::test]
async fn a_path_beyond_the_classic_windows_limit_is_read() {
    let root = tempfile::tempdir().unwrap();
    let mut directory = root.path().to_path_buf();
    for _ in 0..6 {
        directory.push("d".repeat(60));
    }
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("config");
    fs::write(&file, b"[a]\n").unwrap();
    assert!(file.as_os_str().len() > 360);
    assert_eq!(read(&carried(&file)).await.unwrap(), b"[a]\n");
}

#[test]
fn a_relative_value_is_joined_to_its_anchor_with_one_separator() {
    let anchor = rooted("source");
    let joined = rooted("source/nested");
    assert!(framing::absolute(anchor.as_bytes(), b"nested").unwrap().0 == joined.as_bytes());
    assert!(
        framing::absolute(format!("{anchor}/").as_bytes(), b"nested")
            .unwrap()
            .0
            == joined.as_bytes()
    );
    assert!(
        framing::absolute(rooted("").as_bytes(), b"top").unwrap().0 == rooted("top").as_bytes()
    );
}

#[test]
fn an_absolute_value_is_kept_and_needs_no_anchor() {
    let value = rooted("elsewhere/file");
    assert!(framing::absolute(b"", value.as_bytes()).unwrap().0 == value.as_bytes());
    assert!(
        framing::absolute(rooted("source").as_bytes(), value.as_bytes())
            .unwrap()
            .0
            == value.as_bytes()
    );
}

#[test]
fn a_nul_a_missing_anchor_and_a_relative_anchor_are_refused() {
    let anchor = rooted("source");
    for (anchor, value) in [
        (anchor.as_bytes(), b"nul\0byte".as_slice()),
        (b"".as_slice(), b"relative".as_slice()),
        (b"relative".as_slice(), b"nested".as_slice()),
    ] {
        assert!(framing::absolute(anchor, value).is_err());
    }
}

#[test]
fn the_origin_of_a_configuration_file_is_the_path_git_wrote() {
    let origin = rooted("home/.gitconfig");
    let mut bytes = b"global\0file:".to_vec();
    bytes.extend_from_slice(origin.as_bytes());
    bytes.extend_from_slice(b"\0credential.helper\nx\0");
    let roots = discovery(&bytes).unwrap();
    assert!(matches!(&roots[..], [Root::File(path)] if path.0 == origin.as_bytes()));
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn paths_are_bytes_on_unix() {
            let value = b"/dir/\xff\xfe";
            assert!(framing::absolute(b"/", value).unwrap().0 == value);
            assert!(framing::absolute(b"/dir", b"\xff").unwrap().0 == b"/dir/\xff");
            let mut origin = b"global\0file:/relative/config\0credential.helper\nx\0".to_vec();
            assert!(matches!(&discovery(&origin).unwrap()[..], [Root::File(path)] if path.0 == b"/relative/config"));
            origin = b"global\0file:/dir/\xff\0credential.helper\nx\0".to_vec();
            assert!(discovery(&origin).is_ok());
        }
    } else if #[cfg(windows)] {
        /// A value that is not UTF-8 cannot become a wide path: it is refused, never converted with a replacement
        /// character that would name another file.
        #[test]
        fn a_path_that_is_not_utf8_is_refused_on_windows() {
            assert!(framing::absolute(b"C:/dir", b"\xff").is_err());
            assert!(framing::absolute(b"C:/\xff", b"nested").is_err());
            assert!(framing::absolute(b"", b"C:/\xff").is_err());
            assert!(discovery(b"global\0file:C:/\xff\0credential.helper\nx\0").is_err());
        }

        /// Only an absolute path is one: what has a root and no drive, or a drive and no root, depends on the
        /// current drive or directory, which Git decides and the view does not know.
        #[test]
        fn a_windows_value_that_is_neither_absolute_nor_plainly_relative_is_refused() {
            for value in [&b"/rooted"[..], br"\rooted", b"D:drive-relative"] {
                assert!(
                    framing::absolute(b"C:/dir", value).is_err(),
                    "{}",
                    String::from_utf8_lossy(value)
                );
            }
            assert!(framing::absolute(b"C:/dir", br"D:\absolute").unwrap().0 == br"D:\absolute");
        }

        #[test]
        fn windows_separators_and_unc_paths_are_kept() {
            assert!(framing::absolute(br"C:\source", br"sub\file").unwrap().0 == br"C:\source/sub\file");
            assert!(framing::absolute(b"C:/dir", b"//server/share/x").unwrap().0 == b"//server/share/x");
            assert!(framing::absolute(b"C:/dir", br"..\up").unwrap().0 == br"C:/dir/..\up");
        }

        /// Git writes the paths of an origin with forward slashes, and a relative one has no anchor to be
        /// relative to (Unix anchors it at `/`).
        #[test]
        fn a_relative_origin_is_refused_on_windows() {
            assert!(discovery(b"global\0file:relative/config\0credential.helper\nx\0").is_err());
        }

        #[test]
        fn a_home_that_is_not_text_is_refused_rather_than_converted() {
            use std::os::windows::ffi::OsStringExt;
            let mut units: Vec<u16> = "C:\\home".encode_utf16().collect();
            units.push(0xD800);
            let config = Config {
                executable: git(),
                environment: vec![("HOME".into(), OsString::from_wide(&units))],
            };
            assert_eq!(
                include_path(&config, b"C:/anchor", b"~/included").err(),
                Some(AuthError::ConfigurationRefused)
            );
        }

        /// Windows does not care about the case of `HOME`; neither does the view.
        #[test]
        fn the_home_key_is_found_whatever_its_case() {
            for key in ["HOME", "Home", "home"] {
                let config = Config {
                    executable: git(),
                    environment: vec![(key.into(), "C:\\Users\\a b".into())],
                };
                assert!(
                    include_path(&config, b"C:/anchor", b"~/included")
                        .unwrap()
                        .0
                        == b"C:\\Users\\a b/included"
                );
            }
        }

        /// A configuration value that is not UTF-8 cannot be an environment value on Windows, which is UTF-16: the
        /// lookup is refused before Git is asked for a credential.
        #[tokio::test]
        async fn a_view_that_is_not_utf8_is_refused() {
            use super::tests::config;
            let home = tempfile::tempdir().unwrap();
            let global = home.path().join("global");
            fs::write(&global, b"[spike]\n binary = \xff\n[credential]\n helper = !true\n").unwrap();
            assert_eq!(
                lookup(&config(home.path(), &global)).await.err(),
                Some(AuthError::ConfigurationRefused)
            );
        }
    }
}

/// The environment of a lookup that uses the real Git: the platform's spelling of the names, so that on Windows
/// every key is one a Windows program would see in another capitalization.
fn environment(home: &Path, extra: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<(OsString, OsString)> = vec![
        (HOME.into(), home.as_os_str().into()),
        (NO_SYSTEM.into(), "1".into()),
        (
            PATH.into(),
            std::env::var_os("PATH").expect("the test process has a PATH"),
        ),
    ];
    environment.extend(
        platform_environment()
            .into_iter()
            .filter(|(name, _)| !name.to_string_lossy().eq_ignore_ascii_case(PATH)),
    );
    environment.extend(
        extra
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    );
    environment
}

fn git_config(home: &Path, extra: &[(&str, &str)]) -> Config {
    Config {
        executable: PathBuf::from("git"),
        environment: environment(home, extra),
    }
}

/// The whole way: Git names the user's configuration files in the directory it reads them from, the view reads
/// them by those names and the includes by theirs, hands Git the result in the environment and Git runs the
/// helper it names. Every directory has a space and a non-ASCII character in its name, and so does the
/// username that travels in the environment.
#[tokio::test]
async fn a_helper_in_an_included_file_answers_from_directories_with_spaces_and_non_ascii_names() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("h\u{f3}me \u{65e5}\u{672c}\u{8a9e} dir");
    let included = home.join("inc dir \u{e9}");
    fs::create_dir_all(&included).unwrap();
    fs::write(
        home.join(".gitconfig"),
        "[include]\n\tpath = \"~/inc dir \u{e9}/helper.cfg\"\n",
    )
    .unwrap();
    fs::write(
        included.join("helper.cfg"),
        "[credential]\n\thelper = \"!f() { echo password=s3cret; }; f\"\n\tusername = \u{fc}n\u{ef}\n",
    )
    .unwrap();
    let secret = lookup(&git_config(&home, &[])).await.unwrap();
    assert_eq!(secret.username, "\u{fc}n\u{ef}".as_bytes());
    assert!(secret.password == b"s3cret");
}

/// A non-ASCII value in the snapshot reaches the helper as it is: the environment is wide on Windows and bytes
/// on Unix, and neither loses a character.
#[tokio::test]
async fn an_environment_value_with_non_ascii_characters_reaches_the_helper_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(".gitconfig"),
        "[credential]\n\thelper = \"!f() { echo username=$GWZ_PROBE; echo password=s3cret; }; f\"\n",
    )
    .unwrap();
    let probe = "caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1f600}";
    let secret = lookup(&git_config(&home, &[("GWZ_PROBE", probe)]))
        .await
        .unwrap();
    assert_eq!(secret.username, probe.as_bytes());
}

#[tokio::test]
async fn a_git_that_is_not_on_the_captured_path_is_reported_missing() {
    let empty = tempfile::tempdir().unwrap();
    let config = Config {
        executable: PathBuf::from("git"),
        environment: vec![(PATH.into(), empty.path().as_os_str().into())],
    };
    let error = lookup(&config).await.err().unwrap();
    assert_eq!(error, AuthError::MissingExecutable);
    assert_eq!(
        error.code(),
        gwz_transport::protocol::ErrorCode::Unavailable
    );
}
