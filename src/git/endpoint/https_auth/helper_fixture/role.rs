//! The processes that outlive a Windows fixture helper: this test binary, run as a role.
//!
//! A batch file cannot close the pipes it inherited, and `start /b` hands them on to what it starts. A descendant
//! that has none of the helper's pipes, or its standard error alone, is therefore a program that closes them
//! itself. It is not PowerShell, whose start-up on a loaded hosted runner outlasted the bounds of the rows that
//! wait for it: it is this test binary, which a helper's script starts with `GWZ_FIXTURE_ROLE` set and which runs
//! [`fixture_role`] alone (as `host_context::run_in_child` runs a test in a child). With the variable unset, as
//! in every ordinary run, [`fixture_role`] does nothing.
#![cfg(windows)]

use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::windows::io::AsRawHandle,
    path::PathBuf,
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
};

const ROLE: &str = "GWZ_FIXTURE_ROLE";
/// A descendant with none of the helper's pipes (its standard handles are `nul`).
pub(super) const DETACHED: &str = "detached";
/// A descendant that keeps the helper's standard error pipe and nothing else of it.
pub(super) const SURVIVOR: &str = "survivor";

/// The batch lines that start `role` in the background, from the helper's own script. The role's standard input
/// and output are `nul`; the detached one's standard error also, and the survivor's is the helper's pipe.
pub(super) fn start(role: &str) -> String {
    let executable = env::current_exe().unwrap();
    let module = module_path!();
    let test = format!(
        "{}::fixture_role",
        module.split_once("::").map_or(module, |(_, path)| path)
    );
    let redirects = if role == SURVIVOR {
        "<nul >nul"
    } else {
        "<nul >nul 2>nul"
    };
    format!(
        "set {ROLE}={role}\r\nstart \"\" /b \"{}\" --exact {test} --nocapture {redirects}\r\n",
        executable.display()
    )
}

/// The directories of the test process's `PATH` that hold what this binary imports from outside `System32`: the
/// embedded Python's DLL (`python3*.dll`). A role is this binary, and it does not start without them; a helper's
/// `PATH` is `System32` and these, with no PowerShell and no other tool.
pub(super) fn library_directories() -> Vec<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path)
        .filter(|directory| {
            fs::read_dir(directory).is_ok_and(|entries| {
                entries.flatten().any(|entry| {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    name.starts_with("python3") && name.ends_with(".dll")
                })
            })
        })
        .collect()
}

/// The role a helper's script started this binary as: close the pipes it inherited (all of them, or all but its
/// standard error), then append to the heartbeat file while the fixture's `alive` file exists.
#[test]
fn fixture_role() {
    let Ok(role) = env::var(ROLE) else {
        return;
    };
    let keep = (role == SURVIVOR).then(|| io::stderr().as_raw_handle());
    close_inherited_pipes(keep);
    let home =
        PathBuf::from(env::var_os("HELPER_HOME").expect("the helper's script sets HELPER_HOME"));
    let heartbeat =
        PathBuf::from(env::var_os("HEARTBEAT").expect("the helper's script sets HEARTBEAT"));
    let alive = home.join("alive");
    while alive.exists() {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&heartbeat)
        {
            let _ = file.write_all(b"x");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Closes every pipe handle of this process but `keep`. The helper passes its three pipe ends and nothing else, so
/// they are the only pipes there are; handle values are multiples of four.
fn close_inherited_pipes(keep: Option<HANDLE>) {
    for value in (4..8192usize).step_by(4) {
        let handle = value as HANDLE;
        if Some(handle) == keep {
            continue;
        }
        // SAFETY: GetFileType takes any value (an invalid one answers FILE_TYPE_UNKNOWN), and a pipe handle that
        // this process inherited is its own to close: nothing here reads or writes it.
        unsafe {
            if GetFileType(handle) == FILE_TYPE_PIPE {
                CloseHandle(handle);
            }
        }
    }
}

#[test]
fn the_fixtures_path_finds_the_libraries_this_binary_imports() {
    // The first run of the role fixtures put `System32` alone on the helper's `PATH`, and the role (this binary)
    // died at load with STATUS_DLL_NOT_FOUND, as `python313.dll` is found through `PATH` only.
    let output = std::process::Command::new(env::current_exe().unwrap())
        .arg("--list")
        .env_clear()
        .env("SystemRoot", env::var_os("SystemRoot").unwrap())
        .env("PATH", super::fixture_path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
