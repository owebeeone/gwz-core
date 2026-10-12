//! What the transport setting's tests share: snapshots of explicit pairs,
//! configuration files, a workspace on disk and, where the platform has them, FIFOs, a guard for calls that must not
//! block, and `sh` runs of the commands §10 prints.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

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

// The helpers of the rows that are Unix's by nature (operator decision D9, 2026-10-12): a FIFO is a file in a directory
// only there (a Windows named pipe lives in its own namespace), and libgit2's home is walked to from the root by
// components that Windows' drives do not share.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::Duration;

        /// Makes a FIFO at `path`. Opening it to read blocks until a writer opens it.
        pub(super) fn fifo(path: &Path) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
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
                    libgit2_sys::GIT_OPT_GET_HOMEDIR as std::ffi::c_int,
                    &mut buffer as *mut libgit2_sys::git_buf,
                );
                assert_eq!(status, 0, "GIT_OPT_GET_HOMEDIR");
                let bytes = std::slice::from_raw_parts(buffer.ptr as *const u8, buffer.size).to_vec();
                libgit2_sys::git_buf_dispose(&mut buffer);
                bytes
            };
            PathBuf::from(OsString::from_vec(bytes))
        }
    }
}

// The shells the printed commands of design section 10 are pasted into (decision D11): `sh` on Unix, and cmd.exe and
// PowerShell on Windows. The rows that run a command run it as given in each of the platform's shells.
use std::process::{Command, Output};

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// A shell that a printed command is pasted into.
        #[derive(Clone, Copy, Debug)]
        pub(super) enum Shell {
            Sh,
        }

        /// The shells of this platform.
        pub(super) const SHELLS: &[Shell] = &[Shell::Sh];

        impl Shell {
            /// Runs `command` as given in `cwd`, with `HOME` at `home` and git's system file off, as a user would
            /// paste it.
            pub(super) fn run(self, command: &str, cwd: &Path, home: &Path) -> Output {
                let Self::Sh = self;
                Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(cwd)
                    .env("HOME", home)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .output()
                    .unwrap()
            }
        }
    } else {
        /// A shell that a printed command is pasted into.
        #[derive(Clone, Copy, Debug)]
        pub(super) enum Shell {
            Cmd,
            PowerShell,
        }

        /// The shells of this platform.
        pub(super) const SHELLS: &[Shell] = &[Shell::Cmd, Shell::PowerShell];

        impl Shell {
            /// Runs `command` as given in `cwd`, with `HOME` and `USERPROFILE` at `home` and git's system file off, as
            /// a user would paste it: through `cmd /S /C` (the command's own quotes are the shell's, passed raw) or
            /// through `powershell -EncodedCommand`.
            pub(super) fn run(self, command: &str, cwd: &Path, home: &Path) -> Output {
                use std::os::windows::process::CommandExt;
                let mut shell = match self {
                    Self::Cmd => {
                        let mut shell = Command::new("cmd");
                        shell.args(["/D", "/S", "/C"]).raw_arg(format!("\"{command}\""));
                        shell
                    }
                    Self::PowerShell => {
                        // The command is PowerShell text, handed over encoded so that no layer of quoting reads it.
                        let units: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
                        let mut shell = Command::new("powershell");
                        shell.args(["-NoProfile", "-NonInteractive", "-EncodedCommand"]);
                        shell.arg(base64(&units));
                        shell
                    }
                };
                shell
                    .current_dir(cwd)
                    .env("HOME", home)
                    .env("USERPROFILE", home)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .output()
                    .unwrap()
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// `bytes` in standard base64, which PowerShell's `-EncodedCommand` takes (UTF-16LE text).
        fn base64(bytes: &[u8]) -> String {
            const ALPHABET: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut text = String::new();
            for chunk in bytes.chunks(3) {
                let word = chunk.iter().enumerate().fold(0u32, |word, (index, byte)| word | u32::from(*byte) << (16 - 8 * index));
                for index in 0..4 {
                    if index <= chunk.len() {
                        text.push(char::from(ALPHABET[(word >> (18 - 6 * index) & 63) as usize]));
                    } else {
                        text.push('=');
                    }
                }
            }
            text
        }
    }
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

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;

        /// A file that this process cannot open for reading while the value lives (git and libgit2 skip such a
        /// file): mode 000 on Unix, restored on drop.
        pub(super) struct Unreadable(PathBuf);

        impl Unreadable {
            /// `None` when the process reads the file anyway, as root does.
            pub(super) fn new(path: &Path) -> Option<Self> {
                fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
                let guard = Self(path.to_path_buf());
                fs::File::open(path).is_err().then_some(guard)
            }
        }

        impl Drop for Unreadable {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o600));
            }
        }
    } else {
        use std::os::windows::fs::OpenOptionsExt;

        /// A file that this process cannot open for reading while the value lives: Windows holds it open with no
        /// sharing, so another open meets a sharing violation. The file's attributes can still be read, as a
        /// file with no read permission shows them on Unix.
        pub(super) struct Unreadable {
            _held: fs::File,
        }

        impl Unreadable {
            pub(super) fn new(path: &Path) -> Option<Self> {
                let held = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(path)
                    .unwrap();
                Some(Self { _held: held })
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Makes `link` a symbolic link to the file `real`. Always true on Unix.
        pub(super) fn link(real: &Path, link: &Path) -> bool {
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(real, link).unwrap();
            true
        }
    } else {
        /// Makes `link` a symbolic link to the file `real`, or says it may not: creating one takes a privilege on
        /// Windows that an ordinary account lacks unless Developer Mode is on.
        pub(super) fn link(real: &Path, link: &Path) -> bool {
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::windows::fs::symlink_file(real, link).is_ok()
        }
    }
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
