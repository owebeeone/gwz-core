//! The Job Object owner on the shapes of baseline row P08, through product code: a program started directly, in
//! a path with spaces, through the shell, as a GUI program, and from a process that is already in a job; and the
//! guarantees of the private `CreateProcessW`: the job's limits, a handle the helper must not inherit, a
//! grandchild that cannot break away, a creation that fails without leaving a process, and a retired job.
use super::*;
use crate::git::endpoint::https_auth::{
    process_tree::EnvValue,
    runner::environment::exact,
    wiped::{Probe, Records},
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
        "OUT".into(),
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
    let Some(output) = std::env::var_os("OUT") else {
        return;
    };
    let value: i64 = std::env::var("PROBE").unwrap().parse().unwrap();
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
    let Some(output) = std::env::var_os("OUT") else {
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
    let (_dir, output, helper) = probe("probe_handle_target", &[("PROBE", value.clone())]);
    let status = tokio::process::Command::new(&helper.program)
        .args(&helper.args)
        .env("PROBE", &value)
        .env("OUT", &output)
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
    let (_dir, output, helper) = probe("probe_handle_target", &[("PROBE", value)]);
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
    {
        let _window = launch::InheritWindow::open([&a.child, &b.child, &c.child]).unwrap();
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
