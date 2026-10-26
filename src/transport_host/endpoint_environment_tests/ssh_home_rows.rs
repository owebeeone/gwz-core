use super::*;
use crate::git::endpoint::ssh_home::HomeSource;

fn home_of(pairs: &[(&str, &OsStr)]) -> Result<(PathBuf, HomeSource), crate::model::ModelError> {
    let (local, _) = endpoint_config(&snapshot(pairs))?;
    let ssh = local.ssh.expect("an SSH home was chosen");
    Ok((ssh.home, ssh.home_source))
}

#[test]
fn with_home_unset_the_userprofile_is_the_home() {
    // B03: a `HOME`-less environment works.
    let root = tempfile::TempDir::new().unwrap();
    let profile = root.path().join("profile with spaces");
    std::fs::create_dir(&profile).unwrap();
    let (home, source) = home_of(&[("USERPROFILE", profile.as_os_str())]).unwrap();
    assert_eq!((home, source), (profile, HomeSource::UserProfile));
}

#[test]
fn home_comes_first_and_says_it_is_home() {
    let root = tempfile::TempDir::new().unwrap();
    let (a, b) = (root.path().join("a"), root.path().join("b"));
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let (home, source) =
        home_of(&[("HOME", a.as_os_str()), ("USERPROFILE", b.as_os_str())]).unwrap();
    assert_eq!((home, source), (a, HomeSource::Home));
}

#[test]
fn an_empty_home_is_refused_with_words_that_name_it() {
    let root = tempfile::TempDir::new().unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let refused = home_of(&[
        ("HOME", OsStr::new("")),
        ("USERPROFILE", profile.as_os_str()),
    ])
    .unwrap_err();
    assert!(
        refused.message.contains("HOME is set but empty"),
        "{}",
        refused.message
    );
}

#[test]
fn no_home_at_all_is_refused_with_words_that_name_the_variables() {
    let refused = home_of(&[("PATH", OsStr::new("x"))]).unwrap_err();
    assert!(
        refused.message.contains("no SSH home was found") && refused.message.contains("HOME"),
        "{}",
        refused.message
    );
}

#[test]
fn the_runtimes_driver_knows_the_home_and_its_source_that_a_trust_message_prints() {
    // The driver words a host-key failure (design section 14, item 13, U8) and expands `~/` identities: the home
    // and its source are those of the endpoint, the one place `known_hosts` is read from.
    let root = tempfile::TempDir::new().unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let (local, https) =
        endpoint_config(&snapshot(&[("USERPROFILE", profile.as_os_str())])).unwrap();
    let runtime = TransportRuntime::with_https(local, https, HelperSlots::new()).unwrap();
    let home = runtime
        .driver_home_for_test()
        .expect("an SSH runtime names its home");
    assert_eq!((home.path, home.source), (profile, HomeSource::UserProfile));
}

// The rows of each platform's rules: Windows' extra candidates, its requirement that a candidate exist, and its
// relative home; and, elsewhere, a home taken as written, no relative home, and native bytes.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[test]
        fn homedrive_and_homepath_are_the_home_when_home_is_unset() {
            let root = tempfile::TempDir::new().unwrap();
            let profile = root.path().join("profile");
            let share = root.path().join("share");
            std::fs::create_dir(&profile).unwrap();
            std::fs::create_dir(&share).unwrap();
            let text = share.to_str().unwrap();
            let (drive, path) = text.split_at(2);
            let (home, source) = home_of(&[
                ("HOMEDRIVE", OsStr::new(drive)),
                ("HOMEPATH", OsStr::new(path)),
                ("USERPROFILE", profile.as_os_str()),
            ])
            .unwrap();
            assert_eq!((home, source), (share, HomeSource::HomeDrivePath));
        }

        #[test]
        fn a_home_that_is_not_a_directory_is_skipped_on_windows() {
            let root = tempfile::TempDir::new().unwrap();
            let profile = root.path().join("profile");
            std::fs::create_dir(&profile).unwrap();
            let (home, source) = home_of(&[
                ("HOME", root.path().join("missing").as_os_str()),
                ("USERPROFILE", profile.as_os_str()),
            ])
            .unwrap();
            assert_eq!((home, source), (profile, HomeSource::UserProfile));
        }

        #[test]
        fn a_relative_home_that_exists_is_kept_as_an_absolute_path() {
            // OQ7 (a): resolved against the current directory its driver captured, so that a later change of the
            // process directory cannot redirect trust.
            let root = tempfile::TempDir::new().unwrap();
            std::fs::create_dir(root.path().join("rel")).unwrap();
            let environment = snapshot(&[("HOME", OsStr::new("rel"))]);
            let (local, _) = endpoint_config_in(&environment, root.path()).unwrap();
            let ssh = local.ssh.unwrap();
            assert_eq!(ssh.home, root.path().join("rel"));
            assert_eq!(ssh.home_source, HomeSource::Home);
            // A driver that captured no directory leaves a relative home out.
            let refused = endpoint_config(&environment).err().unwrap();
            assert!(refused.message.contains("no SSH home was found"), "{}", refused.message);
        }
    } else if #[cfg(unix)] {
        #[test]
        fn a_relative_home_names_no_home_on_unix() {
            let refused = home_of(&[("HOME", OsStr::new("."))]).unwrap_err();
            assert!(
                refused.message.contains("no SSH home was found"),
                "{}",
                refused.message
            );
            // Even with a captured directory: a relative home is the other platforms' notion.
            let root = tempfile::TempDir::new().unwrap();
            std::fs::create_dir(root.path().join("rel")).unwrap();
            let refused = endpoint_config_in(&snapshot(&[("HOME", OsStr::new("rel"))]), root.path())
                .err()
                .unwrap();
            assert!(
                refused.message.contains("no SSH home was found"),
                "{}",
                refused.message
            );
        }

        #[test]
        fn a_home_that_is_not_utf8_reaches_the_endpoint_unchanged() {
            // TD6 (Unix review): native bytes, never a lossy conversion.
            use std::os::unix::ffi::{OsStrExt, OsStringExt};
            let name = OsString::from_vec(b"/home/\xff\xfe".to_vec());
            let (home, _) = home_of(&[("HOME", name.as_os_str())]).unwrap();
            assert_eq!(home.as_os_str().as_bytes(), name.as_bytes());
        }
    }
}
