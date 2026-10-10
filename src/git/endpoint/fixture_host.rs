//! The parts of the SSH test servers that differ by host (GwzTransportWindowsParityPlan.md, step 1.1).
//!
//! `SshdFixture` runs a real OpenSSH server on a loopback port. On Unix that is `/usr/sbin/sshd`. On Windows it is
//! the OpenSSH server that ships with the system, `sshd.exe`, started as a plain child in the foreground (`-D`):
//! no service, no system-wide configuration, and a configuration, host key and `authorized_keys` of its own in
//! the fixture's temporary directory.
//!
//! **Finding the programs.** Unix uses `/usr/sbin/sshd` and the `ssh-keygen` on `PATH`. Windows uses, in order,
//! the file `GWZ_TEST_SSHD` names, `%SystemRoot%\System32\OpenSSH\sshd.exe`, and an `sshd.exe` on `PATH`; its
//! `ssh-keygen.exe` is the one beside it, so that keys carry the access rules `sshd.exe` insists on. As on Unix,
//! a missing server fails the test; it is never skipped.
//!
//! **Keys.** Unix fixtures use ed25519 keys. libssh2 built with Windows CNG has no ed25519, so Windows fixtures
//! use 2048-bit RSA keys in PEM form; the files keep their `_ed25519` names, which many tests spell.
//!
//! **Commands.** Windows' `sshd.exe` runs an exec request through the host's default shell, which is `cmd.exe`
//! unless `HKLM\SOFTWARE\OpenSSH\DefaultShell` names another. Under a POSIX default shell (a Git for Windows
//! `bash.exe`) the client's command runs as it would on Unix, and the fixture adds nothing. `cmd.exe` neither knows
//! POSIX quoting nor finds `git-upload-pack`, and a forced command cannot fix that: `sshd.exe` gives every channel
//! of a connection after the first the first channel's `SSH_ORIGINAL_COMMAND` (found on dabeest, step 1.6), so a
//! connection that carries two commands would run the first twice. The fixture therefore forces nothing under
//! `cmd.exe`. It puts a directory of **shims** first on the server's `PATH`, which the sessions inherit: a batch
//! file for each Git service (`git-upload-pack.cmd`) that `cmd.exe` finds by the very name the client sent. The
//! shim keeps its arguments as the client quoted them, puts Git on `PATH`, changes into the fixture's directory
//! (the shell-injection marker is relative to it) and has a POSIX `sh` (`GWZ_TEST_SH`, or the `sh.exe` on `PATH`,
//! as Git for Windows provides) rebuild the client's command and `eval` it, so the client's quoting means what it
//! means on Unix. Each command is its own channel's, so a reused connection runs the right one. A default shell
//! of another kind fails the test with that said.
//!
//! **Testing the `cmd.exe` path on a host that has another shell.** The default shell is a registry value, a host
//! setting a test must not change, and `sshd_config` has no directive for it. `GWZ_TEST_SSH_SHELL=cmd` makes the
//! fixture behave as it does under `cmd.exe`: it writes the shims, and forces a session command (the native
//! helper's `cmd-session` mode, `fixture_helper`) that runs the client's command as `sshd.exe` runs it there,
//! `cmd.exe /c` and the raw command line, with the shims first on the server's `PATH`. That tests everything but
//! `sshd.exe`'s own choice of `cmd.exe`, and its reuse of a connection (the simulation forces a command, which
//! `sshd.exe` repeats for every channel of a connection).
use cfg_if::cfg_if;
use std::path::{Path, PathBuf};

/// The server and key generator a fixture runs.
pub(crate) struct Programs {
    pub(crate) sshd: PathBuf,
    pub(crate) keygen: PathBuf,
}

/// `path` as an `sshd_config` value: Windows spells paths with forward slashes there, since the configuration
/// reads a backslash as an escape.
pub(crate) fn forward_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

/// `path` as one `sshd_config` value: forward slashes, and quoted when it holds a space.
pub(crate) fn config_value(path: &str) -> String {
    let path = forward_slashes(path);
    if path.contains(char::is_whitespace) {
        format!("\"{path}\"")
    } else {
        path
    }
}

/// How a Windows `sshd.exe` runs the command of an exec request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionShell {
    /// `cmd.exe`, the default.
    Cmd,
    /// A POSIX shell, `bash.exe` or `sh.exe`, named as the default shell.
    Posix,
}

/// The shell `reg query HKLM\SOFTWARE\OpenSSH /v DefaultShell` output names: `Cmd` when it names no value or
/// `cmd.exe`, `Posix` for a `sh`, `bash`, `dash` or `zsh`, and `None` for any other.
pub(crate) fn session_shell(registry_output: &str) -> Option<SessionShell> {
    let value = registry_output
        .lines()
        .find(|line| line.trim_start().starts_with("DefaultShell") && line.contains("REG_"))
        .and_then(|line| line.split("REG_").nth(1))
        .map(|rest| {
            rest.trim_start_matches(|c: char| c.is_ascii_uppercase() || c == '_')
                .trim()
        });
    let Some(value) = value else {
        return Some(SessionShell::Cmd);
    };
    let name = value
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim_end_matches('"')
        .to_ascii_lowercase();
    match name.trim_end_matches(".exe") {
        "cmd" => Some(SessionShell::Cmd),
        "sh" | "bash" | "dash" | "zsh" => Some(SessionShell::Posix),
        _ => None,
    }
}

/// `path`, a Windows path, as a POSIX shell on Windows spells it: `C:\a b` as `/c/a b`.
pub(crate) fn posix_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    match path.as_bytes() {
        [letter, b':', rest @ ..] if letter.is_ascii_alphabetic() => {
            format!(
                "/{}{}",
                (*letter as char).to_ascii_lowercase(),
                String::from_utf8_lossy(rest)
            )
        }
        _ => path,
    }
}

/// The `ForceCommand` line for a POSIX default shell: change into `directory`, put the Git directories on `PATH`,
/// and evaluate the client's command.
pub(crate) fn posix_force_command(directory: &str, git_directories: &[String]) -> String {
    let path = git_directories
        .iter()
        .map(|directory| posix_path(directory))
        .collect::<Vec<_>>()
        .join(":");
    assert!(
        !directory.contains('\'') && !path.contains('\''),
        "a fixture path with a single quote cannot be quoted for the shell"
    );
    format!(
        "ForceCommand cd '{}' && export PATH='{path}':\"$PATH\" && {SESSION_COMMAND}\n",
        posix_path(directory)
    )
}

/// The shim for `service`, a batch file `cmd.exe` runs in place of the command the client sent: Git on `PATH`, the
/// client's arguments as they came (`%*`), the fixture's directory as the working directory, and the POSIX shell
/// running [`SHIM_SCRIPT`].
pub(crate) fn shim_batch(
    service: &str,
    shell: &str,
    script: &str,
    directory: &str,
    git_directories: &[String],
) -> String {
    let path = git_directories
        .iter()
        .map(|directory| directory.replace('/', "\\"))
        .chain(["%PATH%".to_owned()])
        .collect::<Vec<_>>()
        .join(";");
    format!(
        "@echo off\r\nsetlocal\r\nset \"PATH={path}\"\r\nset \"GWZ_SERVICE={service}\"\r\nset \"GWZ_ARGS=%*\"\r\ncd /d \"{directory}\"\r\n\"{shell}\" \"{script}\"\r\nexit /b %errorlevel%\r\n",
        shell = shell.replace('/', "\\"),
        script = script.replace('/', "\\"),
        directory = directory.replace('/', "\\"),
    )
}

/// What the POSIX shell runs for a shim: the client's command rebuilt from the shim's name and arguments, then the
/// fixture's forced script when it has one, else the command itself.
pub(crate) const SHIM_SCRIPT: &str = "SSH_ORIGINAL_COMMAND=\"$GWZ_SERVICE $GWZ_ARGS\"\nexport SSH_ORIGINAL_COMMAND\nif [ -f ./close-script.sh ]; then . ./close-script.sh; else eval \"$SSH_ORIGINAL_COMMAND\"; fi\n";

/// `existing`, a `PATH`, with `shims` first.
pub(crate) fn path_with_shims(shims: &str, existing: &str) -> String {
    if existing.is_empty() {
        shims.to_owned()
    } else {
        format!("{shims};{existing}")
    }
}

/// The script a fixture may leave in its directory to run in place of the client's command, as a forced command
/// does on Unix (the close fixtures, `ssh_close_fixture`). It is sourced, so its `exit` is the session's.
pub(crate) const FORCED_SCRIPT: &str = "close-script.sh";

/// What the POSIX shell evaluates for a session: the fixture's forced script when it has one, else the command the
/// client sent.
const SESSION_COMMAND: &str = "if [ -f ./close-script.sh ]; then . ./close-script.sh; else eval \"$SSH_ORIGINAL_COMMAND\"; fi";

/// A `known_hosts` entry for `host` from a `.pub` file's text: the key type and key only, on one line. The
/// comment is dropped and the line ending is `\n`, whatever the key generator wrote. Windows' `ssh-keygen` ends
/// its lines with `\r\n`.
pub(crate) fn normalized_known_hosts_line(host: &str, public_key: &str) -> String {
    let mut fields = public_key.split_whitespace();
    let (kind, key) = (fields.next().unwrap_or(""), fields.next().unwrap_or(""));
    format!("{host} {kind} {key}\n")
}

/// The name of the bare repository the fixture serves, which carries a shell-injection marker: a server that
/// evaluates the repository path unquoted would run `touch` and create the marker.
pub(crate) fn repository_name(marker: &Path) -> String {
    format!("repo'$({})'", injected_command(marker))
}

cfg_if! {
    if #[cfg(unix)] {
        use std::process::Command;

        /// `path` as the path of an `ssh://` URL or a Git command names it on this server: as it is on Unix.
        pub(crate) fn server_path(path: &Path) -> String {
            path.display().to_string()
        }

        /// The arguments that make `ssh-keygen` write an RSA key the fixture's libssh2 can use: the default form.
        pub(crate) fn rsa_key_arguments() -> &'static [&'static str] {
            &["-t", "rsa", "-b", "2048"]
        }

        /// The variables a child that otherwise has a clean environment still needs: none on Unix.
        pub(crate) fn system_environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
            Vec::new()
        }

        /// The path through which a URL reaches `repository`: itself on Unix.
        pub(crate) fn url_repository(_temp: &Path, repository: &Path) -> PathBuf {
            repository.to_owned()
        }

        /// A `known_hosts` entry for `host` from a `.pub` file's text, as the Unix `ssh-keygen` wrote it: tests that\n        /// pad the line to libssh2's length limits count its comment.
        pub(crate) fn known_hosts_line(host: &str, public_key: &str) -> String {
            format!("{host} {public_key}")
        }

        /// The command the repository name injects: `touch` on the marker, by its whole path.
        fn injected_command(marker: &Path) -> String {
            format!("touch {}", marker.to_str().unwrap())
        }

        /// Finds the server and key generator.
        pub(crate) fn programs() -> Programs {
            let sshd = PathBuf::from("/usr/sbin/sshd");
            assert!(
                sshd.exists(),
                "native gate requires /usr/sbin/sshd; this is not a skipped qualification"
            );
            Programs {
                sshd,
                keygen: PathBuf::from("ssh-keygen"),
            }
        }

        /// The arguments that make `programs.keygen` write a key the fixture's libssh2 can use.
        pub(crate) fn fixture_key_arguments() -> &'static [&'static str] {
            &["-t", "ed25519"]
        }

        /// The account the server logs in.
        pub(crate) fn login_name() -> String {
            let output = Command::new("id").arg("-un").output().unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        /// The configuration lines that are not portable: none on Unix.
        pub(crate) fn session_directives(_temp: &Path) -> String {
            String::new()
        }

        /// The same for a server whose sessions run a forced script: on Unix that is `authorized_keys`' business.
        pub(crate) fn forced_session_directives(_temp: &Path) -> String {
            String::new()
        }

        /// A symbolic link at `link` to `target`.
        pub(crate) fn make_symlink(target: &Path, link: &Path) {
            std::os::unix::fs::symlink(target, link).unwrap();
        }

        /// The name of the fixture's Job Object: unused on Unix.
        pub(crate) fn job_name(_temp: &Path) -> String {
            String::new()
        }

        /// The server's environment: Unix adds nothing.
        pub(crate) fn server_environment(_temp: &Path, _server: &mut Command) {}

        /// The configuration lines every fixture server has, then the platform's, then `startups`.
        pub(crate) fn server_config(
            extra: &str,
            port: u16,
            host_key: &Path,
            authorized: &Path,
            session: &str,
            startups: &str,
        ) -> String {
            format!(
                "{extra}Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nChallengeResponseAuthentication no\nUsePAM no\nPermitRootLogin yes\nPubkeyAuthentication yes\nStrictModes no\nLogLevel ERROR\n{session}{startups}",
                host_key.display(),
                authorized.display(),
            )
        }
    } else if #[cfg(windows)] {
        use std::{env, fs, os::windows::fs::OpenOptionsExt, process::Command};

        /// `path` as the path of an `ssh://` URL or a Git command names it on this server: the POSIX form a Windows
        /// server's shell and Git take (`E:\\a\\b` as `/e/a/b`), since a drive letter has no place in a URL path.
        pub(crate) fn server_path(path: &Path) -> String {
            posix_path(&path.display().to_string())
        }

        /// The arguments that make `ssh-keygen` write an RSA key the fixture's libssh2 can use: in PEM form, the only
        /// form libssh2 on Windows CNG reads.
        pub(crate) fn rsa_key_arguments() -> &'static [&'static str] {
            &["-t", "rsa", "-b", "2048", "-m", "PEM"]
        }

        /// The variables a child that otherwise has a clean environment still needs on Windows, to run at all: `Path`
        /// (the test executable loads the Python runtime from it), the system directory, the account (the fixture's server logs in as it), the temporary directories, the
        /// command interpreter and the program directories Git looks in; and the test hosts' overrides.
        pub(crate) fn system_environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
            const NAMES: &[&str] = &[
                "Path",
                "SystemRoot",
                "SystemDrive",
                "windir",
                "USERNAME",
                "USERDOMAIN",
                "TEMP",
                "TMP",
                "ComSpec",
                "PATHEXT",
                "ProgramData",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
                "LOCALAPPDATA",
                "APPDATA",
                "USERPROFILE",
                SSHD_ENV,
                SH_ENV,
                SHELL_ENV,
            ];
            NAMES
                .iter()
                .filter_map(|name| env::var_os(name).map(|value| ((*name).into(), value)))
                .collect()
        }

        /// The path through which a URL reaches `repository`: a directory junction to it in `temp`. The
        /// repository's own name carries a single quote (the shell-injection marker), and a POSIX shell on Windows
        /// does not convert a POSIX-form path that holds one into the Windows form Git needs, so a URL, which
        /// cannot hold a drive letter, reaches it by a name without one. (The SSH endpoint's own tests give Git the
        /// Windows form directly.) A junction needs no privilege.
        pub(crate) fn url_repository(temp: &Path, repository: &Path) -> PathBuf {
            let link = temp.join("url-repository");
            make_junction(&link, repository).unwrap_or_else(|error| {
                panic!("creating the junction {} failed: {error}", link.display())
            });
            link
        }

        /// A directory junction at `link` to the directory `target`: an empty directory whose reparse data is a
        /// mount point (`IO_REPARSE_TAG_MOUNT_POINT`) naming `target` in the NT namespace.
        fn make_junction(link: &Path, target: &Path) -> std::io::Result<()> {
            use std::{
                os::windows::{ffi::OsStrExt, io::AsRawHandle},
                ptr,
            };
            use windows_sys::Win32::{
                Foundation::HANDLE,
                Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
                System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_REPARSE_POINT},
            };
            const MOUNT_POINT: u32 = 0xA000_0003;
            // The substitute name is the NT path `\??\E:\...` and the print name is empty.
            let target = std::fs::canonicalize(target)?;
            let canonical: Vec<u16> = target.as_os_str().encode_wide().collect();
            // `canonicalize` gives `\\?\E:\...`; the NT form of that prefix is `\??\`.
            let substitute: Vec<u16> = [r"\??\".encode_utf16().collect::<Vec<_>>(), canonical[4..].to_vec()].concat();
            std::fs::create_dir(link)?;
            let directory = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(link)?;
            let mut data = Vec::new();
            let name_bytes = (substitute.len() * 2) as u16;
            // REPARSE_DATA_BUFFER header: tag, data length, reserved; then the mount point fields.
            data.extend_from_slice(&MOUNT_POINT.to_le_bytes());
            data.extend_from_slice(&(8 + name_bytes + 2 + 2).to_le_bytes());
            data.extend_from_slice(&0_u16.to_le_bytes());
            data.extend_from_slice(&0_u16.to_le_bytes()); // substitute name offset
            data.extend_from_slice(&name_bytes.to_le_bytes());
            data.extend_from_slice(&(name_bytes + 2).to_le_bytes()); // print name offset
            data.extend_from_slice(&0_u16.to_le_bytes()); // print name length
            for unit in &substitute {
                data.extend_from_slice(&unit.to_le_bytes());
            }
            data.extend_from_slice(&[0, 0, 0, 0]); // the substitute name's and the print name's terminators
            let mut returned = 0_u32;
            let done = unsafe {
                DeviceIoControl(
                    directory.as_raw_handle() as HANDLE,
                    FSCTL_SET_REPARSE_POINT,
                    data.as_ptr().cast(),
                    data.len() as u32,
                    ptr::null_mut(),
                    0,
                    &mut returned,
                    ptr::null_mut(),
                )
            };
            if done == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        /// The Git services whose commands a `cmd.exe` session finds shims for.
        pub(crate) const SHIMMED_SERVICES: [&str; 3] = ["git-upload-pack", "git-receive-pack", "git-upload-archive"];
        /// The directory of a fixture that holds its shims.
        pub(crate) const SHIMS_DIRECTORY: &str = "shims";
        /// Names the `sshd.exe` a Windows fixture runs, when set.
        pub(crate) const SSHD_ENV: &str = "GWZ_TEST_SSHD";
        /// Names the POSIX `sh.exe` a Windows fixture's commands run under, when set.
        pub(crate) const SH_ENV: &str = "GWZ_TEST_SH";

        /// A `known_hosts` entry for `host` from a `.pub` file's text, normalized.
        pub(crate) fn known_hosts_line(host: &str, public_key: &str) -> String {
            normalized_known_hosts_line(host, public_key)
        }

        /// The command the repository name injects: `touch` on the marker. Under `cmd.exe` the shim changes
        /// into the fixture's directory and a Windows file name has no `:` or `\`, so the marker is named relative to
        /// it. Under a POSIX default shell the commands run where the server puts them, so the marker is named by its
        /// whole path; the command's own space is `${IFS}` there, since a space ending a path segment is not a Windows
        /// directory name.
        fn injected_command(marker: &Path) -> String {
            match default_shell() {
                SessionShell::Posix => format!("touch${{IFS}}{}", posix_path(&marker.display().to_string())),
                SessionShell::Cmd => format!("touch {}", marker.file_name().unwrap().to_string_lossy()),
            }
        }

        /// The first file named `name` in a directory of `PATH`.
        pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
            let path = env::var_os("PATH")?;
            env::split_paths(&path)
                .map(|directory| directory.join(name))
                .find(|candidate| candidate.is_file())
        }

        /// Finds the server and key generator.
        pub(crate) fn programs() -> Programs {
            let system = env::var_os("SystemRoot").map(|root| {
                PathBuf::from(root).join("System32").join("OpenSSH").join("sshd.exe")
            });
            let sshd = match env::var_os(SSHD_ENV) {
                Some(named) => {
                    let named = PathBuf::from(named);
                    assert!(named.is_file(), "{SSHD_ENV} names {}, which is not a file", named.display());
                    named
                }
                None => system
                    .filter(|candidate| candidate.is_file())
                    .or_else(|| on_path("sshd.exe"))
                    .unwrap_or_else(|| {
                        panic!(
                            "native gate requires Windows' OpenSSH server (System32\\OpenSSH\\sshd.exe, an sshd.exe on PATH, or {SSHD_ENV}); this is not a skipped qualification"
                        )
                    }),
            };
            let beside = sshd.with_file_name("ssh-keygen.exe");
            let keygen = if beside.is_file() {
                beside
            } else {
                on_path("ssh-keygen.exe").expect("native gate requires ssh-keygen.exe beside sshd.exe or on PATH")
            };
            Programs { sshd, keygen }
        }

        /// The arguments that make `programs.keygen` write a key the fixture's libssh2 can use: RSA in PEM form,
        /// since libssh2 on Windows CNG has no ed25519.
        pub(crate) fn fixture_key_arguments() -> &'static [&'static str] {
            &["-t", "rsa", "-b", "2048", "-m", "PEM"]
        }

        /// The account the server logs in.
        pub(crate) fn login_name() -> String {
            env::var("USERNAME").expect("USERNAME names the account the server logs in")
        }

        /// Names the shell a fixture server's sessions are to behave as, whatever the host's default shell is.
        /// `cmd` runs every command under `cmd.exe /c` and the shims, which is what `sshd.exe` does on a host
        /// whose default shell is `cmd.exe` (a hosted `windows-2022` runner), and so lets a host with another
        /// default shell exercise that path.
        pub(crate) const SHELL_ENV: &str = "GWZ_TEST_SSH_SHELL";

        /// The host's OpenSSH default shell, from the registry.
        fn registry_shell() -> SessionShell {
            let registry = Command::new("reg")
                .args(["query", "HKLM\\SOFTWARE\\OpenSSH", "/v", "DefaultShell"])
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default();
            session_shell(&registry).unwrap_or_else(|| {
                panic!(
                    "the host's OpenSSH default shell is neither cmd.exe nor a POSIX shell, and the fixture runs commands under only those: {registry}"
                )
            })
        }

        /// The shell the sessions behave as: [`SHELL_ENV`]'s, else the host's.
        fn default_shell() -> SessionShell {
            match env::var(SHELL_ENV).as_deref() {
                Ok("cmd") => SessionShell::Cmd,
                Ok(other) => panic!("{SHELL_ENV} is {other:?}; the only simulated shell is \"cmd\""),
                Err(_) => registry_shell(),
            }
        }

        /// The configuration lines that make the host's default shell run an exec request's command the way a Unix
        /// `sshd` would: none. Under a POSIX default shell the shell takes the command as it is. Under `cmd.exe` the
        /// fixture writes shims for the Git services into `temp` ([`server_environment`] puts them on the server's
        /// `PATH`), because a forced command would be wrong: see the module's note on `SSH_ORIGINAL_COMMAND`.
        pub(crate) fn session_directives(temp: &Path) -> String {
            if default_shell() != SessionShell::Cmd {
                return String::new();
            }
            write_cmd_shims(temp, &git_directories());
            if registry_shell() == SessionShell::Cmd {
                return String::new();
            }
            // The host's shell is another, so the sessions are made to start `cmd.exe` as `sshd.exe` would.
            format!("ForceCommand {}\n", super::fixture_helper::script_line(super::fixture_helper::Mode::CmdSession))
        }

        /// The same for a server whose sessions run [`FORCED_SCRIPT`] when the fixture leaves one in its directory
        /// (the close fixtures). Under a POSIX default shell that is a forced command, which is safe there because
        /// a close fixture's connection carries one command; under `cmd.exe` the shims run the script.
        pub(crate) fn forced_session_directives(temp: &Path) -> String {
            match default_shell() {
                SessionShell::Posix => posix_force_command(&temp.display().to_string(), &git_directories()),
                SessionShell::Cmd => session_directives(temp),
            }
        }

        /// A symbolic link at `link` to `target`, a file or a directory. Creating one needs the symbolic-link
        /// privilege (`SeCreateSymbolicLinkPrivilege`, which an administrator's elevated logon and a logon with
        /// Developer Mode hold, and an ordinary one does not); without it the test fails and says so, since a skipped
        /// row would leave the symlink behaviour unqualified without a trace.
        pub(crate) fn make_symlink(target: &Path, link: &Path) {
            let made = if target.is_dir() {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            };
            match made {
                Ok(()) => {}
                Err(error) if error.raw_os_error() == Some(1314) => {
                    panic!(
                        "creating a symbolic link needs the symbolic-link privilege (ERROR_PRIVILEGE_NOT_HELD); run this test from an elevated logon or with Developer Mode on: {error}"
                    );
                }
                Err(error) => panic!("creating a symbolic link failed: {error}"),
            }
        }

        /// The name of the fixture's Job Object, unique because its directory is.
        pub(crate) fn job_name(temp: &Path) -> String {
            format!("gwz-fixture-{}", temp.file_name().unwrap().to_string_lossy())
        }

        /// The server's environment: the name of the fixture's job, so that a session's helper can list the
        /// processes the fixture owns, and the shims first on `PATH` when the fixture wrote any.
        pub(crate) fn server_environment(temp: &Path, server: &mut Command) {
            server.env(super::fixture_helper::JOB_ENV, job_name(temp));
            let shims = temp.join(SHIMS_DIRECTORY);
            if shims.is_dir() {
                let existing = env::var("PATH").unwrap_or_default();
                let path = path_with_shims(&shims.display().to_string(), &existing);
                // Under a simulated `cmd.exe` the sessions' own shell would reorder `PATH`; this copy is the server's.
                server.env(super::fixture_helper::SERVER_PATH_ENV, &path);
                server.env("PATH", path);
            }
        }

        /// The directories Git runs from: the one that holds `git.exe`, and its exec path.
        fn git_directories() -> Vec<String> {
            let mut directories = Vec::new();
            if let Some(git) = on_path("git.exe") {
                directories.push(git.parent().unwrap().display().to_string());
            }
            let exec_path = Command::new("git").arg("--exec-path").output().unwrap();
            assert!(exec_path.status.success(), "git --exec-path failed");
            directories.push(String::from_utf8(exec_path.stdout).unwrap().trim().to_owned());
            directories
        }

        /// Writes the shims and the POSIX script they run into `temp`/[`SHIMS_DIRECTORY`]; returns that directory.
        pub(crate) fn write_cmd_shims(temp: &Path, git_directories: &[String]) -> PathBuf {
            let shell = env::var_os(SH_ENV)
                .map(PathBuf::from)
                .or_else(|| on_path("sh.exe"))
                .unwrap_or_else(|| {
                    panic!("native gate requires a POSIX sh.exe (on PATH, or named by {SH_ENV}), as Git for Windows provides")
                });
            let shims = temp.join(SHIMS_DIRECTORY);
            fs::create_dir_all(&shims).unwrap();
            let script = shims.join("shim.sh");
            fs::write(&script, SHIM_SCRIPT).unwrap();
            for service in SHIMMED_SERVICES {
                fs::write(
                    shims.join(format!("{service}.cmd")),
                    shim_batch(
                        service,
                        &shell.display().to_string(),
                        &script.display().to_string(),
                        &temp.display().to_string(),
                        git_directories,
                    ),
                )
                .unwrap();
            }
            shims
        }

        /// The configuration lines every fixture server has, then the platform's, then `startups`. The Windows
        /// server has no PAM and no root account to configure.
        pub(crate) fn server_config(
            extra: &str,
            port: u16,
            host_key: &Path,
            authorized: &Path,
            session: &str,
            startups: &str,
        ) -> String {
            format!(
                "{extra}Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\nStrictModes no\nLogLevel ERROR\n{session}{startups}",
                config_value(&host_key.display().to_string()),
                config_value(&authorized.display().to_string()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_hosts_line_is_the_host_the_key_type_and_the_key_alone() {
        let public = "ssh-rsa AAAAB3Nza user@machine\r\n";
        assert_eq!(
            normalized_known_hosts_line("[127.0.0.1]:2222", public),
            "[127.0.0.1]:2222 ssh-rsa AAAAB3Nza\n"
        );
        assert_eq!(
            normalized_known_hosts_line("[127.0.0.1]:2222", "ssh-ed25519 AAAAC3Nz\n"),
            "[127.0.0.1]:2222 ssh-ed25519 AAAAC3Nz\n"
        );
    }

    #[test]
    fn paths_in_the_server_configuration_use_forward_slashes() {
        assert_eq!(
            forward_slashes("E:\\gwz-tests\\run 1\\host_key"),
            "E:/gwz-tests/run 1/host_key"
        );
        assert_eq!(forward_slashes("/tmp/a b"), "/tmp/a b");
    }

    #[test]
    fn a_configuration_value_with_a_space_is_quoted() {
        assert_eq!(config_value("E:\\a b\\key"), "\"E:/a b/key\"");
        assert_eq!(config_value("E:\\ab\\key"), "E:/ab/key");
    }

    #[test]
    fn the_default_shell_is_read_from_the_registry_output() {
        let bash = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\OpenSSH\r\n    DefaultShell    REG_SZ    C:\\Program Files\\Git\\bin\\bash.exe\r\n\r\n";
        assert_eq!(session_shell(bash), Some(SessionShell::Posix));
        let cmd = "    DefaultShell    REG_SZ    C:\\Windows\\System32\\cmd.exe\r\n";
        assert_eq!(session_shell(cmd), Some(SessionShell::Cmd));
        assert_eq!(session_shell(""), Some(SessionShell::Cmd));
        assert_eq!(
            session_shell(
                "ERROR: The system was unable to find the specified registry key or value."
            ),
            Some(SessionShell::Cmd)
        );
        let powershell = "    DefaultShell    REG_SZ    C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\r\n";
        assert_eq!(session_shell(powershell), None);
        let sh = "    DefaultShell    REG_EXPAND_SZ    %ProgramFiles%\\Git\\usr\\bin\\sh.exe\r\n";
        assert_eq!(session_shell(sh), Some(SessionShell::Posix));
    }

    #[test]
    fn a_windows_path_is_spelled_for_a_posix_shell_on_windows() {
        assert_eq!(
            posix_path("C:\\Program Files\\Git\\cmd"),
            "/c/Program Files/Git/cmd"
        );
        assert_eq!(posix_path("e:/work/t"), "/e/work/t");
        assert_eq!(posix_path("/already/posix"), "/already/posix");
    }

    #[test]
    fn the_posix_forced_command_changes_directory_extends_the_path_and_evaluates_the_client_command()
     {
        assert_eq!(
            posix_force_command(
                "C:\\Users\\a b\\Temp\\.tmpX",
                &[
                    "C:\\Program Files\\Git\\cmd".into(),
                    "C:/Program Files/Git/mingw64/libexec/git-core".into()
                ]
            ),
            "ForceCommand cd '/c/Users/a b/Temp/.tmpX' && export PATH='/c/Program Files/Git/cmd:/c/Program Files/Git/mingw64/libexec/git-core':\"$PATH\" && if [ -f ./close-script.sh ]; then . ./close-script.sh; else eval \"$SSH_ORIGINAL_COMMAND\"; fi\n"
        );
    }

    #[test]
    fn a_shim_runs_the_service_in_the_fixture_directory_with_git_on_the_path() {
        let batch = shim_batch(
            "git-upload-pack",
            "C:/Program Files/Git/usr/bin/sh.exe",
            "E:/t/shim.sh",
            "E:/t",
            &[
                "C:/Program Files/Git/cmd".into(),
                "C:/Program Files/Git/mingw64/libexec/git-core".into(),
            ],
        );
        assert_eq!(
            batch,
            "@echo off\r\nsetlocal\r\nset \"PATH=C:\\Program Files\\Git\\cmd;C:\\Program Files\\Git\\mingw64\\libexec\\git-core;%PATH%\"\r\nset \"GWZ_SERVICE=git-upload-pack\"\r\nset \"GWZ_ARGS=%*\"\r\ncd /d \"E:\\t\"\r\n\"C:\\Program Files\\Git\\usr\\bin\\sh.exe\" \"E:\\t\\shim.sh\"\r\nexit /b %errorlevel%\r\n"
        );
    }

    #[test]
    fn the_shim_script_rebuilds_the_clients_command_and_runs_the_forced_script_or_evaluates_it() {
        assert!(SHIM_SCRIPT.starts_with(
            "SSH_ORIGINAL_COMMAND=\"$GWZ_SERVICE $GWZ_ARGS\"\nexport SSH_ORIGINAL_COMMAND\n"
        ));
        assert!(SHIM_SCRIPT.contains("eval \"$SSH_ORIGINAL_COMMAND\""));
        assert!(SHIM_SCRIPT.contains(FORCED_SCRIPT) && SESSION_COMMAND.contains(FORCED_SCRIPT));
    }

    #[test]
    fn the_shims_directory_comes_first_on_the_servers_path() {
        assert_eq!(
            path_with_shims("E:\\t\\shims", "C:\\Windows;C:\\Git"),
            "E:\\t\\shims;C:\\Windows;C:\\Git"
        );
        assert_eq!(path_with_shims("E:\\t\\shims", ""), "E:\\t\\shims");
    }

    #[test]
    fn the_repository_name_carries_the_injection_marker() {
        let name = repository_name(&Path::new("/work/temp").join("injection-marker"));
        // The space is `${IFS}` where the marker is named by a whole path and a space would end a directory name.
        assert!(
            name.starts_with("repo'$(touch ") || name.starts_with("repo'$(touch${IFS}"),
            "{name}"
        );
        assert!(name.ends_with("injection-marker)'"), "{name}");
    }

    #[test]
    fn the_server_configuration_names_the_port_and_the_keys() {
        let text = server_config(
            "LogLevel VERBOSE\n",
            2222,
            Path::new("/t/host_key"),
            Path::new("/t/authorized_keys"),
            "",
            "MaxStartups 64\n",
        );
        assert!(text.starts_with("LogLevel VERBOSE\nPort 2222\nListenAddress 127.0.0.1\n"));
        assert!(text.contains("HostKey /t/host_key\n"));
        assert!(text.contains("AuthorizedKeysFile /t/authorized_keys\n"));
        assert!(text.ends_with("MaxStartups 64\n"));
        assert!(text.contains("StrictModes no\n"));
    }

    cfg_if! {
        if #[cfg(windows)] {
            use std::os::windows::process::CommandExt;

            /// A shim, run as `cmd.exe` runs it when it is `sshd.exe`'s default shell: the command line the client sent,
            /// POSIX quoting and all, with the shims first on `PATH`. A host whose default shell is another still
            /// runs this, since it needs no server.
            #[test]
            fn a_shim_runs_a_posix_quoted_git_command_in_the_fixture_directory_under_cmd() {
                let temp = tempfile::TempDir::new().unwrap();
                let marker = temp.path().join("injection-marker");
                let repository = temp.path().join(repository_name(&marker));
                let init = Command::new("git").args(["init", "-q", "--bare", "--"]).arg(&repository).status().unwrap();
                assert!(init.success());
                let shims = write_cmd_shims(temp.path(), &git_directories());
                let quoted = repository.to_str().unwrap().replace('\'', "'\\''");
                let mut server = Command::new("cmd");
                server_environment(temp.path(), &mut server);
                let output = server
                    .arg("/c")
                    .raw_arg(format!("git-upload-pack '{quoted}'"))
                    .stdin(std::process::Stdio::null())
                    .output()
                    .unwrap();
                // The advertisement is complete; Git then reports that the client, here no client, hung up.
                assert!(output.stdout.windows(4).any(|window| window == b"0000"), "{output:?} {shims:?}");
                assert!(!marker.exists(), "the repository path was shell-injected");
            }

            /// Two commands, one after the other, each the shim's own: the second is not the first again.
            #[test]
            fn each_shim_runs_its_own_service() {
                let temp = tempfile::TempDir::new().unwrap();
                let repository = temp.path().join("repo");
                let init = Command::new("git").args(["init", "-q", "--bare", "--"]).arg(&repository).status().unwrap();
                assert!(init.success());
                write_cmd_shims(temp.path(), &git_directories());
                let quoted = repository.to_str().unwrap().to_owned();
                let advertisement = |service: &str| {
                    let mut server = Command::new("cmd");
                    server_environment(temp.path(), &mut server);
                    let output = server
                        .arg("/c")
                        .raw_arg(format!("{service} '{quoted}'"))
                        .stdin(std::process::Stdio::null())
                        .output()
                        .unwrap();
                    String::from_utf8_lossy(&output.stdout).into_owned()
                };
                let upload = advertisement("git-upload-pack");
                let receive = advertisement("git-receive-pack");
                assert!(upload.contains("multi_ack"), "{upload}");
                assert!(receive.contains("report-status"), "{receive}");
            }
        }
    }
}
