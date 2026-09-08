//! White-box Tier A tests for the native seam: the two errno tables, the
//! probe's one shortcut, the per-copy plan, and the platform wrapper itself
//! where this host has one.
//!
//! Every table is checked here whatever the host is, because every table is
//! compiled here (see [`super::classify`]) -- the classification is the
//! policy, and it must be readable and testable without the machine whose
//! call it describes. So is the block-clone geometry
//! ([`super::block_clone`]), which is arithmetic and needs no machine at all.
//! What a host cannot check is the binding: that `FICLONE` really returns
//! these errno for these filesystems is Linux CI's job, and that ReFS really
//! answers these Win32 codes is Windows CI's.

use std::fs;

use gwz_copy_contract::{CopyErrorCategory, CopyMode, CopyRequest, contract_tests::TempTree};

use super::*;
use crate::NativeMechanism;

// ------------------------------------------------------------ errno tables

#[cfg(unix)]
mod tables {
    use rustix::io::Errno;

    use super::*;
    use crate::native::classify::{self, Class};

    /// Every errno that means "this pair cannot be cloned; copy it
    /// ordinarily" -- and nothing else.
    #[test]
    fn clonefile_falls_back_only_for_an_unsupported_filesystem_or_a_cross_device_pair() {
        for errno in [Errno::NOTSUP, Errno::OPNOTSUPP, Errno::XDEV] {
            assert!(
                matches!(classify::clonefile(errno), Class::Unsupported(_)),
                "{errno} means clonefile cannot serve this pair"
            );
        }
        // Permission, space and I/O failures are errors, not "unsupported"
        // (design §4). `EINVAL` is among them here: for `clonefile` it means
        // invalid flags, a bug in the wrapper, not a filesystem that cannot
        // clone.
        for errno in [
            Errno::ACCESS,
            Errno::PERM,
            Errno::ROFS,
            Errno::EXIST,
            Errno::NOENT,
            Errno::NOTDIR,
            Errno::NOSPC,
            Errno::DQUOT,
        ] {
            assert_eq!(
                classify::clonefile(errno),
                Class::Failed(CopyErrorCategory::DestinationUnwritable),
                "{errno} is a destination failure"
            );
        }
        for errno in [
            Errno::INVAL,
            Errno::IO,
            Errno::BADF,
            Errno::ISDIR,
            Errno::LOOP,
            Errno::NAMETOOLONG,
        ] {
            assert_eq!(
                classify::clonefile(errno),
                Class::Failed(CopyErrorCategory::Io),
                "{errno} is a failure, not a missing capability"
            );
        }
    }

    /// The FICLONE table differs from the clonefile one in exactly the place
    /// the manual pages differ: `ioctl_ficlone(2)` documents `EINVAL` as "the
    /// filesystem does not support reflinking the ranges of the given files",
    /// so for this call -- and only this call -- it is a fallback.
    #[test]
    fn ficlone_reads_einval_and_enotty_as_a_filesystem_that_cannot_reflink() {
        for errno in [Errno::NOTSUP, Errno::OPNOTSUPP, Errno::XDEV] {
            assert!(matches!(classify::ficlone(errno), Class::Unsupported(_)));
        }
        for errno in [Errno::INVAL, Errno::NOTTY] {
            assert!(
                matches!(classify::ficlone(errno), Class::Unsupported(_)),
                "{errno} is how a filesystem without reflink answers FICLONE"
            );
            assert_eq!(
                classify::clonefile(errno),
                Class::Failed(CopyErrorCategory::Io),
                "{errno} means something else entirely to clonefile: each call is \
                 classified from its own manual page"
            );
        }
        for errno in [Errno::ACCESS, Errno::PERM, Errno::NOSPC, Errno::DQUOT] {
            assert_eq!(
                classify::ficlone(errno),
                Class::Failed(CopyErrorCategory::DestinationUnwritable)
            );
        }
        for errno in [Errno::IO, Errno::BADF, Errno::ISDIR, Errno::TXTBSY] {
            assert_eq!(
                classify::ficlone(errno),
                Class::Failed(CopyErrorCategory::Io),
                "{errno} is a real failure of the ioctl"
            );
        }
    }

    #[test]
    fn a_described_outcome_names_the_call_and_carries_the_errno() {
        let unsupported =
            classify::describe(classify::ficlone(Errno::XDEV), Errno::XDEV, "FICLONE");
        let Outcome::Unsupported(detail) = unsupported else {
            panic!("a cross-device pair falls back: {unsupported:?}");
        };
        assert!(detail.contains("FICLONE"), "{detail}");
        assert!(detail.contains("EXDEV"), "{detail}");

        let failed =
            classify::describe(classify::clonefile(Errno::NOSPC), Errno::NOSPC, "clonefile");
        assert_eq!(
            failed,
            Outcome::Failed(
                CopyErrorCategory::DestinationUnwritable,
                format!("clonefile failed: {}", Errno::NOSPC)
            )
        );
    }
}

// ------------------------------------------------------------------- probe

#[test]
fn the_probe_answers_unknown_whenever_it_cannot_rule_the_pair_out() {
    let tree = TempTree::new("r-probe-unit");
    let beside = tree.path().join("not-created-yet");
    let expected = if MECHANISM == NativeMechanism::None {
        // Nothing to probe for: the honest answer is that there is no path.
        NativeCapability::Unavailable
    } else {
        // One device, so nothing is ruled out; the operation decides.
        NativeCapability::Unknown
    };
    assert_eq!(probe(tree.path(), &beside), expected);
    assert_eq!(probe(tree.path(), tree.path()), expected);
    // A destination with no readable ancestor at all: an unreadable hint
    // rules nothing out, so the attempt is still made.
    assert_eq!(
        probe(tree.path(), Path::new("")),
        if MECHANISM == NativeMechanism::None {
            NativeCapability::Unavailable
        } else {
            NativeCapability::Unknown
        }
    );
    // A source that does not exist is admission's business, not the probe's.
    assert_eq!(
        probe(Path::new("/does/not/exist/xyzzy"), tree.path()),
        expected
    );
}

#[cfg(unix)]
#[test]
fn the_device_of_a_new_path_is_its_nearest_existing_ancestors() {
    use std::os::unix::fs::MetadataExt;

    let tree = TempTree::new("r-probe-device");
    let deep = tree.path().join("a/b/c/not-created-yet");
    assert_eq!(
        device_of_nearest_existing(&deep),
        Some(fs::metadata(tree.path()).unwrap().dev()),
        "a destination that does not exist lands on its nearest existing ancestor's device"
    );
    assert_eq!(
        device_of_nearest_existing(Path::new("")),
        None,
        "a path with no existing ancestor answers nothing at all"
    );
}

/// The one thing the probe rules out without copying. `/dev` is a separate
/// filesystem from the temporary directory on both hosts this ships to
/// (devfs on Apple targets, devtmpfs on Linux); where it is not, or where
/// there is no mechanism to rule out, the test says so and stops.
#[cfg(unix)]
#[test]
fn a_pair_on_two_devices_is_the_one_thing_the_probe_rules_out() {
    let tree = TempTree::new("r-probe-cross-device");
    let elsewhere = Path::new("/dev");
    if MECHANISM == NativeMechanism::None {
        eprintln!("skipped: no native mechanism is compiled in for this target");
        return;
    }
    if device_of(elsewhere) == device_of(tree.path()) || device_of(elsewhere).is_none() {
        eprintln!("skipped: this host has no second device to probe across");
        return;
    }
    assert_eq!(
        probe(elsewhere, tree.path()),
        NativeCapability::Unavailable,
        "no copy-on-write mechanism clones across devices, and that is knowable in advance"
    );
    // So the copy is planned ordinary from the start, and says why once,
    // rather than making one doomed attempt per file.
    let plan = Plan::for_request(&CopyRequest {
        source: elsewhere.to_path_buf(),
        destination: tree.path().join("copy"),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    });
    assert_eq!(plan.attempt, Attempt::Skip);
    assert!(
        plan.unavailable
            .is_some_and(|reason| reason.contains("different devices")),
        "{:?}",
        plan.unavailable
    );
}

// -------------------------------------------------------------------- plan

#[test]
fn only_an_auto_request_with_a_usable_mechanism_attempts_a_native_clone() {
    let tree = TempTree::new("r-plan");
    let plan_for = |mode| {
        Plan::for_request(&CopyRequest {
            source: tree.path().to_path_buf(),
            destination: tree.path().join("copy"),
            exclusions: Vec::new(),
            mode,
        })
    };
    let ordinary = plan_for(CopyMode::OrdinaryOnly);
    assert_eq!(ordinary.attempt, Attempt::Skip);
    assert_eq!(
        ordinary.unavailable, None,
        "an ordinary-only copy was never promised a native path, so it warns about nothing"
    );

    let auto = plan_for(CopyMode::Auto);
    if MECHANISM == NativeMechanism::None {
        assert_eq!(auto.attempt, Attempt::Skip);
        assert!(
            auto.unavailable
                .is_some_and(|reason| reason.contains("compiled")),
            "a build with no mechanism says so once: {:?}",
            auto.unavailable
        );
    } else {
        assert_eq!(auto.attempt, Attempt::Native);
        assert_eq!(
            auto.unavailable, None,
            "attempts are being made, so nothing is promised in advance"
        );
    }
    assert!(!Attempt::Skip.is_attempted());
    assert!(Attempt::Native.is_attempted());
}

// -------------------------------------------------------- platform wrapper

/// The wrapper itself, on this host: a clone lands the source's bytes at the
/// temporary name and nowhere else. This is the lowest-level assertion that
/// the native path really ran.
#[test]
fn the_platform_wrapper_clones_a_file_on_a_host_that_can() {
    if MECHANISM == NativeMechanism::None {
        eprintln!("skipped: no native mechanism is compiled in for this target");
        return;
    }
    let tree = TempTree::new("r-wrapper");
    let source_path = tree.file("source.bin", b"cloned bytes");
    let source = fs::File::open(&source_path).expect("the fixture opens");
    let temporary = tree.path().join(".gwz-refcopy.wrapper.tmp");

    match Attempt::Native.clone_regular_file(&source, &temporary) {
        Outcome::Cloned => {
            assert_eq!(fs::read(&temporary).unwrap(), b"cloned bytes");
            assert_eq!(
                fs::read(&source_path).unwrap(),
                b"cloned bytes",
                "the source is only ever read"
            );
        }
        Outcome::Unsupported(detail) => {
            eprintln!("skipped: this host's filesystem cannot clone: {detail}");
        }
        Outcome::Failed(category, detail) => {
            panic!("the wrapper failed on an ordinary file: {category:?}: {detail}");
        }
    }
}

/// A wrapper that cannot create its temporary reports a real failure, and
/// leaves nothing behind for the fallback to append to.
#[test]
fn a_wrapper_that_cannot_create_its_temporary_fails_and_leaves_nothing() {
    if MECHANISM == NativeMechanism::None {
        eprintln!("skipped: no native mechanism is compiled in for this target");
        return;
    }
    let tree = TempTree::new("r-wrapper-fail");
    let source = fs::File::open(tree.file("source.bin", b"bytes")).expect("the fixture opens");
    let temporary = tree.path().join("absent-directory/.gwz-refcopy.tmp");

    let outcome = Attempt::Native.clone_regular_file(&source, &temporary);
    assert!(
        matches!(
            outcome,
            Outcome::Failed(CopyErrorCategory::DestinationUnwritable, _)
        ),
        "a missing destination directory is a destination failure, not a missing capability: \
         {outcome:?}"
    );
    assert!(!temporary.exists(), "and nothing was created");
}
