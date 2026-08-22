//! PROBE BRANCH ONLY (`probe/g15-gate-dump`) — test-only per-gate dump for the
//! Linux-runner g15 `root_preservation` class (GwzArmPreservationHandoffDiagnosis.md,
//! "Prescribed probe"). Never compiled outside `cfg(test)`; observes only, mutates
//! nothing, and touches no fault boundary (every `fault()` site lives on a
//! mutation path). Delete with the branch.

use super::*;

use std::fmt::Write as _;
use std::process::Command;
use std::sync::Once;

const TAG: &str = "PROBE-G15";

/// Emitted from the failing arm of `prepare_root_preservation_stash`, so every
/// one of the 29 runner failures names its own false gate on its own fixture.
pub(crate) fn dump_failed_prepare(
    backend: &Git2Backend,
    root: &Path,
    spec: &GitRootPreservationSpec,
) {
    dump(backend, root, spec, "prepare-refused");
}

/// Non-asserting dump of every `prepare_root_preservation_stash` gate.
pub(crate) fn dump(
    backend: &Git2Backend,
    root: &Path,
    spec: &GitRootPreservationSpec,
    label: &str,
) {
    environment_once();
    let mut out = String::new();
    let p = |out: &mut String, line: std::fmt::Arguments| {
        let _ = writeln!(out, "{TAG} [{label}] {line}");
    };

    p(&mut out, format_args!("==== begin gate dump ===="));
    p(&mut out, format_args!("root = {}", root.display()));
    p(
        &mut out,
        format_args!(
            "spec.attached_branch = {:?} attached_commit = {:?} restore_commit = {:?}",
            spec.attached_branch, spec.attached_commit, spec.restore_commit
        ),
    );
    p(
        &mut out,
        format_args!("spec.managed_marker_path = {:?}", spec.managed_marker_path),
    );

    // Gate 0 (error-only, not part of the boolean composite).
    p(
        &mut out,
        format_args!(
            "gate0.validate_spec = {}",
            match index::validate_spec(root, spec) {
                Ok(()) => "Ok".to_owned(),
                Err(error) => format!("ERR({error:?})"),
            }
        ),
    );

    // Gate 1: exact_head.
    p(
        &mut out,
        format_args!(
            "gate1.exact_head = {}",
            result(exact_head(
                backend,
                root,
                &spec.attached_branch,
                &spec.attached_commit
            ))
        ),
    );
    p(
        &mut out,
        format_args!("  head.observed  = {}", result(backend.head(root))),
    );
    p(
        &mut out,
        format_args!(
            "  head.expected  = branch {:?} commit {:?} is_detached false",
            spec.attached_branch, spec.attached_commit
        ),
    );
    let ref_name = format!("refs/heads/{}", spec.attached_branch);
    p(
        &mut out,
        format_args!(
            "  read_ref({ref_name}).observed = {}",
            result(backend.read_ref(root, &ref_name))
        ),
    );

    // Gate 2: repository_state.
    p(
        &mut out,
        format_args!(
            "gate2.repository_state = {} (expected Clean)",
            result(backend.repository_state(root))
        ),
    );

    // Gate 3: full_form_matches(handoff_form), leg by leg.
    let expected = &spec.handoff_form;
    let staging = parent::staging_name(
        spec,
        GitRootManagedFormName::AttachedClean,
        GitRootManagedFormName::Handoff,
    );
    p(
        &mut out,
        format_args!(
            "gate3.full_form_matches(handoff_form) = {}",
            result(full_form_matches(root, spec, expected))
        ),
    );
    p(&mut out, format_args!("  staging_name = {staging:?}"));
    for object in [
        GitRootManagedObject::MarkerWorktree,
        GitRootManagedObject::LockWorktree,
        GitRootManagedObject::Index,
        GitRootManagedObject::MarkerParentDirectory,
    ] {
        p(
            &mut out,
            format_args!(
                "  gate3.leg.{object:?} = {}",
                result(object_matches(root, spec, &staging, object, expected))
            ),
        );
    }

    // Gate 3 leg detail: MarkerWorktree / LockWorktree candidate bytes.
    p(
        &mut out,
        format_args!(
            "  expected.marker = {}",
            match &expected.marker {
                None => "None".to_owned(),
                Some(file) => format!("path {:?} bytes {}", file.path, escaped(&file.bytes)),
            }
        ),
    );
    p(
        &mut out,
        format_args!(
            "  expected.lock   = path {:?} bytes {}",
            expected.lock.path,
            escaped(&expected.lock.bytes)
        ),
    );
    p(
        &mut out,
        format_args!(
            "  observed.marker = {}",
            file_state(&root.join(&spec.managed_marker_path))
        ),
    );
    p(
        &mut out,
        format_args!(
            "  observed.lock   = {}",
            file_state(&root.join(&expected.lock.path))
        ),
    );

    // Gate 3 leg detail: the raw index chain (index_format::parse + index::observe).
    p(
        &mut out,
        format_args!("  expected.index.marker = {:?}", expected.index.marker),
    );
    p(
        &mut out,
        format_args!("  expected.index.lock   = {:?}", expected.index.lock),
    );
    match open_repo(root).and_then(|repo| index_format::read(&repo)) {
        Ok(raw) => {
            p(
                &mut out,
                format_args!("  observed.index.version = {}", raw.version),
            );
            for entry in &raw.entries {
                p(
                    &mut out,
                    format_args!(
                        "  observed.index.entry = path {:?} oid {} mode {:o} stage {} \
                         flags 0x{:04x} extended_flags 0x{:04x}",
                        String::from_utf8_lossy(&entry.path),
                        entry.object_id,
                        entry.mode,
                        entry.stage,
                        entry.flags,
                        entry.extended_flags
                    ),
                );
            }
        }
        Err(error) => p(&mut out, format_args!("  observed.index = ERR({error:?})")),
    }
    for line in hexdump(&std::fs::read(root.join(".git/index")).unwrap_or_default()) {
        p(&mut out, format_args!("  .git/index {line}"));
    }

    // Gate 3 leg detail: the marker-parent dirent chain.
    p(
        &mut out,
        format_args!(
            "  parent::observe(marker_path {:?}, staging {staging:?}) = {} \
             (expected marker_present {})",
            spec.managed_marker_path,
            result(parent::observe(root, &spec.managed_marker_path, &staging)),
            expected.marker.is_some()
        ),
    );
    p(
        &mut out,
        format_args!(
            "  dirents gwz.conf         = {}",
            dirents(&root.join("gwz.conf"))
        ),
    );
    p(
        &mut out,
        format_args!(
            "  dirents gwz.conf/markers = {}",
            dirents(&root.join(crate::artifact::MARKER_DIR))
        ),
    );

    // Gate 4: observe_boundary (the sole consumer of the git_directory policy).
    p(
        &mut out,
        format_args!(
            "gate4.observe_boundary = {}",
            result(files::observe_boundary(root, &spec.handoff_boundary))
        ),
    );
    p(
        &mut out,
        format_args!("  boundary.expected = {}", escaped(&spec.handoff_boundary)),
    );
    p(
        &mut out,
        format_args!(
            "  boundary.observed = {}",
            file_state(&root.join(".git/info/exclude"))
        ),
    );

    // Ref-file bytes.
    p(
        &mut out,
        format_args!(
            ".git/HEAD             = {}",
            file_state(&root.join(".git/HEAD"))
        ),
    );
    p(
        &mut out,
        format_args!(
            ".git/{ref_name} = {}",
            file_state(&root.join(".git").join(&ref_name))
        ),
    );
    p(
        &mut out,
        format_args!(
            ".git/packed-refs      = {}",
            file_state(&root.join(".git/packed-refs"))
        ),
    );
    p(
        &mut out,
        format_args!("statfs(root) = {}", statfs_line(root)),
    );
    p(&mut out, format_args!("==== end gate dump ===="));

    // One write: the four test threads interleave, and every line is TAG-prefixed
    // so `grep PROBE-G15` recovers the whole dump regardless.
    print!("{out}");
}

fn environment_once() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut out = String::new();
        for (name, program, args) in [
            ("uname", "uname", vec!["-a"]),
            ("git", "git", vec!["--version"]),
        ] {
            let _ = writeln!(out, "{TAG} [env] {name} = {}", command(program, &args));
        }
        let _ = writeln!(
            out,
            "{TAG} [env] tmpdir = {:?}",
            std::env::temp_dir().display()
        );
        print!("{out}");
    });
}

fn statfs_line(root: &Path) -> String {
    let root = root.to_string_lossy().into_owned();
    format!(
        "stat[{}] findmnt[{}]",
        command(
            "stat",
            &[
                "-f",
                "-c",
                "f_type=%t(%T) f_fsid=%i namelen=%l bsize=%s blocks=%b free=%f",
                &root,
            ],
        ),
        command(
            "findmnt",
            &["-no", "SOURCE,FSTYPE,OPTIONS", "--target", &root]
        ),
    )
}

fn command(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(output) => format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            if output.status.success() {
                String::new()
            } else {
                format!(
                    " <exit {:?}: {}>",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )
            }
        ),
        Err(error) => format!("<unavailable: {error}>"),
    }
}

fn result<T: std::fmt::Debug>(value: ModelResult<T>) -> String {
    match value {
        Ok(value) => format!("{value:?}"),
        Err(error) => format!("ERR({error:?})"),
    }
}

fn file_state(path: &Path) -> String {
    match std::fs::symlink_metadata(path) {
        Err(error) => format!("<{}>", error.kind().to_string()),
        Ok(metadata) => {
            let kind = if metadata.is_dir() {
                "dir"
            } else if metadata.is_symlink() {
                "symlink"
            } else {
                "file"
            };
            let bytes = std::fs::read(path).unwrap_or_default();
            format!(
                "{kind} len {} {}{}",
                metadata.len(),
                identity_of(&metadata),
                if metadata.is_dir() {
                    String::new()
                } else {
                    format!(" bytes {}", escaped(&bytes))
                }
            )
        }
    }
}

#[cfg(unix)]
fn identity_of(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt as _;
    format!(
        "dev {} ino {} mode {:o} nlink {}",
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.nlink()
    )
}

#[cfg(not(unix))]
fn identity_of(_metadata: &std::fs::Metadata) -> String {
    String::new()
}

fn dirents(path: &Path) -> String {
    let mut entries = match std::fs::read_dir(path) {
        Err(error) => return format!("<{}>", error.kind()),
        Ok(iterator) => iterator
            .map(|entry| match entry {
                Err(error) => format!("<{error}>"),
                Ok(entry) => format!(
                    "{:?}[{}]",
                    entry.file_name(),
                    match entry.metadata() {
                        Ok(metadata) => identity_of(&metadata),
                        Err(error) => format!("<{error}>"),
                    }
                ),
            })
            .collect::<Vec<_>>(),
    };
    entries.sort();
    format!("{} entries {:?}", entries.len(), entries)
}

fn escaped(bytes: &[u8]) -> String {
    let rendered = bytes
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte))
        .map(char::from)
        .collect::<String>();
    format!("(len {}) \"{rendered}\"", bytes.len())
}

fn hexdump(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let hex = chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii = chunk
                .iter()
                .map(|byte| {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        char::from(*byte)
                    } else {
                        '.'
                    }
                })
                .collect::<String>();
            format!("{:08x}  {hex:<47}  |{ascii}|", row * 16)
        })
        .collect()
}
