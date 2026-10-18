//! The Job Object owner on the shapes of baseline row P08, through product code: a program started directly, in
//! a path with spaces, through the shell, as a GUI program, and from a process that is already in a job; and the
//! guarantees of the private `CreateProcessW`: the job's limits, a handle the helper must not inherit, a
//! grandchild that cannot break away, a creation that fails without leaving a process, and a retired job.
use super::*;
use crate::git::{
    endpoint::https_auth::{
        process_tree::EnvValue,
        runner::environment::exact,
        wiped::{Probe, Records},
    },
    spawn_gate::SpawnGate,
};
use std::{
    ffi::{OsStr, OsString},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation},
    Storage::FileSystem::GetFinalPathNameByHandleW,
    System::{
        JobObjects::{
            IsProcessInJob, JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList,
            QueryInformationJobObject as Query,
        },
        Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CreateEventW, GetCurrentProcess, OpenProcess,
            PROCESS_TERMINATE, TerminateProcess,
        },
    },
};

/// A launch under a gate of the test's own: these rows race no other spawn (`gate_tests.rs` is the ones that do).
fn spawn(command: &HelperCommand) -> io::Result<(HelperChild, ProcessTree)> {
    super::spawn(command, &SpawnGate::new().hold())
}

fn system32() -> PathBuf {
    Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32")
}

fn command(program: &Path, args: &[&str]) -> HelperCommand {
    HelperCommand {
        program: program.to_path_buf(),
        args: args.iter().map(OsString::from).collect(),
        directory: system32(),
        environment: exact(&[
            ("SystemRoot".into(), std::env::var_os("SystemRoot").unwrap()),
            ("PATH".into(), system32().into_os_string()),
            ("TEMP".into(), std::env::temp_dir().into_os_string()),
            ("TMP".into(), std::env::temp_dir().into_os_string()),
        ]),
    }
}

/// The ids of the processes in the job.
fn members(tree: &ProcessTree) -> Vec<u32> {
    // The header holds room for one id; the buffer behind it for sixty-three more.
    #[repr(C)]
    struct List {
        header: JOBOBJECT_BASIC_PROCESS_ID_LIST,
        more: [usize; 63],
    }
    let mut list: List = unsafe { std::mem::zeroed() };
    // SAFETY: the pointer and length describe `list`, which outlives the call.
    let queried = unsafe {
        Query(
            tree.raw(),
            JobObjectBasicProcessIdList,
            std::ptr::from_mut(&mut list).cast(),
            size_of::<List>() as u32,
            std::ptr::null_mut(),
        )
    };
    assert!(queried != 0, "{}", io::Error::last_os_error());
    let count = list.header.NumberOfProcessIdsInList as usize;
    // SAFETY: the list holds `count` ids from the array's first element on, within `List`.
    unsafe {
        std::slice::from_raw_parts(list.header.ProcessIdList.as_ptr(), count)
            .iter()
            .map(|id| *id as u32)
            .collect()
    }
}

async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Starts `command`, waits for `wanted` processes to be in its job, ends the job and waits for it to empty.
async fn contained_then_ended(command: HelperCommand, wanted: usize) {
    let (mut child, tree) = spawn(&command).unwrap();
    until("the program started too few processes", || {
        members(&tree).len() >= wanted
    })
    .await;
    assert!(!tree.drained());
    tree.kill();
    until("the job did not empty", || tree.drained()).await;
    assert!(child.wait().await.is_ok());
    assert!(members(&tree).is_empty());
}

fn ping(seconds: &str) -> HelperCommand {
    command(&system32().join("ping.exe"), &["-n", seconds, "127.0.0.1"])
}

#[tokio::test]
async fn a_program_started_directly_is_contained_and_ended_with_its_job() {
    contained_then_ended(ping("60"), 1).await;
}

#[tokio::test]
async fn a_program_in_a_path_with_spaces_is_contained_and_ended_with_its_job() {
    let directory = tempfile::tempdir().unwrap();
    let spaced = directory.path().join("with spaces").join("and more");
    std::fs::create_dir_all(&spaced).unwrap();
    let program = spaced.join("ping.exe");
    std::fs::copy(system32().join("ping.exe"), &program).unwrap();
    contained_then_ended(command(&program, &["-n", "60", "127.0.0.1"]), 1).await;
}

#[tokio::test]
async fn a_program_started_through_the_shell_is_contained_with_what_the_shell_starts() {
    contained_then_ended(
        command(
            &system32().join("cmd.exe"),
            &["/c", "ping -n 60 127.0.0.1 >nul"],
        ),
        2,
    )
    .await;
}

#[tokio::test]
async fn a_gui_program_is_contained_and_ended_with_its_job() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("sleeper script.vbs");
    std::fs::write(&script, "WScript.Sleep 60000\r\n").unwrap();
    let script = script.to_string_lossy().into_owned();
    contained_then_ended(
        command(
            &system32().join("wscript.exe"),
            &["//B", "//Nologo", &script],
        ),
        1,
    )
    .await;
}

#[tokio::test]
async fn a_program_started_from_a_process_that_is_already_in_a_job_is_still_contained() {
    // The test process joins a job of its own if it is not in one already (on a CI runner it is). It cannot leave
    // it; the rest of the test run is simply nested too, which is the shape this row is about.
    let mut inside = 0;
    // SAFETY: a null job asks whether the process is in any job.
    unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut inside) };
    if inside == 0 {
        let outer = ProcessTree::new().unwrap();
        // SAFETY: both handles are open.
        assert!(
            unsafe {
                windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                    outer.raw(),
                    GetCurrentProcess(),
                )
            } != 0
        );
        std::mem::forget(outer);
    }
    contained_then_ended(ping("60"), 1).await;
}

#[tokio::test]
async fn the_leaders_exit_does_not_empty_the_tree() {
    let command = command(
        &system32().join("cmd.exe"),
        &["/c", "start /b ping -n 60 127.0.0.1 >nul"],
    );
    let (mut child, tree) = spawn(&command).unwrap();
    child.wait().await.unwrap();
    assert!(
        !tree.drained(),
        "a descendant outlives its leader, and the job must say so"
    );
    tree.kill();
    until("the job did not empty", || tree.drained()).await;
}

#[tokio::test]
async fn closing_the_last_handle_of_the_job_ends_what_is_in_it() {
    let (mut child, tree) = spawn(&command(
        &system32().join("cmd.exe"),
        &["/c", "ping -n 60 127.0.0.1 >nul"],
    ))
    .unwrap();
    until("the shell started nothing", || members(&tree).len() >= 2).await;
    let observer = tree.job.try_clone().unwrap();
    drop(tree);
    // The observer is a second handle: the job is still open.
    assert!(child.try_wait().unwrap().is_none());
    drop(observer);
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    assert!(
        status.is_ok(),
        "the last handle closed and the program kept running"
    );
}

#[tokio::test]
async fn a_retired_job_leaves_what_is_in_it_running() {
    let (mut child, tree) = spawn(&command(
        &system32().join("cmd.exe"),
        &["/c", "ping -n 60 127.0.0.1 >nul"],
    ))
    .unwrap();
    until("the shell started nothing", || members(&tree).len() >= 2).await;
    let survivors = members(&tree);
    let observer = tree.job.try_clone().unwrap();
    // OQ-A: a success clears kill-on-close and closes the job. The job outlives its owner while a member does.
    tree.retire();
    drop(tree);
    drop(observer);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "a retired job's member must keep running after the last handle closes"
    );
    // The test ends what it left running, by the exact ids it saw in the job.
    for id in survivors {
        // SAFETY: opening a process by id for termination; a process that already ended fails the open.
        let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, id) };
        if !process.is_null() {
            // SAFETY: the handle is open and owned here.
            unsafe {
                TerminateProcess(process, 1);
                CloseHandle(process);
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
}

#[test]
fn a_job_has_exactly_the_documented_limits_and_no_breakaway() {
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    };
    let tree = ProcessTree::new().unwrap();
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: the pointer and length describe `limits`, which outlives the call.
    let queried = unsafe {
        Query(
            tree.raw(),
            JobObjectExtendedLimitInformation,
            std::ptr::from_mut(&mut limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    assert!(queried != 0);
    let flags = limits.BasicLimitInformation.LimitFlags;
    assert_eq!(
        flags,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
    );
    assert_eq!(
        flags & (JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK),
        0
    );
    // After retirement kill-on-close is gone and nothing else was added.
    tree.retire();
    let mut retired = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: as above.
    unsafe {
        Query(
            tree.raw(),
            JobObjectExtendedLimitInformation,
            std::ptr::from_mut(&mut retired).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(retired.BasicLimitInformation.LimitFlags, 0);
}

#[tokio::test]
async fn a_program_that_cannot_start_leaves_no_process_and_no_job_member() {
    let missing = system32().join("no such program.exe");
    let error = spawn(&command(&missing, &[])).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[tokio::test]
async fn a_creation_that_fails_after_the_job_exists_runs_nothing() {
    // The failure the old assign-then-resume design had to undo: here the job handle is not a job, so the
    // creation is refused as a whole, and the program, which would leave a file, never runs.
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("marker");
    let script = format!("echo x> \"{}\"", marker.display());
    let not_a_job = {
        // SAFETY: an unnamed event; the result is checked and owned at once.
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        assert!(!event.is_null());
        // SAFETY: `event` is a new handle that nothing else owns.
        unsafe { OwnedHandle::from_raw_handle(event) }
    };
    let result = launch::create(
        &command(&system32().join("cmd.exe"), &["/c", &script]),
        &not_a_job,
        &SpawnGate::new().hold(),
    );
    assert!(
        result.is_err(),
        "a creation in something that is not a job must be refused"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!marker.exists(), "the refused program ran");
}

#[tokio::test]
async fn a_relative_program_and_a_nul_in_an_argument_are_refused_before_anything_starts() {
    let relative = spawn(&command(Path::new("ping.exe"), &[])).err().unwrap();
    assert_eq!(relative.kind(), io::ErrorKind::InvalidInput);
    let nul = spawn(&command(&system32().join("ping.exe"), &["a\0b"]))
        .err()
        .unwrap();
    assert_eq!(nul.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn arguments_are_quoted_the_way_the_command_line_parser_reads_them_back() {
    let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    let line = launch::command_line(
        &wide(r"C:\a b\x.exe"),
        &[
            wide("plain"),
            wide(""),
            wide("two words"),
            wide(r#"say "hi""#),
            wide(r"tail\"),
            wide(r"gap \"),
        ],
    )
    .unwrap();
    assert_eq!(
        String::from_utf16(&line[..line.len() - 1]).unwrap(),
        r#""C:\a b\x.exe" plain "" "two words" "say \"hi\"" tail\ "gap \\""#
    );
}

/// This test binary as a helper through the owner: it runs the probe `name` (an `#[ignore]`d row below) and nothing
/// else. A Rust program starts in milliseconds under any load. The rows used PowerShell for this, whose start and
/// whose `Add-Type` compile of the probe took longer than a loaded hosted runner allowed (CI run 38799134384 lost
/// `a_grandchild_cannot_break_away_from_the_job` to a 60 s wait for it).
fn probe(name: &str, extra: &[(&str, String)]) -> (tempfile::TempDir, PathBuf, HelperCommand) {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output");
    let (_, module) = module_path!().split_once("::").unwrap();
    let test = format!("{module}::{name}");
    let mut command = command(
        &std::env::current_exe().unwrap(),
        &["--exact", &test, "--ignored", "--nocapture"],
    );
    // The other helpers get `System32` for a `PATH`. This one is the test binary, which may import DLLs that only the
    // process's own `PATH` finds, and without them it ends before it runs anything (`0xC0000135` on dabeest).
    command
        .environment
        .retain(|(name, _)| !name.eq_ignore_ascii_case("PATH"));
    command.environment.push((
        "PATH".into(),
        EnvValue::from_os(
            &std::env::var_os("PATH").unwrap_or_default(),
            Probe::inert(),
        ),
    ));
    command.environment.push((
        PROBE_OUT.into(),
        EnvValue::from_os(output.as_os_str(), Probe::inert()),
    ));
    for (name, value) in extra {
        command.environment.push((
            (*name).into(),
            EnvValue::from_os(OsStr::new(value), Probe::inert()),
        ));
    }
    (directory, output, command)
}

/// The environment of the probe rows below, named for them: the rows do nothing unless a row that starts them sets
/// these, so a run of the ignored rows in a shell that has an `OUT` of its own touches nothing.
const PROBE_OUT: &str = "GWZ_TEST_PROCESS_TREE_PROBE_OUT";
const PROBE_HANDLE: &str = "GWZ_TEST_PROCESS_TREE_PROBE_HANDLE";

/// How long a row waits for a probe to start, run and end: readiness, not a speed claim.
const PROBE_WITHIN: Duration = Duration::from_secs(30);

/// Runs the helper until it ends, and returns what it printed (so that a probe that ends without its answer says why).
async fn run_to_exit(command: &HelperCommand) -> String {
    use tokio::io::AsyncReadExt;
    let (mut child, tree) = spawn(command).unwrap();
    let status = tokio::time::timeout(PROBE_WITHIN, child.wait()).await;
    tree.kill();
    let mut printed = String::new();
    for (name, pipe) in [
        ("stdout", child.stdout.as_mut()),
        ("stderr", child.stderr.as_mut()),
    ] {
        let Some(pipe) = pipe else {
            continue;
        };
        let mut bytes = vec![0u8; 4096];
        let read = tokio::time::timeout(Duration::from_secs(2), pipe.read(&mut bytes)).await;
        if let Ok(Ok(count)) = read {
            printed.push_str(&format!(
                "{name}: {}\n",
                String::from_utf8_lossy(&bytes[..count])
            ));
        }
    }
    assert!(status.is_ok(), "the probe did not finish: {printed}");
    if let Ok(Ok(code)) = status {
        printed.push_str(&format!("exit status: {code}\n"));
    }
    printed
}

/// The helper side of `an_inheritable_handle_of_the_process_is_not_inherited_by_the_helper`: the probe names what
/// its handle value refers to. A bare "is the value valid" would prove nothing: handle values are small multiples of
/// four, and the helper has handles of its own.
#[test]
#[ignore = "a helper process of a row below, started by it; not a test"]
fn probe_handle_target() {
    let Some(output) = std::env::var_os(PROBE_OUT) else {
        return;
    };
    let Ok(value) = std::env::var(PROBE_HANDLE) else {
        return;
    };
    let value: i64 = value.parse().unwrap();
    let mut path = [0u16; 1024];
    // SAFETY: the pointer and length describe `path`; a value that is no handle only makes the call return 0.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            value as usize as HANDLE,
            path.as_mut_ptr(),
            path.len() as u32,
            0,
        )
    };
    let text = String::from_utf16_lossy(&path[..(length as usize).min(path.len())]);
    std::fs::write(output, format!("path={text}")).unwrap();
}

/// The helper side of `a_grandchild_cannot_break_away_from_the_job`: it asks for a process outside its job.
#[test]
#[ignore = "a helper process of a row below, started by it; not a test"]
fn probe_breakaway() {
    let Some(output) = std::env::var_os(PROBE_OUT) else {
        return;
    };
    let attempt = std::process::Command::new(system32().join("cmd.exe"))
        .args(["/c", "exit", "0"])
        .creation_flags(CREATE_BREAKAWAY_FROM_JOB)
        .spawn();
    let text = match attempt {
        Ok(mut grandchild) => {
            let _ = grandchild.kill();
            let _ = grandchild.wait();
            "ok=True err=0".to_string()
        }
        Err(error) => format!("ok=False err={}", error.raw_os_error().unwrap_or(-1)),
    };
    std::fs::write(output, text).unwrap();
}

#[tokio::test]
async fn an_inheritable_handle_of_the_process_is_not_inherited_by_the_helper() {
    // An inheritable handle to a uniquely named file in this process: any child created with "inherit
    // everything" holds it.
    let directory = tempfile::tempdir().unwrap();
    let file = std::fs::File::create(directory.path().join("inheritable-canary.txt")).unwrap();
    // SAFETY: the file's handle is open; the flag changes only whether children inherit it.
    assert!(
        unsafe {
            SetHandleInformation(
                file.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        } != 0
    );
    let value = (file.as_raw_handle() as usize as i64).to_string();

    // Control: a process created the standard way inherits it, so the probe finds the file by that handle.
    let (_dir, output, helper) = probe("probe_handle_target", &[(PROBE_HANDLE, value.clone())]);
    let status = tokio::process::Command::new(&helper.program)
        .args(&helper.args)
        .env(PROBE_HANDLE, &value)
        .env(PROBE_OUT, &output)
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let control = std::fs::read_to_string(&output).unwrap();
    assert!(
        control.contains("inheritable-canary.txt"),
        "control: {control:?}"
    );

    // The owner's helper has the same probe, the same value and not the file.
    let (_dir, output, helper) = probe("probe_handle_target", &[(PROBE_HANDLE, value)]);
    let printed = run_to_exit(&helper).await;
    let seen = std::fs::read_to_string(&output)
        .unwrap_or_else(|error| panic!("the probe left no answer ({error}): {printed}"));
    assert!(seen.starts_with("path="), "the probe did not run: {seen:?}");
    assert!(!seen.contains("inheritable-canary.txt"), "owner: {seen:?}");
    drop(file);
}

#[tokio::test]
async fn a_grandchild_cannot_break_away_from_the_job() {
    let (_dir, output, helper) = probe("probe_breakaway", &[]);
    let printed = run_to_exit(&helper).await;
    let seen = std::fs::read_to_string(&output)
        .unwrap_or_else(|error| panic!("the probe left no answer ({error}): {printed}"));
    // ERROR_ACCESS_DENIED: the job allows no breakaway.
    assert!(seen.contains("ok=False err=5"), "{seen:?}");
}

#[test]
fn an_environment_block_keeps_a_drive_variable_and_refuses_an_equals_after_the_first_unit() {
    let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    let pairs = |items: &[(&str, &str)]| -> Vec<(OsString, EnvValue)> {
        items
            .iter()
            .map(|(n, v)| {
                (
                    (*n).into(),
                    EnvValue::from_os(OsStr::new(v), Probe::inert()),
                )
            })
            .collect()
    };
    // The snapshot accepts "=C:" (a hidden per-drive directory variable), and so must the block; it sorts first.
    let block = launch::environment_block(
        &pairs(&[("b", "2"), ("=C:", r"C:\work"), ("A", "1")]),
        Probe::inert(),
    )
    .unwrap();
    let mut expected = wide(r"=C:=C:\work");
    expected.push(0);
    expected.extend(wide("A=1"));
    expected.push(0);
    expected.extend(wide("b=2"));
    expected.push(0);
    expected.push(0);
    assert_eq!(block.as_slice(), expected);
    for refused in [("A=B", "v"), ("", "v"), ("N", "a\0b"), ("N\0", "v")] {
        let error = launch::environment_block(&pairs(&[refused]), Probe::inert()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{refused:?}");
    }
}

/// P3-3: the block is one owner of exactly its size that is overwritten when it drops, built from the values'
/// own owners, which stay alive and unwiped until their own drop. The probes see both.
#[test]
fn the_environment_block_is_one_wiping_owner_of_exactly_its_size() {
    let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    let values = Records::default();
    let blocks = Records::default();
    let pairs: Vec<(OsString, EnvValue)> = vec![
        (
            "b".into(),
            EnvValue::from_os(
                OsStr::new("Authorization: Bearer token"),
                Probe::recording(&values),
            ),
        ),
        (
            "A".into(),
            EnvValue::from_os(OsStr::new("1"), Probe::inert()),
        ),
    ];
    let block = launch::environment_block(&pairs, Probe::recording(&blocks)).unwrap();
    let mut expected = wide("A=1");
    expected.push(0);
    expected.extend(wide("b=Authorization: Bearer token"));
    expected.push(0);
    expected.push(0);
    assert_eq!(block.as_slice(), expected);
    assert!(blocks.lock().unwrap().is_empty() && values.lock().unwrap().is_empty());
    drop(block);
    assert_eq!(*blocks.lock().unwrap(), vec![(expected.len(), true)]);
    assert!(
        values.lock().unwrap().is_empty(),
        "a value's owner outlives the block built from it"
    );
    drop(pairs);
    assert_eq!(
        *values.lock().unwrap(),
        vec![("Authorization: Bearer token".len(), true)]
    );
}

#[tokio::test]
async fn a_helper_started_with_a_drive_variable_in_its_snapshot_sees_it() {
    let mut start = command(&system32().join("cmd.exe"), &["/d", "/c", "echo %=Q:%"]);
    start.environment.push((
        "=Q:".into(),
        EnvValue::from_os(OsStr::new(r"Q:\drive-variable"), Probe::inert()),
    ));
    let (mut child, tree) = spawn(&start).unwrap();
    let mut seen = Vec::new();
    {
        use tokio::io::AsyncReadExt;
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut seen)
            .await
            .unwrap();
    }
    child.wait().await.unwrap();
    tree.kill();
    assert!(
        String::from_utf8_lossy(&seen).contains(r"Q:\drive-variable"),
        "{:?}",
        String::from_utf8_lossy(&seen)
    );
}

#[test]
fn the_childs_ends_of_a_pipe_are_not_inheritable_until_the_creation_window_opens() {
    use windows_sys::Win32::Foundation::GetHandleInformation;
    let inheritable = |handle: &OwnedHandle| {
        let mut flags = 0;
        // SAFETY: the handle is open.
        assert!(unsafe { GetHandleInformation(handle.as_raw_handle(), &mut flags) } != 0);
        flags & HANDLE_FLAG_INHERIT != 0
    };
    for parent in [pipe::Parent::Reads, pipe::Parent::Writes] {
        let ends = pipe::create(parent).unwrap();
        assert!(
            !inheritable(&ends.parent),
            "the parent end is never inheritable"
        );
        assert!(
            !inheritable(&ends.child),
            "the child end must not be inheritable while the command line and the environment are built"
        );
    }
}

#[test]
fn the_creation_window_makes_the_childs_ends_inheritable_only_while_it_is_open() {
    use windows_sys::Win32::Foundation::GetHandleInformation;
    let inheritable = |handle: &OwnedHandle| {
        let mut flags = 0;
        // SAFETY: the handle is open.
        assert!(unsafe { GetHandleInformation(handle.as_raw_handle(), &mut flags) } != 0);
        flags & HANDLE_FLAG_INHERIT != 0
    };
    let (a, b, c) = (
        pipe::create(pipe::Parent::Writes).unwrap(),
        pipe::create(pipe::Parent::Reads).unwrap(),
        pipe::create(pipe::Parent::Reads).unwrap(),
    );
    let gate = SpawnGate::new();
    {
        let hold = gate.hold();
        let _window = launch::InheritWindow::open(&hold, [&a.child, &b.child, &c.child]).unwrap();
        assert!(inheritable(&a.child) && inheritable(&b.child) && inheritable(&c.child));
        assert!(!inheritable(&a.parent) && !inheritable(&b.parent) && !inheritable(&c.parent));
    }
    assert!(!inheritable(&a.child) && !inheritable(&b.child) && !inheritable(&c.child));
}

#[tokio::test]
async fn a_creation_that_fails_leaves_the_childs_ends_closed_to_inheritance() {
    // A refused creation (a relative program fails before the window; a missing one fails inside it) must leave
    // no inheritable handle behind: the ends are closed with the failed call, and the window is closed first.
    let missing = system32().join("no such program.exe");
    assert_eq!(
        spawn(&command(&missing, &[])).err().unwrap().kind(),
        io::ErrorKind::NotFound
    );
}

// The spawn gate against the creation window (WH2 contract C9; operator decision D25). The window is opened by hand
// over three pipe ends, which is the whole of what the production launch does around `CreateProcessW`, so that a
// standard spawn can be made to land inside it every time instead of by chance.

/// A probe that prints the name of every pipe it holds a handle to: the helper pipes' names are unique, so the
/// output says which of a launch's ends a process inherited and nothing about other tests' handles.
/// The helper side of the gate rows: it names every pipe it holds other than its own standard handles (a name query
/// on a synchronous pipe that is busy waits for the read or write in flight, and the test run's pipes are busy).
#[test]
#[ignore = "a helper process of a row below, started by it; not a test"]
fn probe_pipe_names() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};
    let Some(output) = std::env::var_os(PROBE_OUT) else {
        return;
    };
    let own = [
        std::io::stdin().as_raw_handle() as usize,
        std::io::stdout().as_raw_handle() as usize,
        std::io::stderr().as_raw_handle() as usize,
    ];
    let mut found = Vec::new();
    let mut unnamed = 0;
    let mut anonymous = 0;
    let mut foreign = 0;
    let mut private = 0;
    let mut other = 0;
    let mut values: Vec<String> = Vec::new();
    for value in (4..65536usize).step_by(4) {
        if own.contains(&value) {
            continue;
        }
        let handle = value as HANDLE;
        // SAFETY: a value that is no handle only makes the call answer FILE_TYPE_UNKNOWN.
        if unsafe { GetFileType(handle) } != FILE_TYPE_PIPE {
            continue;
        }
        match handle_pipe_name(handle) {
            // A handle that answers as a pipe and has no name is an anonymous pipe, if it also answers the pipe query;
            // a socket answers as a pipe, has no name and does not (`foreign`). Only one that is inheritable counts
            // (`anonymous`): a handle is inheritable here because the process that started this one made it
            // inheritable, and a pipe this process made for itself is not (`private`).
            Some(name) if name.is_empty() => {
                if !is_pipe(handle) {
                    foreign += 1;
                } else if is_inheritable(handle) {
                    // CreatePipe with no size asks for the system's default, 4096 bytes each way: a pipe of
                    // another size is another maker's (a test run's own, in the process that started this one).
                    let summary = pipe_summary(handle);
                    if summary.default_buffers() {
                        anonymous += 1;
                    } else {
                        other += 1;
                    }
                    values.push(format!("{value:x}/{summary}"));
                } else {
                    private += 1;
                }
            }
            Some(name) => found.push(name),
            None => unnamed += 1,
        }
    }
    // The other lines count the pipes with no name, which are the anonymous ones CreatePipe makes (gwz-sspi's worker
    // pipes among them), and the pipes whose name could not be read.
    std::fs::write(
        output,
        format!(
            "names={}\nunnamed={unnamed}\nanonymous={anonymous}\nforeign={foreign}\nprivate={private}\nother={other}\nvalues={}",
            found.join(";"),
            values.join(",")
        ),
    )
    .unwrap();
}

/// What a pipe end says of itself: its flags and buffer sizes.
struct PipeSummary {
    answered: bool,
    flags: u32,
    out: u32,
    input: u32,
    instances: u32,
}

impl PipeSummary {
    /// CreatePipe's default: 4096 bytes each way.
    fn default_buffers(&self) -> bool {
        self.answered && self.out == 4096 && self.input == 4096
    }
}

impl std::fmt::Display for PipeSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{:x}:{}:{}:{}",
            u8::from(self.answered),
            self.flags,
            self.out,
            self.input,
            self.instances
        )
    }
}

fn pipe_summary(handle: HANDLE) -> PipeSummary {
    use windows_sys::Win32::System::Pipes::GetNamedPipeInfo;
    let mut summary = PipeSummary {
        answered: false,
        flags: 0,
        out: 0,
        input: 0,
        instances: 0,
    };
    // SAFETY: initialized outputs; a value that is no pipe only makes the call fail.
    summary.answered = unsafe {
        GetNamedPipeInfo(
            handle,
            &mut summary.flags,
            &mut summary.out,
            &mut summary.input,
            &mut summary.instances,
        )
    } != 0;
    summary
}

/// Whether `handle` is inheritable.
fn is_inheritable(handle: HANDLE) -> bool {
    let mut flags = 0;
    // SAFETY: an initialized output; a value that is no handle only makes the call fail.
    let answered =
        unsafe { windows_sys::Win32::Foundation::GetHandleInformation(handle, &mut flags) };
    answered != 0 && flags & HANDLE_FLAG_INHERIT != 0
}

/// Whether `handle` is an end of a pipe, which a socket that answers `FILE_TYPE_PIPE` is not.
fn is_pipe(handle: HANDLE) -> bool {
    use windows_sys::Win32::System::Pipes::GetNamedPipeInfo;
    let mut flags = 0;
    // SAFETY: an initialized output; a value that is no pipe only makes the call fail.
    let answered = unsafe {
        GetNamedPipeInfo(
            handle,
            &mut flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    answered != 0
}

/// The pipe a handle is an end of, as the system names it (`\gwz-helper-...`).
fn pipe_name(end: &OwnedHandle) -> String {
    handle_pipe_name(end.as_raw_handle())
        .unwrap_or_else(|| panic!("{}", io::Error::last_os_error()))
}

/// The name the system gives the pipe `handle` is an end of, or `None` if the query fails.
fn handle_pipe_name(handle: HANDLE) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_INFO, FileNameInfo, GetFileInformationByHandleEx,
    };
    let mut buffer = vec![0u32; 512];
    // SAFETY: the pointer and length describe `buffer`, which outlives the call; a value that is no open handle only
    // makes the call fail.
    let read = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileNameInfo,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * size_of::<u32>()) as u32,
        )
    };
    if read == 0 {
        return None;
    }
    let info = buffer.as_ptr().cast::<FILE_NAME_INFO>();
    // SAFETY: the call filled the length and that many bytes of name after it, inside `buffer`.
    let name = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*info).FileName).cast::<u16>(),
            (*info).FileNameLength as usize / 2,
        )
    };
    String::from_utf16(name).ok()
}

/// Three pipes the way a launch makes them (stdin, stdout, stderr), and the names of their child ends.
fn three_pipes() -> ([pipe::Pipe; 3], Vec<String>) {
    let pipes = [
        pipe::create(pipe::Parent::Writes).unwrap(),
        pipe::create(pipe::Parent::Reads).unwrap(),
        pipe::create(pipe::Parent::Reads).unwrap(),
    ];
    let names: Vec<String> = pipes.iter().map(|pipe| pipe_name(&pipe.child)).collect();
    assert!(
        names.iter().all(|name| name.starts_with(r"\gwz-helper-")),
        "{names:?}"
    );
    (pipes, names)
}

/// The probe as a standard `Command`, which names no handle list.
fn standard_probe() -> (tempfile::TempDir, PathBuf, std::process::Command) {
    let (directory, output, helper) = probe("probe_pipe_names", &[]);
    let mut command = std::process::Command::new(&helper.program);
    command.args(&helper.args).env(PROBE_OUT, &output);
    (directory, output, command)
}

#[test]
fn without_the_gate_a_standard_spawn_inside_an_open_window_inherits_the_childs_ends() {
    // The control: without the gate the exposure is real, so the row below means something.
    let (pipes, names) = three_pipes();
    let gate = SpawnGate::new();
    let hold = gate.hold();
    let window =
        launch::InheritWindow::open(&hold, [&pipes[0].child, &pipes[1].child, &pipes[2].child])
            .unwrap();
    let (_directory, output, mut command) = standard_probe();
    let status = command.status().unwrap();
    drop(window);
    drop(hold);
    assert!(status.success());
    let seen = std::fs::read_to_string(&output).unwrap();
    for name in &names {
        assert!(
            seen.contains(name.as_str()),
            "control: {name} not in {seen:?}"
        );
    }
}

#[test]
fn a_spawn_through_the_gate_waits_for_the_window_and_inherits_none_of_the_ends() {
    let (pipes, names) = three_pipes();
    let gate = SpawnGate::new();
    let hold = gate.hold();
    let window =
        launch::InheritWindow::open(&hold, [&pipes[0].child, &pipes[1].child, &pipes[2].child])
            .unwrap();
    let (_directory, output, mut command) = standard_probe();
    let (spawned, started) = std::sync::mpsc::channel();
    let spawner = {
        let gate = gate.clone();
        std::thread::spawn(move || {
            let mut child = gate.spawn(&mut command).unwrap();
            spawned.send(()).unwrap();
            child.wait().unwrap().success()
        })
    };
    // The spawn is asked for while the window is open, and does not happen.
    assert!(
        started.recv_timeout(Duration::from_millis(750)).is_err(),
        "a standard spawn went ahead inside the creation window"
    );
    drop(window);
    drop(hold);
    started
        .recv_timeout(Duration::from_secs(60))
        .expect("the spawn goes ahead once the window is closed");
    assert!(spawner.join().unwrap());
    let seen = std::fs::read_to_string(&output).unwrap();
    assert!(seen.starts_with("names="), "the probe ran: {seen:?}");
    for name in &names {
        assert!(
            !seen.contains(name.as_str()),
            "the gated spawn inherited {name}: {seen:?}"
        );
    }
}

mod sspi_window;
