//! Fake credential helpers for the process owner's tests, one script dialect per platform (step 4.2).
//!
//! Each [`Behavior`] is written once as a `/bin/sh` script and once as a `.cmd` batch file, so that the owner's
//! tests are the same tests on both platforms. A descendant of a helper is a loop that appends to a heartbeat
//! file: a descendant that is still alive is a file that is still growing. Step 4.3 brings the Windows form of
//! the older `helper_script` fixtures; these stay the small fixtures of the owner's own tests.
use super::*;
use std::{fs, path::Path};

/// What a fake helper does.
#[derive(Clone, Copy)]
pub(super) enum Behavior {
    /// Prints a credential and exits.
    Answer,
    /// Starts a background descendant that keeps the helper's output pipes, and exits at once.
    ExitsLeavingDescendantOnOutput,
    /// Starts a background descendant that keeps the helper's output pipes, and then runs for a long time.
    RunsWithDescendantOnOutput,
    /// Starts a background descendant with no pipes of its own, prints a credential and exits.
    AnswersLeavingDetachedDescendant,
    /// Prints a credential, then creates the marker file and exits.
    AnswersAndMarks,
    /// Runs for a long time and never reads its input.
    Hangs,
    /// Writes 40,000 bytes to its diagnostic output, prints a credential and exits.
    FloodsDiagnosticsThenAnswers,
    /// Exits with failure if the environment holds `GIT_ASKPASS` under any capitalization.
    RefusesAskPass,
}

/// A directory holding one fake helper, its heartbeat file and the configuration that names them. The helper's
/// background loops run only while the fixture's `alive` file exists, so that dropping the fixture ends every
/// loop a test left behind, however the test ended.
pub(super) struct Fixture {
    directory: tempfile::TempDir,
    alive: PathBuf,
    pub(super) config: Config,
    arguments: Vec<String>,
    pub(super) heartbeat: PathBuf,
    pub(super) marker: PathBuf,
}

impl Fixture {
    /// A helper under a directory named `with spaces`, which is every Windows shape's hard case and Unix's own.
    pub(super) fn new(behavior: Behavior) -> Self {
        Self::named(behavior, "helper dir")
    }

    pub(super) fn named(behavior: Behavior, directory_name: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join(directory_name);
        fs::create_dir_all(&home).unwrap();
        let heartbeat = home.join("heartbeat");
        let marker = home.join("marker");
        let alive = home.join("alive");
        fs::write(&alive, "").unwrap();
        let (executable, arguments) = write(&home, behavior);
        let mut environment = platform_environment();
        environment.push(("HEARTBEAT".into(), heartbeat.as_os_str().to_owned()));
        environment.push(("MARKER".into(), marker.as_os_str().to_owned()));
        environment.push(("HELPER_HOME".into(), home.as_os_str().to_owned()));
        Self {
            directory,
            alive,
            config: Config {
                executable,
                environment,
            },
            arguments,
            heartbeat,
            marker,
        }
    }

    /// The arguments that run the fake helper after `config.executable`: none for a Unix script, the shell's
    /// `/d /c script` for a Windows batch file.
    pub(super) fn argv(&self) -> Vec<&str> {
        self.arguments.iter().map(String::as_str).collect()
    }

    pub(super) fn heartbeats(&self) -> u64 {
        fs::metadata(&self.heartbeat).map_or(0, |metadata| metadata.len())
    }

    pub(super) fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// Waits until the descendant has written at least once.
    pub(super) async fn started(&self) {
        let until = Instant::now() + Duration::from_secs(10);
        while self.heartbeats() < 2 {
            assert!(Instant::now() < until, "the descendant must have written");
            sleep(Duration::from_millis(5)).await;
        }
    }

    /// Asserts that the descendant has stopped: after the cleanup grace, the heartbeat no longer grows.
    pub(super) async fn assert_stopped(&self, message: &str) {
        sleep(CLEANUP_GRACE).await;
        let length = self.heartbeats();
        sleep(Duration::from_millis(300)).await;
        assert_eq!(self.heartbeats(), length, "{message}");
    }

    /// Asserts that the descendant is still running: the heartbeat grows.
    pub(super) async fn assert_alive(&self, message: &str) {
        sleep(CLEANUP_GRACE).await;
        let length = self.heartbeats();
        sleep(Duration::from_millis(300)).await;
        assert!(self.heartbeats() > length, "{message}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Every background loop of this fixture ends at its next pass. The check that follows is the group's
        // guard against a leaked loop: once the fixture is gone, no heartbeat may keep growing.
        let _ = fs::remove_file(&self.alive);
        if self.heartbeats() == 0 {
            return;
        }
        let until = std::time::Instant::now() + Duration::from_secs(10);
        let mut last = self.heartbeats();
        loop {
            std::thread::sleep(Duration::from_millis(300));
            let now = self.heartbeats();
            if now == last {
                return;
            }
            last = now;
            if std::time::Instant::now() >= until {
                break;
            }
        }
        if !std::thread::panicking() {
            panic!(
                "a heartbeat loop outlived its fixture: {}",
                self.heartbeat.display()
            );
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use crate::git::endpoint::helper_script::write_helper_script;

        /// What the fake helpers' environment needs besides what a test adds: nothing on Unix, where scripts name
        /// the programs they run by absolute path.
        pub(super) fn platform_environment() -> Vec<(OsString, OsString)> {
            Vec::new()
        }

        const LOOP: &str = "/bin/sh -c 'while [ -e \"$HELPER_HOME/alive\" ]; do printf x >> \"$HEARTBEAT\"; /bin/sleep 0.02; done'";

        fn write(home: &Path, behavior: Behavior) -> (PathBuf, Vec<String>) {
            let path = home.join("helper");
            let body = match behavior {
                Behavior::Answer => "printf 'username=alice\\npassword=token\\n'".to_string(),
                Behavior::ExitsLeavingDescendantOnOutput => format!("{LOOP} &\nexit 0"),
                Behavior::RunsWithDescendantOnOutput => format!("{LOOP} &\n/bin/sleep 30"),
                Behavior::AnswersLeavingDetachedDescendant => format!(
                    "{LOOP} 0</dev/null 1>/dev/null 2>/dev/null &\nprintf 'username=alice\\npassword=token\\n'\nexit 0"
                ),
                Behavior::AnswersAndMarks => {
                    "printf 'username=alice\\npassword=token\\n'\nexec 1>&-\nprintf x > \"$MARKER\"\nexit 0".to_string()
                }
                Behavior::Hangs => "/bin/sleep 30".to_string(),
                Behavior::FloodsDiagnosticsThenAnswers => {
                    "/usr/bin/head -c 40000 /dev/zero >&2\nprintf 'username=alice\\npassword=token\\n'".to_string()
                }
                Behavior::RefusesAskPass => {
                    "if [ \"${GIT_ASKPASS+x}\" = x ]; then exit 7; fi\nprintf 'username=alice\\npassword=token\\n'".to_string()
                }
            };
            write_helper_script(&path, &body);
            (path, Vec::new())
        }
    } else if #[cfg(windows)] {
        /// What the fake helpers' environment needs besides what a test adds: the Windows directory, and the
        /// system directories, where `cmd` finds `ping` and `powershell`. A helper's environment is the snapshot
        /// and nothing else, so a test supplies them as a snapshot would.
        pub(super) fn platform_environment() -> Vec<(OsString, OsString)> {
            let root = std::env::var_os("SystemRoot").expect("Windows sets SystemRoot");
            let system = Path::new(&root).join("System32");
            let path = std::env::join_paths([system.clone(), system.join("WindowsPowerShell").join("v1.0")]).unwrap();
            vec![("SystemRoot".into(), root.clone()), ("PATH".into(), path)]
        }

        const LOOP_FILE: &str = "loop.cmd";
        const LOOP: &str = "@echo off\r\n:again\r\nif not exist \"%HELPER_HOME%\\alive\" exit /b\r\necho x>>\"%HEARTBEAT%\"\r\nping -n 1 127.0.0.1 >nul\r\ngoto again\r\n";
        const DETACH_FILE: &str = "detach.ps1";
        /// Starts the loop in a console of its own, hidden. A batch file cannot close the pipes it inherited, and
        /// `start /b` shares them, so a descendant with no pipe of the helper's is started by PowerShell, which
        /// creates it without inheriting handles.
        const DETACH: &str = "Start-Process -WindowStyle Hidden -FilePath cmd.exe -ArgumentList @('/c', ('\"{0}\"' -f (Join-Path $PSScriptRoot 'loop.cmd')))\r\n";
        const ANSWER: &str = "echo username=alice\r\necho password=token\r\n";

        fn write(home: &Path, behavior: Behavior) -> (PathBuf, Vec<String>) {
            let path = home.join("helper.cmd");
            fs::write(home.join(LOOP_FILE), LOOP).unwrap();
            fs::write(home.join(DETACH_FILE), DETACH).unwrap();
            let start = format!("start \"\" /b cmd /d /c call \"%~dp0{LOOP_FILE}\"");
            let body = match behavior {
                Behavior::Answer => ANSWER.to_string(),
                Behavior::ExitsLeavingDescendantOnOutput => format!("{start}\r\nexit /b 0\r\n"),
                Behavior::RunsWithDescendantOnOutput => format!("{start}\r\nping -n 30 127.0.0.1 >nul\r\n"),
                Behavior::AnswersLeavingDetachedDescendant => {
                    format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{DETACH_FILE}\" <nul >nul 2>nul\r\n{ANSWER}exit /b 0\r\n")
                }
                Behavior::AnswersAndMarks => {
                    format!("{ANSWER}echo x> \"%MARKER%\"\r\nexit /b 0\r\n")
                }
                Behavior::Hangs => "ping -n 30 127.0.0.1 >nul\r\n".to_string(),
                Behavior::FloodsDiagnosticsThenAnswers => format!(
                    "for /l %%i in (1,1,400) do echo 0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789>&2\r\n{ANSWER}"
                ),
                Behavior::RefusesAskPass => {
                    format!("if defined GIT_ASKPASS exit /b 7\r\n{ANSWER}")
                }
            };
            fs::write(&path, format!("@echo off\r\n{body}")).unwrap();
            let shell = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32").join("cmd.exe");
            (shell, vec!["/d".into(), "/c".into(), path.to_string_lossy().into_owned()])
        }
    }
}
