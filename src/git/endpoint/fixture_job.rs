//! Process control for the SSH test servers on Windows (GwzTransportWindowsParityPlan.md, step 1.1).
//!
//! The Unix fixtures stop a server with `kill`, pause its tree with `kill -STOP` and `ps`, and rely on the server
//! being a plain child. Windows has no process groups and an `sshd.exe` starts a process per connection, so a
//! test that kills only the server leaves its sessions behind. A [`ProcessJob`] is a Job Object with
//! kill-on-close: every process the server starts is a member, and the job's owner ends them all, by exact
//! membership and never by image name. [`suspend_tree`] is the `kill -STOP` of a tree.
//!
//! Test-only. Step 4.2's helper owner is the production Job Object; this one is the small test twin.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        use std::{
            io,
            mem::size_of,
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::Child,
        };
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
                    TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
                },
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject, TerminateJobObject,
                },
                Threading::{OpenThread, ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME},
            },
        };

        /// A Job Object that ends every process in it when it is dropped or terminated.
        pub(crate) struct ProcessJob(OwnedHandle);

        impl ProcessJob {
            pub(crate) fn new() -> io::Result<Self> {
                let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = unsafe {
                    SetInformationJobObject(
                        job.raw(),
                        JobObjectExtendedLimitInformation,
                        std::ptr::from_ref(&limits).cast(),
                        size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    )
                };
                if set == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(job)
            }

            /// Puts `child`, and so every process it starts from now on, in the job.
            pub(crate) fn adopt(&self, child: &Child) -> io::Result<()> {
                if unsafe { AssignProcessToJobObject(self.raw(), child.as_raw_handle()) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }

            /// Ends every process in the job now.
            pub(crate) fn terminate(&self) {
                let _ = unsafe { TerminateJobObject(self.raw(), 1) };
            }

            fn raw(&self) -> HANDLE {
                self.0.as_raw_handle()
            }
        }

        impl Drop for ProcessJob {
            fn drop(&mut self) {
                self.terminate();
            }
        }

        /// The threads of a process tree, held suspended until [`Suspended::resume`] or drop.
        pub(crate) struct Suspended {
            threads: Vec<HANDLE>,
        }

        impl Suspended {
            pub(crate) fn resume(&mut self) {
                for thread in self.threads.drain(..) {
                    unsafe {
                        ResumeThread(thread);
                        CloseHandle(thread);
                    }
                }
            }
        }

        impl Drop for Suspended {
            fn drop(&mut self) {
                self.resume();
            }
        }

        /// Suspends every thread of `root` and of every process descended from it, and returns the guard that
        /// resumes them. Like `kill -STOP`, it does not stop a process that is started after the snapshot.
        pub(crate) fn suspend_tree(root: u32) -> io::Result<Suspended> {
            let tree = descendants(root)?;
            let mut suspended = Suspended { threads: Vec::new() };
            for thread_id in threads_of(&tree)? {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
                if thread.is_null() {
                    // The thread ended since the snapshot.
                    continue;
                }
                if unsafe { SuspendThread(thread) } == u32::MAX {
                    unsafe { CloseHandle(thread) };
                    continue;
                }
                suspended.threads.push(thread);
            }
            Ok(suspended)
        }

        /// `root` and the ids of the processes it started, directly or not.
        fn descendants(root: u32) -> io::Result<Vec<u32>> {
            let snapshot = Snapshot::new(TH32CS_SNAPPROCESS)?;
            let mut entry = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..unsafe { std::mem::zeroed() }
            };
            let mut parents = Vec::new();
            let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
            while more {
                parents.push((entry.th32ProcessID, entry.th32ParentProcessID));
                more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
            }
            let mut tree = vec![root];
            let mut next = 0;
            while next < tree.len() {
                let parent = tree[next];
                next += 1;
                for (process, owner) in &parents {
                    if *owner == parent && !tree.contains(process) {
                        tree.push(*process);
                    }
                }
            }
            Ok(tree)
        }

        /// The ids of the threads that belong to the processes in `tree`.
        fn threads_of(tree: &[u32]) -> io::Result<Vec<u32>> {
            let snapshot = Snapshot::new(TH32CS_SNAPTHREAD)?;
            let mut entry = THREADENTRY32 {
                dwSize: size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut threads = Vec::new();
            let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
            while more {
                if tree.contains(&entry.th32OwnerProcessID) {
                    threads.push(entry.th32ThreadID);
                }
                more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
            }
            Ok(threads)
        }

        struct Snapshot(HANDLE);

        impl Snapshot {
            fn new(kind: u32) -> io::Result<Self> {
                let handle = unsafe { CreateToolhelp32Snapshot(kind, 0) };
                if handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self(handle))
            }
        }

        impl Drop for Snapshot {
            fn drop(&mut self) {
                unsafe { CloseHandle(self.0) };
            }
        }
    } else if #[cfg(unix)] {
        use std::{io, process::Child};

        /// The Unix twin: a server there is one process that its owner kills and reaps by its id, so the job has
        /// nothing to hold.
        pub(crate) struct ProcessJob;

        impl ProcessJob {
            pub(crate) fn new() -> io::Result<Self> {
                Ok(Self)
            }

            pub(crate) fn adopt(&self, _child: &Child) -> io::Result<()> {
                Ok(())
            }

            pub(crate) fn terminate(&self) {}
        }
    }
}

#[cfg(test)]
mod tests {
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            use super::*;
            use std::{
                process::{Command, Stdio},
                thread,
                time::{Duration, Instant},
            };

            /// A command that runs for about `seconds` and starts a grandchild to do it.
            fn slow_tree(seconds: u32) -> Child {
                Command::new("cmd")
                    .args(["/c", &format!("ping -n {} 127.0.0.1 > nul", seconds + 1)])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap()
            }

            /// Waits until the tree under `root` has a process besides `root` itself.
            fn grown(root: u32) -> Vec<u32> {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    let tree = descendants(root).unwrap();
                    if tree.len() > 1 {
                        return tree;
                    }
                    assert!(Instant::now() < deadline, "the command started no process");
                    thread::sleep(Duration::from_millis(20));
                }
            }

            fn alive(process: u32) -> bool {
                let snapshot = Snapshot::new(TH32CS_SNAPPROCESS).unwrap();
                let mut entry = PROCESSENTRY32W {
                    dwSize: size_of::<PROCESSENTRY32W>() as u32,
                    ..unsafe { std::mem::zeroed() }
                };
                let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
                while more {
                    if entry.th32ProcessID == process {
                        return true;
                    }
                    more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
                }
                false
            }

            #[test]
            fn terminating_a_job_ends_the_process_and_the_process_it_started() {
                let job = ProcessJob::new().unwrap();
                let mut child = slow_tree(60);
                job.adopt(&child).unwrap();
                let tree = grown(child.id());
                job.terminate();
                let deadline = Instant::now() + Duration::from_secs(10);
                while child.try_wait().unwrap().is_none() {
                    assert!(Instant::now() < deadline, "the job did not end its process");
                    thread::sleep(Duration::from_millis(20));
                }
                while tree.iter().any(|process| alive(*process)) {
                    assert!(Instant::now() < deadline, "a process of the tree outlived its job");
                    thread::sleep(Duration::from_millis(20));
                }
            }

            #[test]
            fn dropping_a_job_ends_its_processes() {
                let mut child = slow_tree(60);
                {
                    let job = ProcessJob::new().unwrap();
                    job.adopt(&child).unwrap();
                    grown(child.id());
                }
                let deadline = Instant::now() + Duration::from_secs(10);
                while child.try_wait().unwrap().is_none() {
                    assert!(Instant::now() < deadline, "dropping the job left its process running");
                    thread::sleep(Duration::from_millis(20));
                }
            }

            #[test]
            fn a_suspended_tree_makes_no_progress_until_it_is_resumed() {
                let job = ProcessJob::new().unwrap();
                let mut child = slow_tree(2);
                job.adopt(&child).unwrap();
                grown(child.id());
                let mut paused = suspend_tree(child.id()).unwrap();
                thread::sleep(Duration::from_millis(3_500));
                assert!(child.try_wait().unwrap().is_none(), "a suspended tree finished");
                paused.resume();
                let deadline = Instant::now() + Duration::from_secs(10);
                while child.try_wait().unwrap().is_none() {
                    assert!(Instant::now() < deadline, "a resumed tree did not finish");
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}
