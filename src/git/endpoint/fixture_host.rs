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
//! unless `HKLM\SOFTWARE\OpenSSH\DefaultShell` names another. `cmd.exe` neither knows POSIX quoting nor finds
//! `git-upload-pack`, so the fixture's configuration forces a command that fixes both, and which command depends
//! on the shell: under `cmd.exe`, a `.cmd` file in the temporary directory puts Git on `PATH`, changes into that
//! directory and has a POSIX `sh` (`GWZ_TEST_SH`, or the `sh.exe` on `PATH`, as Git for Windows provides)
//! `eval` the original command; under a POSIX default shell (a Git for Windows `bash.exe`, say) the forced command
//! does the same in the shell's own words. Either way the client's quoting means what it means on Unix, and the
//! working directory is the fixture's, which the shell-injection marker is relative to. A default shell of
//! another kind fails the test with that said.
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

/// The `ForceCommand` line that runs `script`, a Windows path, under `cmd.exe`. `sshd_config` takes the rest of the
/// line as the command, unparsed, so the path is only quoted for `cmd`, and only when it holds a space.
pub(crate) fn force_command_directive(script: &str) -> String {
    if script.contains(char::is_whitespace) {
        format!("ForceCommand \"{script}\"\n")
    } else {
        format!("ForceCommand {script}\n")
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

/// The batch file a Windows session runs: Git on `PATH`, the fixture's directory as the working directory (the
/// shell-injection marker is relative to it), and the POSIX shell evaluating the client's command.
pub(crate) fn session_batch(
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
        "@echo off\r\nset \"PATH={path}\"\r\ncd /d \"{directory}\"\r\n\"{shell}\" \"{script}\"\r\nexit /b %errorlevel%\r\n",
        shell = shell.replace('/', "\\"),
        script = script.replace('/', "\\"),
        directory = directory.replace('/', "\\"),
    )
}

/// The script a fixture may leave in its directory to run in place of the client's command, as a forced command
/// does on Unix (the close fixtures, `ssh_close_fixture`). It is sourced, so its `exit` is the session's.
pub(crate) const FORCED_SCRIPT: &str = "close-script.sh";

/// What the POSIX shell evaluates for a session: the fixture's forced script when it has one, else the command the
/// client sent.
const SESSION_COMMAND: &str = "if [ -f ./close-script.sh ]; then . ./close-script.sh; else eval \"$SSH_ORIGINAL_COMMAND\"; fi";

/// What the POSIX shell runs for each session.
pub(crate) const SESSION_SCRIPT: &str = "if [ -f ./close-script.sh ]; then . ./close-script.sh; else eval \"$SSH_ORIGINAL_COMMAND\"; fi\n";

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
        use std::{env, fs, process::Command};

        /// Names the `sshd.exe` a Windows fixture runs, when set.
        pub(crate) const SSHD_ENV: &str = "GWZ_TEST_SSHD";
        /// Names the POSIX `sh.exe` a Windows fixture's commands run under, when set.
        pub(crate) const SH_ENV: &str = "GWZ_TEST_SH";

        /// A `known_hosts` entry for `host` from a `.pub` file's text, normalized.
        pub(crate) fn known_hosts_line(host: &str, public_key: &str) -> String {
            normalized_known_hosts_line(host, public_key)
        }

        /// The command the repository name injects: `touch` on the marker. Under `cmd.exe` the session wrapper changes
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

        /// The host's OpenSSH default shell, from the registry.
        fn default_shell() -> SessionShell {
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

        /// The configuration lines that make the host's default shell run an exec request's command the way a Unix
        /// `sshd` would. Under a POSIX default shell there are none: the shell takes the command as it is. A forced
        /// command would be wrong there, because Windows' `sshd.exe` gives every channel of a connection after the
        /// first the first channel's `SSH_ORIGINAL_COMMAND` (found on dabeest, step 1.6: a push after a fetch on one
        /// connection ran `git-upload-pack`). Under `cmd.exe`, which knows neither POSIX quoting nor
        /// `git-upload-pack`, it forces a command that fixes both, and writes the wrapper it names into `temp`.
        pub(crate) fn session_directives(temp: &Path) -> String {
            match default_shell() {
                SessionShell::Posix => String::new(),
                SessionShell::Cmd => cmd_session_directives(temp),
            }
        }

        /// The same for a server whose sessions run [`FORCED_SCRIPT`] when the fixture leaves one in its directory
        /// (the close fixtures): a forced command under either shell, with the first-command limit above.
        pub(crate) fn forced_session_directives(temp: &Path) -> String {
            match default_shell() {
                SessionShell::Posix => posix_force_command(&temp.display().to_string(), &git_directories()),
                SessionShell::Cmd => cmd_session_directives(temp),
            }
        }

        fn cmd_session_directives(temp: &Path) -> String {
            force_command_directive(&write_cmd_session(temp, &git_directories()).display().to_string())
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

        /// Writes the `cmd.exe` session wrapper and the POSIX script it runs into `temp`; returns the wrapper.
        fn write_cmd_session(temp: &Path, git_directories: &[String]) -> PathBuf {
            let shell = env::var_os(SH_ENV)
                .map(PathBuf::from)
                .or_else(|| on_path("sh.exe"))
                .unwrap_or_else(|| {
                    panic!("native gate requires a POSIX sh.exe (on PATH, or named by {SH_ENV}), as Git for Windows provides")
                });
            let script = temp.join("session.sh");
            fs::write(&script, SESSION_SCRIPT).unwrap();
            let batch = temp.join("session.cmd");
            fs::write(
                &batch,
                session_batch(
                    &shell.display().to_string(),
                    &script.display().to_string(),
                    &temp.display().to_string(),
                    git_directories,
                ),
            )
            .unwrap();
            batch
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
    fn the_forced_command_is_the_rest_of_the_line_quoted_only_for_cmd() {
        assert_eq!(
            force_command_directive("E:\\t\\session.cmd"),
            "ForceCommand E:\\t\\session.cmd\n"
        );
        assert_eq!(
            force_command_directive("E:\\a b\\session.cmd"),
            "ForceCommand \"E:\\a b\\session.cmd\"\n"
        );
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
    fn the_session_batch_puts_git_on_the_path_and_runs_the_shell_in_the_fixture_directory() {
        let batch = session_batch(
            "C:/Program Files/Git/usr/bin/sh.exe",
            "E:/t/session.sh",
            "E:/t",
            &[
                "C:/Program Files/Git/cmd".into(),
                "C:/Program Files/Git/mingw64/libexec/git-core".into(),
            ],
        );
        assert_eq!(
            batch,
            "@echo off\r\nset \"PATH=C:\\Program Files\\Git\\cmd;C:\\Program Files\\Git\\mingw64\\libexec\\git-core;%PATH%\"\r\ncd /d \"E:\\t\"\r\n\"C:\\Program Files\\Git\\usr\\bin\\sh.exe\" \"E:\\t\\session.sh\"\r\nexit /b %errorlevel%\r\n"
        );
    }

    #[test]
    fn the_session_script_evaluates_the_original_command() {
        assert!(SESSION_SCRIPT.contains("eval \"$SSH_ORIGINAL_COMMAND\""));
        assert!(SESSION_SCRIPT.contains(FORCED_SCRIPT) && SESSION_COMMAND.contains(FORCED_SCRIPT));
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
            /// The `cmd.exe` session wrapper, run as `sshd.exe` runs it when the default shell is `cmd.exe`: with the
            /// client's command in `SSH_ORIGINAL_COMMAND`. A host whose default shell is another still runs this, since
            /// it needs no server.
            #[test]
            fn the_cmd_session_wrapper_runs_a_posix_quoted_git_command_in_the_fixture_directory() {
                let temp = tempfile::TempDir::new().unwrap();
                let marker = temp.path().join("injection-marker");
                let repository = temp.path().join(repository_name(&marker));
                let init = Command::new("git").args(["init", "-q", "--bare", "--"]).arg(&repository).status().unwrap();
                assert!(init.success());
                let batch = write_cmd_session(temp.path(), &git_directories());
                let quoted = repository.to_str().unwrap().replace('\'', "'\\''");
                let output = Command::new("cmd")
                    .arg("/c")
                    .arg(&batch)
                    .env("SSH_ORIGINAL_COMMAND", format!("git-upload-pack '{quoted}'"))
                    .stdin(std::process::Stdio::null())
                    .output()
                    .unwrap();
                // The advertisement is complete; Git then reports that the client, here no client, hung up.
                assert!(output.stdout.windows(4).any(|window| window == b"0000"), "{output:?}");
                assert!(!marker.exists(), "the repository path was shell-injected");
            }
        }
    }
}
