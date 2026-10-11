//! The B03 and B04 rows of the home contract as pure tests (plan step 3.1, "tests first"): the candidates in
//! order, the empty ones, the existence probe, the captured current directory, native paths. The same rows run
//! through the fixture in `transport_host`'s endpoint tests and on the Windows host.
use super::*;
use std::collections::BTreeSet;
use std::ffi::OsStr;

/// Variables that are exactly the pairs given, and a file system that holds exactly the directories given.
struct World {
    variables: Vec<(&'static str, OsString)>,
    directories: BTreeSet<PathBuf>,
}

impl World {
    fn new(variables: &[(&'static str, &str)], directories: &[&str]) -> Self {
        Self {
            variables: variables
                .iter()
                .map(|(name, value)| (*name, OsString::from(value)))
                .collect(),
            directories: directories.iter().map(PathBuf::from).collect(),
        }
    }
}

impl Variables for World {
    fn get(&self, name: &str) -> Option<OsString> {
        self.variables
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, value)| value.clone())
    }
}

impl Probe for World {
    fn is_directory(&self, path: &Path) -> bool {
        self.directories.contains(path)
    }
}

// Absolute paths in a form that is absolute on the platform the test runs on.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        const A: &str = "C:\\home\\a";
        const B: &str = "C:\\home\\b";
        const C: &str = "C:\\home\\c";
        const DRIVE: &str = "C:";
        const PATH_PART: &str = "\\home\\b";
        const CWD: &str = "C:\\work\\dir";
        const UNDER_CWD: &str = "C:\\work\\dir\\rel";
        /// A name with spaces, Unicode and a character outside the Basic Multilingual Plane.
        const NAMED: &str = "C:\\dir with spaces\\caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1f600}";
    } else {
        const A: &str = "/home/a";
        const B: &str = "/home/b";
        const C: &str = "/home/c";
        const DRIVE: &str = "";
        const PATH_PART: &str = "/home/b";
        const CWD: &str = "/work/dir";
        const UNDER_CWD: &str = "/work/dir/rel";
        /// A name with spaces, Unicode and a character outside the Basic Multilingual Plane.
        const NAMED: &str = "/dir with spaces/caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1f600}";
    }
}

fn windows(world: &World) -> Result<Home, Refusal> {
    resolve(Rules::Windows, world, Some(Path::new(CWD)), world)
}

fn unix(world: &World) -> Result<Home, Refusal> {
    resolve(Rules::Unix, world, Some(Path::new(CWD)), world)
}

fn chosen(path: &str, source: HomeSource) -> Result<Home, Refusal> {
    Ok(Home {
        path: PathBuf::from(path),
        source,
    })
}

#[test]
fn windows_candidates_are_tried_in_libgit2s_order() {
    let all = World::new(
        &[
            ("HOME", A),
            ("HOMEDRIVE", DRIVE),
            ("HOMEPATH", PATH_PART),
            ("USERPROFILE", C),
        ],
        &[A, B, C],
    );
    assert_eq!(windows(&all), chosen(A, HomeSource::Home));
    // HOME names no directory: HOMEDRIVE and HOMEPATH are next.
    let second = World::new(
        &[
            ("HOME", A),
            ("HOMEDRIVE", DRIVE),
            ("HOMEPATH", PATH_PART),
            ("USERPROFILE", C),
        ],
        &[B, C],
    );
    assert_eq!(windows(&second), chosen(B, HomeSource::HomeDrivePath));
    // Neither: USERPROFILE.
    let third = World::new(
        &[
            ("HOME", A),
            ("HOMEDRIVE", DRIVE),
            ("HOMEPATH", PATH_PART),
            ("USERPROFILE", C),
        ],
        &[C],
    );
    assert_eq!(windows(&third), chosen(C, HomeSource::UserProfile));
}

#[test]
fn an_unset_home_works_through_homedrive_and_homepath_or_through_userprofile_alone() {
    // B03, RS/b04-home-unset-homedrive-homepath.
    let drive_path = World::new(&[("HOMEDRIVE", DRIVE), ("HOMEPATH", PATH_PART)], &[B]);
    assert_eq!(windows(&drive_path), chosen(B, HomeSource::HomeDrivePath));
    // B03, RS/b04-userprofile-only.
    let profile = World::new(&[("USERPROFILE", C)], &[C]);
    assert_eq!(windows(&profile), chosen(C, HomeSource::UserProfile));
}

#[test]
fn homedrive_without_homepath_is_no_candidate() {
    let half = World::new(&[("HOMEDRIVE", DRIVE), ("USERPROFILE", C)], &[B, C]);
    assert_eq!(windows(&half), chosen(C, HomeSource::UserProfile));
    let other_half = World::new(&[("HOMEPATH", PATH_PART), ("USERPROFILE", C)], &[B, C]);
    assert_eq!(windows(&other_half), chosen(C, HomeSource::UserProfile));
}

#[test]
fn a_candidate_that_is_not_an_existing_directory_is_skipped() {
    // RS/b04-relative-home-nonexistent-good-fallback's absolute twin.
    let missing_first = World::new(&[("HOME", A), ("USERPROFILE", C)], &[C]);
    assert_eq!(windows(&missing_first), chosen(C, HomeSource::UserProfile));
    // Nothing exists: no home, not the first candidate.
    let nothing = World::new(&[("HOME", A), ("USERPROFILE", C)], &[]);
    assert_eq!(windows(&nothing), Err(Refusal::NoHome(Rules::Windows)));
    // Nothing set.
    let bare = World::new(&[], &[A]);
    assert_eq!(windows(&bare), Err(Refusal::NoHome(Rules::Windows)));
}

#[test]
fn the_first_existing_home_is_the_home_and_a_later_good_one_is_never_tried() {
    // RS/b04-first-existing-home-*: a home with no good known_hosts refuses; it does not fall through to the next.
    // The choice sees only directories, so the first that exists wins whatever the later ones hold.
    let both = World::new(&[("HOME", A), ("USERPROFILE", C)], &[A, C]);
    assert_eq!(windows(&both), chosen(A, HomeSource::Home));
}

#[test]
fn an_explicitly_empty_home_refuses_even_when_a_later_candidate_is_good() {
    // RS/b04-empty-home-good-fallback.
    let empty = World::new(&[("HOME", ""), ("USERPROFILE", C)], &[C]);
    assert_eq!(windows(&empty), Err(Refusal::EmptyHome));
    assert_eq!(unix(&empty), Err(Refusal::EmptyHome));
    // Unset is another case, and succeeds.
    let unset = World::new(&[("USERPROFILE", C)], &[C]);
    assert_eq!(windows(&unset), chosen(C, HomeSource::UserProfile));
}

#[test]
fn an_empty_candidate_other_than_home_is_skipped() {
    let world = World::new(
        &[("HOMEDRIVE", ""), ("HOMEPATH", ""), ("USERPROFILE", C)],
        &[C],
    );
    assert_eq!(windows(&world), chosen(C, HomeSource::UserProfile));
    let empty_profile = World::new(&[("USERPROFILE", "")], &[C]);
    assert_eq!(
        windows(&empty_profile),
        Err(Refusal::NoHome(Rules::Windows))
    );
}

#[test]
fn a_relative_home_that_exists_under_the_captured_directory_is_used_as_that_absolute_path() {
    // RS/b04-relative-home-exists-under-cwd.
    let relative = World::new(&[("HOME", "rel")], &[UNDER_CWD]);
    assert_eq!(windows(&relative), chosen(UNDER_CWD, HomeSource::Home));
}

#[test]
fn a_relative_home_that_does_not_exist_is_skipped_and_without_a_captured_directory_it_is_skipped() {
    // RS/b04-relative-home-nonexistent-good-fallback.
    let missing = World::new(&[("HOME", "rel"), ("USERPROFILE", C)], &[C]);
    assert_eq!(windows(&missing), chosen(C, HomeSource::UserProfile));
    // The directory it would name exists, but no current directory was captured to anchor it.
    let world = World::new(&[("HOME", "rel"), ("USERPROFILE", C)], &[UNDER_CWD, C]);
    assert_eq!(
        resolve(Rules::Windows, &world, None, &world),
        chosen(C, HomeSource::UserProfile)
    );
}

#[test]
fn the_choice_is_made_from_the_captured_directory_and_nothing_else() {
    // The captured directory is an argument: a different one names a different home, and the process's own is
    // never read by the pure choice.
    let world = World::new(&[("HOME", "rel")], &[UNDER_CWD]);
    assert_eq!(windows(&world), chosen(UNDER_CWD, HomeSource::Home));
    assert_eq!(
        resolve(Rules::Windows, &world, Some(Path::new(A)), &world),
        Err(Refusal::NoHome(Rules::Windows))
    );
}

#[test]
fn a_candidate_with_a_nul_is_malformed_and_refuses_before_a_later_candidate_is_tried() {
    let world = World::new(&[("HOME", "a\0b"), ("USERPROFILE", C)], &[C]);
    assert_eq!(windows(&world), Err(Refusal::Malformed(HomeSource::Home)));
    assert_eq!(unix(&world), Err(Refusal::Malformed(HomeSource::Home)));
    let profile = World::new(&[("USERPROFILE", "c\0d")], &[]);
    assert_eq!(
        windows(&profile),
        Err(Refusal::Malformed(HomeSource::UserProfile))
    );
    let drive = World::new(&[("HOMEDRIVE", "C:"), ("HOMEPATH", "\0")], &[]);
    assert_eq!(
        windows(&drive),
        Err(Refusal::Malformed(HomeSource::HomeDrivePath))
    );
}

#[test]
fn unicode_and_spaces_are_kept_exactly() {
    // RS/x10-unicode-and-spaces: the path is not converted, so the home that 1.0.17 chose and then failed to open
    // is the home the transport opens.
    let world = World::new(&[("HOME", NAMED)], &[NAMED]);
    assert_eq!(windows(&world), chosen(NAMED, HomeSource::Home));
    assert_eq!(windows(&world).unwrap().path.as_os_str(), OsStr::new(NAMED));
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn a_name_that_is_not_utf8_survives_on_unix() {
            use std::os::unix::ffi::{OsStrExt, OsStringExt};
            // TD6: native bytes through to the file operations, never a lossy conversion.
            let name = OsString::from_vec(b"/home/\xff\xfe".to_vec());
            let world = World {
                variables: vec![("HOME", name.clone())],
                directories: BTreeSet::new(),
            };
            let home = unix(&world).unwrap();
            assert_eq!(home.path.as_os_str().as_bytes(), name.as_bytes());
        }
    } else if #[cfg(windows)] {
        #[test]
        fn a_name_that_is_not_unicode_survives_on_windows() {
            use std::os::windows::ffi::{OsStrExt, OsStringExt};
            // A lone surrogate is a legal Windows name. It must not be replaced by U+FFFD on the way.
            let mut units: Vec<u16> = OsStr::new("C:\\home\\").encode_wide().collect();
            units.push(0xD800);
            let name = OsString::from_wide(&units);
            let world = World {
                variables: vec![("HOME", name.clone())],
                directories: [PathBuf::from(&name)].into_iter().collect(),
            };
            let home = windows(&world).unwrap();
            assert_eq!(home.path.as_os_str(), name);
        }
    }
}

#[test]
fn elsewhere_home_is_taken_as_written_when_absolute_without_asking_whether_it_exists() {
    let world = World::new(&[("HOME", A)], &[]);
    assert_eq!(unix(&world), chosen(A, HomeSource::Home));
    // A relative one names no home on these rules, whether or not it exists under the directory.
    let relative = World::new(&[("HOME", "rel")], &[UNDER_CWD]);
    assert_eq!(unix(&relative), Err(Refusal::NoHome(Rules::Unix)));
}

#[test]
fn elsewhere_userprofile_is_the_fallback_and_homedrive_is_not_a_candidate() {
    // Operator, 2026-10-11: with no HOME, `~/` falls back to USERPROFILE.
    let profile = World::new(&[("USERPROFILE", C)], &[]);
    assert_eq!(unix(&profile), chosen(C, HomeSource::UserProfile));
    let drive_path = World::new(&[("HOMEDRIVE", DRIVE), ("HOMEPATH", PATH_PART)], &[B]);
    assert_eq!(unix(&drive_path), Err(Refusal::NoHome(Rules::Unix)));
    // HOME wins when it is set.
    let both = World::new(&[("HOME", A), ("USERPROFILE", C)], &[]);
    assert_eq!(unix(&both), chosen(A, HomeSource::Home));
}

#[test]
fn the_refusals_say_which_variable_needs_correcting() {
    let empty = Refusal::EmptyHome.to_string();
    assert!(empty.contains("HOME is set but empty"), "{empty}");
    let malformed = Refusal::Malformed(HomeSource::UserProfile).to_string();
    assert!(
        malformed.contains("USERPROFILE does not hold a usable path"),
        "{malformed}"
    );
    let pair = Refusal::Malformed(HomeSource::HomeDrivePath).to_string();
    assert!(pair.contains("HOMEDRIVE and HOMEPATH"), "{pair}");
    let none = Refusal::NoHome(Rules::Windows).to_string();
    assert!(
        none.contains("HOME, HOMEDRIVE and HOMEPATH, or USERPROFILE") && none.contains("existing"),
        "{none}"
    );
    let elsewhere = Refusal::NoHome(Rules::Unix).to_string();
    assert!(
        elsewhere.contains("HOME is unset or not an absolute path"),
        "{elsewhere}"
    );
}

#[test]
fn the_disk_probe_answers_for_a_directory_a_file_and_a_missing_path() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(Disk.is_directory(directory.path()));
    assert!(!Disk.is_directory(&file));
    assert!(!Disk.is_directory(&directory.path().join("missing")));
}

#[test]
fn a_snapshot_is_a_set_of_variables_under_its_platforms_name_rules() {
    let snapshot = EnvironmentSnapshot::from_os_pairs([
        (OsString::from("HOME"), OsString::from(A)),
        (OsString::from("USERPROFILE"), OsString::from("")),
    ])
    .unwrap();
    assert_eq!(Variables::get(&snapshot, "HOME"), Some(OsString::from(A)));
    assert_eq!(
        Variables::get(&snapshot, "USERPROFILE"),
        Some(OsString::new())
    );
    assert_eq!(Variables::get(&snapshot, "HOMEPATH"), None);
}

#[test]
fn the_whole_choice_runs_over_the_disk_through_a_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("profile dir");
    std::fs::create_dir_all(&real).unwrap();
    let missing = root.path().join("missing");
    let snapshot = EnvironmentSnapshot::from_os_pairs([
        (OsString::from("HOME"), missing.into_os_string()),
        (OsString::from("USERPROFILE"), real.clone().into_os_string()),
    ])
    .unwrap();
    // On Windows the missing HOME is skipped (libgit2's rule); elsewhere HOME is taken as written.
    let home = resolve(PLATFORM, &snapshot, Some(root.path()), &Disk).unwrap();
    match PLATFORM {
        Rules::Windows => assert_eq!((home.path, home.source), (real, HomeSource::UserProfile)),
        Rules::Unix => assert_eq!(home.source, HomeSource::Home),
    }
}
