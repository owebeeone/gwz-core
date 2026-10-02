//! What the transport setting's tests share: snapshots of explicit pairs,
//! configuration files, FIFOs, a workspace on disk, a guard for calls that
//! must not block, and `sh` runs of the commands §10 prints.

use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::artifact::{
    self, ArtifactSourceKind, ManifestArtifact, ManifestMember, WorkspaceHeader,
};
use crate::session_host::EnvironmentSnapshot;
use crate::test_support::TempDir;
use crate::{RequestMeta, Selection, WorkspaceRef};

/// A file holding `gwz.transport = native`.
pub(super) const NATIVE: &str = "[gwz]\n\ttransport = native\n";
/// A file holding `gwz.transport = gwz`.
pub(super) const GWZ: &str = "[gwz]\n\ttransport = gwz\n";

/// A snapshot holding exactly `pairs`, as a driver passes one.
pub(super) fn snapshot<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a OsStr)>,
) -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_os_pairs(
        pairs
            .into_iter()
            .map(|(name, value)| (OsString::from(name), value.to_os_string())),
    )
    .unwrap()
}

/// A snapshot whose only entry is `HOME`.
pub(super) fn home(home: &Path) -> EnvironmentSnapshot {
    snapshot([("HOME", home.as_os_str())])
}

/// Writes `contents` to `path`, creating its directories.
pub(super) fn write(path: &Path, contents: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Makes a FIFO at `path`. Opening it to read blocks until a writer opens it.
pub(super) fn fifo(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `name` is a NUL-terminated path that outlives the call.
    let made = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
    assert_eq!(made, 0, "mkfifo {}", path.display());
}

/// `call`'s result, or a failure if it has not returned within 20 seconds: a
/// call that opens a FIFO to read never returns.
pub(super) fn within<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(call());
    });
    match receiver.recv_timeout(Duration::from_secs(20)) {
        Ok(value) => value,
        Err(RecvTimeoutError::Timeout) => {
            panic!("blocked: the call opened a file it must not open")
        }
        Err(RecvTimeoutError::Disconnected) => panic!("the call panicked"),
    }
}

/// Runs `command` through `sh -c` in `cwd`, with `HOME` at `home` and git's
/// system file off, as a user would paste it.
pub(super) fn sh(command: &str, cwd: &Path, home: &Path) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

/// Asserts that `output` succeeded and returns its standard output's lines.
pub(super) fn lines(output: &Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// libgit2's process-wide home, which it fixed when it first initialised in
/// this process and against which it resolves a `~/` include.
pub(super) fn libgit2_home() -> PathBuf {
    // Any git2 call initialises libgit2 first.
    git2::Config::new().unwrap();
    let mut buffer = libgit2_sys::git_buf {
        ptr: std::ptr::null_mut(),
        reserved: 0,
        size: 0,
    };
    // SAFETY: GIT_OPT_GET_HOMEDIR (37 in libgit2's enum and libgit2-sys's)
    // writes the home into `buffer`, which libgit2 allocates and
    // `git_buf_dispose` frees, and nothing in gwz-core sets the home.
    let bytes = unsafe {
        let status = libgit2_sys::git_libgit2_opts(
            libgit2_sys::GIT_OPT_GET_HOMEDIR as libc::c_int,
            &mut buffer as *mut libgit2_sys::git_buf,
        );
        assert_eq!(status, 0, "GIT_OPT_GET_HOMEDIR");
        let bytes = std::slice::from_raw_parts(buffer.ptr as *const u8, buffer.size).to_vec();
        libgit2_sys::git_buf_dispose(&mut buffer);
        bytes
    };
    PathBuf::from(OsString::from_vec(bytes))
}

/// The workspace ID of every `Workspace`.
pub(super) const WORKSPACE_ID: &str = "ws_setting";

/// A workspace on disk: a manifest naming the members, and a `.git`
/// directory for the root and each member. The repositories hold only the
/// files a test writes; the scan opens none of them as a repository.
pub(super) struct Workspace {
    _dir: TempDir,
    pub(super) root: PathBuf,
}

impl Workspace {
    /// A workspace whose members are `(member ID, path)` pairs.
    pub(super) fn new(label: &str, members: &[(&str, &str)]) -> Self {
        let dir = TempDir::new(label);
        let root = dir.path().join("ws");
        fs::create_dir_all(root.join(".git")).unwrap();
        let manifest = ManifestArtifact {
            schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: WORKSPACE_ID.to_owned(),
            },
            members: members
                .iter()
                .map(|(id, path)| ManifestMember {
                    private: false,
                    id: (*id).to_owned(),
                    path: (*path).to_owned(),
                    source_kind: ArtifactSourceKind::Git,
                    source_id: "src_setting".to_owned(),
                    active: true,
                    desired: None,
                    remotes: Vec::new(),
                })
                .collect(),
        };
        artifact::write_manifest(&root, &manifest).unwrap();
        for (_, path) in members {
            fs::create_dir_all(root.join(path).join(".git")).unwrap();
        }
        Self { _dir: dir, root }
    }

    /// The `.git` directory of the member at `path`, or of the root for `""`.
    pub(super) fn git_dir(&self, path: &str) -> PathBuf {
        self.root.join(path).join(".git")
    }

    /// A request's metadata for this workspace that selects `targets`, or
    /// none, so that the operation's default applies.
    pub(super) fn meta(&self, targets: &[&str]) -> RequestMeta {
        RequestMeta {
            request_id: "req_setting".to_owned(),
            schema_version: "gwz.proto/v0".to_owned(),
            workspace: Some(WorkspaceRef {
                root: Some(self.root.to_str().unwrap().to_owned()),
                workspace_id: Some(WORKSPACE_ID.to_owned()),
            }),
            selection: (!targets.is_empty()).then(|| Selection {
                targets: targets.iter().map(|target| (*target).to_owned()).collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
