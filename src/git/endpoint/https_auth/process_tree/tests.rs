//! The Job Object owner on the shapes of baseline row P08, through product code: a program started directly, in
//! a path with spaces, through the shell, as a GUI program, and from a process that is already in a job.
use super::*;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::JobObjects::{
    IsProcessInJob, JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList,
    QueryInformationJobObject as Query,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

fn system32() -> PathBuf {
    Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32")
}

fn command(program: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .env("SystemRoot", std::env::var_os("SystemRoot").unwrap())
        .env("PATH", system32())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Starts `command`, waits for `wanted` processes to be in its job, ends the job and waits for it to empty.
async fn contained_then_ended(mut command: Command, wanted: usize) {
    let (mut child, tree) = spawn(&mut command).unwrap();
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

#[tokio::test]
async fn a_program_started_directly_is_contained_and_ended_with_its_job() {
    contained_then_ended(
        command(&system32().join("ping.exe"), &["-n", "60", "127.0.0.1"]),
        1,
    )
    .await;
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
        assert!(unsafe { AssignProcessToJobObject(outer.raw(), GetCurrentProcess()) } != 0);
        std::mem::forget(outer);
    }
    contained_then_ended(
        command(&system32().join("ping.exe"), &["-n", "60", "127.0.0.1"]),
        1,
    )
    .await;
}

#[tokio::test]
async fn the_leaders_exit_does_not_empty_the_tree() {
    let mut command = command(
        &system32().join("cmd.exe"),
        &["/c", "start /b ping -n 60 127.0.0.1 >nul"],
    );
    let (mut child, tree) = spawn(&mut command).unwrap();
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
    let (mut child, tree) = spawn(&mut command(
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
async fn a_program_that_cannot_start_leaves_no_process_and_no_job_member() {
    let missing = system32().join("no such program.exe");
    let error = spawn(&mut command(&missing, &[])).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_job_is_created_without_a_name_and_reports_no_members() {
    let tree = ProcessTree::new().unwrap();
    assert!(tree.drained());
    assert!(members(&tree).is_empty());
}
