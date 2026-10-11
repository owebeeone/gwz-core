//! A native helper process for the Windows SSH test servers (GwzTransportWindowsParityPlan.md, step 1.5).
//!
//! `dropped_close_fixture` (`ssh_close_fixture`) needs a server whose session dies while a channel is closing.
//! On Unix a shell script walks up to the server's own session process with `ps` and kills it with `kill -9`. A
//! Windows script cannot: the shell that `sshd.exe` starts is a POSIX `bash.exe` (or `cmd.exe` with an `sh.exe`
//! under it), whose `ps` lists MSYS processes by MSYS ids. The helper is native: it lists the Windows processes
//! above itself and ends the server's by exact id.
//!
//! The helper is the test binary itself, run again. A script names the test executable and sets [`HELPER_ENV`]; a
//! process constructor ([`CONSTRUCTOR`]) that runs before the test harness starts sees the variable, does what the
//! mode says and never reaches the harness, whose banner would otherwise go into the channel. Test-only: the
//! product has no such process.
//!
//! [`Mode::FakeGit`] is a `git` for the credential-helper tests (plan step 4.11). A Windows helper is created as a
//! program, never as a script, so a test cannot write its fake `git` as a script as a Unix test does. The fake is
//! the test executable run in this mode, named by the helper's `Config`: [`GIT_SCRIPT_ENV`] names a POSIX `sh`
//! script and [`GIT_SH_ENV`] the `sh.exe` that runs it (Git for Windows', which also provides the tools such a
//! script calls), and the mode runs the script with the arguments `git` was given, the helper's own standard
//! handles and environment, and exits with the script's status. The script is the body the Unix test writes, so a
//! test reads the same on both platforms (`helper_script::git_fixture`).
//!
//! [`Mode::DropSession`] ends the server's session processes. It does not look at the processes around it: the
//! fixture put the server in a named Job Object ([`JOB_ENV`] names it, in the server's environment, which its
//! sessions inherit), and the helper lists that job's members and ends the `sshd.exe` ones whose parent is also a
//! member. The listener, whose parent is the test, is the job's one other `sshd.exe` and is left alone. Only
//! processes the fixture's own job holds are ever ended, by exact id; nothing is walked up to, and nothing is named
//! by image except to tell the job's `sshd.exe` sessions from its other members.
//!
//! What the helper cannot do is end a channel's output and keep the channel open. A Windows `sshd.exe` sends the
//! channel's EOF when the session's process exits and not when its output pipes close: a helper that closed every
//! handle to the output pipes, in every process that held one (found through the system handle table), got no EOF
//! until the process exited, when EOF, the exit status and CHANNEL_CLOSE arrived together (dabeest, 2026-10-11).
//! The rows that need that state stay Unix.
use super::fixture_host::posix_path;
use std::{env, mem::size_of, time::Duration};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        JobObjects::{
            JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, OpenJobObjectW,
            QueryInformationJobObject,
        },
        Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    },
};

/// The access right to list a job's processes (`JOB_OBJECT_QUERY`, which `windows-sys` does not export).
const JOB_OBJECT_QUERY: u32 = 0x0004;

/// The variable that names the fixture's Job Object, in the server's environment.
pub(crate) const JOB_ENV: &str = "GWZ_FIXTURE_JOB";

/// The environment variable that turns a run of the test executable into the helper; its value is a [`Mode`] name.
pub(crate) const HELPER_ENV: &str = "GWZ_FIXTURE_HELPER";

/// Names the `sh` script a [`Mode::FakeGit`] run executes.
pub(crate) const GIT_SCRIPT_ENV: &str = "GWZ_FIXTURE_GIT_SCRIPT";

/// Names the `sh.exe` that runs a [`Mode::FakeGit`] script.
pub(crate) const GIT_SH_ENV: &str = "GWZ_FIXTURE_GIT_SH";

/// The variable that holds the `PATH` the server was started with, which `cmd-session` gives `cmd.exe` as its own.
pub(crate) const SERVER_PATH_ENV: &str = "GWZ_FIXTURE_SERVER_PATH";

/// How long the helper stays alive, at the most, whatever happens to the test that started it.
const LIFETIME: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// End the server's session processes, and stay alive.
    DropSession,
    /// Run the client's command (`SSH_ORIGINAL_COMMAND`) as `sshd.exe` runs it under a `cmd.exe` default shell:
    /// `cmd.exe /c` and the raw command line. For a host whose own default shell is another, where it stands in
    /// for the one it does not have.
    CmdSession,
    /// Run the script [`GIT_SCRIPT_ENV`] names under the `sh` [`GIT_SH_ENV`] names, with this process's arguments, as
    /// a fake `git` does, and return its exit status.
    FakeGit,
}

impl Mode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::DropSession => "drop-session",
            Self::CmdSession => "cmd-session",
            Self::FakeGit => "fake-git",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::DropSession, Self::CmdSession, Self::FakeGit]
            .into_iter()
            .find(|mode| mode.name() == name)
    }
}

/// The command of a POSIX forced script that runs the helper in `mode`: the test executable with [`HELPER_ENV`].
pub(crate) fn script_line(mode: Mode) -> String {
    let executable = posix_path(&env::current_exe().unwrap().display().to_string());
    assert!(
        !executable.contains('\''),
        "the test executable's path has a single quote, which the script cannot quote"
    );
    format!("{HELPER_ENV}={} '{executable}'", mode.name())
}

// The process constructor: the C runtime calls what is in `.CRT$XCU` before `main`, so a helper run never starts
// the test harness.
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static CONSTRUCTOR: extern "C" fn() = constructor;

extern "C" fn constructor() {
    let Some(mode) = env::var(HELPER_ENV)
        .ok()
        .and_then(|name| Mode::parse(&name))
    else {
        return;
    };
    if mode == Mode::CmdSession {
        std::process::exit(cmd_session());
    }
    if mode == Mode::FakeGit {
        std::process::exit(fake_git());
    }
    run(mode);
    std::thread::sleep(LIFETIME);
    std::process::exit(0);
}

/// What the helper did, for a test that fails: the file `GWZ_FIXTURE_HELPER_LOG` names, else `helper.log` in the
/// working directory, which is the fixture's (and is gone with it).
fn note(line: &str) {
    use std::io::Write;
    let path = env::var_os("GWZ_FIXTURE_HELPER_LOG").unwrap_or_else(|| "helper.log".into());
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
    {
        let _ = writeln!(log, "{line}");
    }
}

/// Runs the fake `git`'s script under its `sh`, with the arguments this process was started with, and returns the
/// script's exit status: 255 when the script cannot run, which the note says.
fn fake_git() -> i32 {
    let (Some(shell), Some(script)) = (env::var_os(GIT_SH_ENV), env::var_os(GIT_SCRIPT_ENV)) else {
        note("fake-git: no shell or script named");
        return 255;
    };
    let mut command = std::process::Command::new(&shell);
    // The tools a script calls are beside its shell; the helper's environment, which a test gives, may name none.
    let mut path = std::ffi::OsString::new();
    if let Some(directory) = std::path::Path::new(&shell).parent() {
        path.push(directory);
    }
    if let Some(existing) = env::var_os("PATH") {
        path.push(";");
        path.push(existing);
    }
    command.env("PATH", path);
    // The helper's mode is this process's and not the script's.
    command.env_remove(HELPER_ENV);
    match command.arg(&script).args(env::args_os().skip(1)).status() {
        Ok(status) => status.code().unwrap_or(255),
        Err(error) => {
            note(&format!("fake-git: {error}"));
            255
        }
    }
}

/// Runs the client's command line under `cmd.exe /c` as `sshd.exe` does, with this process's standard handles, and
/// returns its exit code.
fn cmd_session() -> i32 {
    use std::os::windows::process::CommandExt;
    let Some(command) = env::var_os("SSH_ORIGINAL_COMMAND") else {
        note("cmd-session: no SSH_ORIGINAL_COMMAND");
        return 255;
    };
    let mut cmd = std::process::Command::new("cmd.exe");
    // The server's own `PATH`: a POSIX shell between the server and this process puts its directories first.
    if let Some(path) = env::var_os(SERVER_PATH_ENV) {
        cmd.env("PATH", path);
    }
    let status = cmd.arg("/c").raw_arg(&command).status();
    match status {
        Ok(status) => status.code().unwrap_or(255),
        Err(error) => {
            note(&format!("cmd-session: {error}"));
            255
        }
    }
}

fn run(mode: Mode) {
    match mode {
        Mode::CmdSession | Mode::FakeGit => {}
        Mode::DropSession => {
            let Some(name) = env::var(JOB_ENV).ok() else {
                note("drop-session: no fixture job named");
                return;
            };
            let Some(members) = job_members(&name) else {
                note(&format!("drop-session: cannot list the job {name}"));
                return;
            };
            for process in session_processes(&processes(), &members) {
                note(&format!("ending {} {}", process.id, process.name));
                terminate(process.id);
            }
        }
    }
}

struct Process {
    id: u32,
    parent: u32,
    name: String,
}

impl Process {
    fn is_sshd(&self) -> bool {
        self.name.eq_ignore_ascii_case("sshd.exe")
    }
}

/// The `sshd.exe` session processes among `members`, the processes a fixture's job holds: those whose parent is a
/// member too. The listener's parent is the test, which is not a member.
fn session_processes<'a>(all: &'a [Process], members: &[u32]) -> Vec<&'a Process> {
    all.iter()
        .filter(|process| {
            members.contains(&process.id) && members.contains(&process.parent) && process.is_sshd()
        })
        .collect()
}

/// The ids of the processes in the job called `name`, or `None` when it cannot be opened or listed.
fn job_members(name: &str) -> Option<Vec<u32>> {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let job = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, 0, wide.as_ptr()) };
    if job.is_null() {
        return None;
    }
    // The list is a header and as many ids as fit after it.
    let mut buffer = vec![0_usize; 1024];
    let mut returned = 0_u32;
    let listed = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * size_of::<usize>()) as u32,
            &mut returned,
        )
    };
    unsafe { CloseHandle(job) };
    if listed == 0 {
        return None;
    }
    // SAFETY: the buffer is aligned for usize, which the list's header and ids are made of, and the kernel filled it.
    let list = unsafe { &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
    let count = list.NumberOfProcessIdsInList as usize;
    let ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
    Some(ids.iter().map(|id| *id as u32).collect())
}

/// Every process now: its id, its parent's and its image name.
fn processes() -> Vec<Process> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut all = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let length = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(0);
        all.push(Process {
            id: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            name: String::from_utf16_lossy(&entry.szExeFile[..length]),
        });
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    all
}

fn terminate(id: u32) {
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, id) };
    if !process.is_null() {
        unsafe {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::endpoint::fixture_job::ProcessJob;
    use std::process::{Command, Stdio};

    fn process(id: u32, parent: u32, name: &str) -> Process {
        Process {
            id,
            parent,
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_mode_names_are_the_modes_it_parses() {
        for mode in [Mode::DropSession, Mode::CmdSession, Mode::FakeGit] {
            assert_eq!(Mode::parse(mode.name()), Some(mode));
        }
        assert_eq!(Mode::parse("sleep"), None);
    }

    #[test]
    fn the_script_line_runs_the_test_executable_in_the_mode() {
        let line = script_line(Mode::DropSession);
        assert!(
            line.starts_with("GWZ_FIXTURE_HELPER=drop-session '/"),
            "{line}"
        );
        assert!(line.ends_with(".exe'"), "{line}");
    }

    #[test]
    fn the_session_processes_are_the_jobs_sshd_below_the_listener() {
        // The job: listener (parent: the test, not a member) -> monitor -> session -> bash; and an sshd.exe that is
        // not in the job (a login that started the test run, say), which is never looked at.
        let all = [
            process(6, 5, "sshd.exe"),
            process(7, 6, "SSHD.EXE"),
            process(8, 7, "sshd.exe"),
            process(9, 8, "bash.exe"),
            process(2, 1, "sshd.exe"),
            process(3, 2, "sshd.exe"),
        ];
        let ids: Vec<u32> = session_processes(&all, &[6, 7, 8, 9])
            .iter()
            .map(|process| process.id)
            .collect();
        assert_eq!(ids, vec![7, 8]);
        assert!(
            session_processes(&all, &[6]).is_empty(),
            "a lone sshd is the listener"
        );
    }

    #[test]
    fn the_members_of_a_named_job_are_its_processes_and_a_missing_job_is_none() {
        let name = format!("gwz-fixture-test-{}", std::process::id());
        let job = ProcessJob::named(&name).unwrap();
        let mut child = Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 > nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        job.adopt(&child).unwrap();
        let members = job_members(&name);
        job.terminate();
        let _ = child.wait();
        assert!(members.expect("the job lists").contains(&child.id()));
        assert!(job_members("gwz-fixture-test-no-such-job").is_none());
    }

    #[test]
    fn the_cmd_session_runs_the_raw_command_line_under_cmd_and_returns_its_exit_code() {
        // `&` is `cmd.exe`'s own syntax: it is only a separator if the line reaches `cmd.exe /c` as it came.
        let output = Command::new(env::current_exe().unwrap())
            .env(HELPER_ENV, Mode::CmdSession.name())
            .env("SSH_ORIGINAL_COMMAND", "echo carried & exit /b 3")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("carried"),
            "{output:?}"
        );
    }

    #[test]
    fn the_fake_git_runs_its_script_with_the_arguments_the_environment_and_the_standard_handles_it_was_given() {
        use crate::git::endpoint::helper_script::git_fixture;
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let config = git_fixture(
            directory.path(),
            "input=$(cat)\nprintf 'args=%s|%s|%s\\n' \"$1\" \"$2\" \"$3\"\nprintf 'input=%s\\n' \"$input\"\nprintf 'kept=%s\\n' \"$KEPT\"\nprintf 'diagnostic\\n' >&2\nexit 7",
        );
        let mut child = Command::new(&config.executable)
            .args(["-c", "core.askPass=", "credential", "fill"])
            .env_clear()
            .envs(config.environment.iter().cloned())
            .env("KEPT", "snapshot")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"url=https://example.test/repo\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(7), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("args=-c|core.askPass=|credential"), "{stdout}");
        assert!(stdout.contains("input=url=https://example.test/repo"), "{stdout}");
        assert!(stdout.contains("kept=snapshot"), "{stdout}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("diagnostic"), "{output:?}");
    }

    #[test]
    fn the_fake_git_hands_the_config_subcommand_to_the_real_git_with_no_configuration_of_the_host() {
        use crate::git::endpoint::helper_script::git_fixture;
        let directory = tempfile::tempdir().unwrap();
        let config = git_fixture(directory.path(), "exit 9");
        let output = Command::new(&config.executable)
            .args(["config", "--list"])
            .env_clear()
            .envs(config.environment.iter().cloned())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        // The real Git answered (an empty list: no system or global file), not the script (which exits 9).
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
    }

    #[test]
    fn a_fake_git_with_no_script_fails_and_says_so() {
        let output = Command::new(env::current_exe().unwrap())
            .env(HELPER_ENV, Mode::FakeGit.name())
            .env_remove(GIT_SCRIPT_ENV)
            .env_remove(GIT_SH_ENV)
            .env(
                "GWZ_FIXTURE_HELPER_LOG",
                env::temp_dir().join("gwz-helper-noscript.log"),
            )
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(255));
        let _ = std::fs::remove_file(env::temp_dir().join("gwz-helper-noscript.log"));
    }

    #[test]
    fn the_cmd_session_without_a_command_fails_and_says_so() {
        let output = Command::new(env::current_exe().unwrap())
            .env(HELPER_ENV, Mode::CmdSession.name())
            .env_remove("SSH_ORIGINAL_COMMAND")
            .env(
                "GWZ_FIXTURE_HELPER_LOG",
                env::temp_dir().join("gwz-helper-nocommand.log"),
            )
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(255));
        let _ = std::fs::remove_file(env::temp_dir().join("gwz-helper-nocommand.log"));
    }
}
