//! The process tree of one helper, and the one owner that ends all of it (GwzTransportWindowsParityPlan.md, step 4.2;
//! GwzTransportWindowsHelperOwnerContract-DRAFT.md).
//!
//! A credential helper may start processes of its own. The helper's owner must be able to end every one of them,
//! by what the process tree is and never by a name. [`spawn`] starts the helper inside its tree and returns the
//! [`ProcessTree`] that is the capability to end it: a process group on Unix, a Job Object on Windows. Nothing
//! else in `https_auth` knows which.
//!
//! The two arms keep one interface:
//! - [`spawn`] starts the command contained, or fails and leaves no process behind;
//! - [`ProcessTree::kill`] ends every member now;
//! - [`ProcessTree::retire`] is called when the helper succeeded and its owner lets go of the tree;
//! - [`ProcessTree::drained`] reports that no member is left, which only a platform that can tell may answer `true`
//!   truthfully.
//!
//! On Windows the helper runs inside a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so closing the last
//! handle ends every process in it, and with no breakaway flag, so no process can leave it. The helper is created
//! suspended, assigned to the job and only then resumed: not one instruction of the helper, and so no descendant,
//! exists outside the job. If any step fails the helper is ended and `spawn` fails; a helper is never left running
//! uncontained.
//!
//! Handles on Windows: std creates the helper's pipe ends inheritable, and `std::process::Command::spawn` holds one
//! process-wide lock while it does, so std-spawned children never inherit one another's pipes. A handle that some
//! other code created inheritable would still be inherited; the standard library offers no handle allowlist on
//! stable (`raw_attribute` is unstable), and the contract records that.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::io;
        use tokio::process::{Child, Command};

        /// The group a helper leads. `None` once the helper's id is gone, so that no later kill can name a recycled group.
        pub(super) struct ProcessTree {
            group: Option<u32>,
        }

        /// Starts `command` as the leader of a new process group.
        pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, ProcessTree)> {
            command.process_group(0);
            let child = command.spawn()?;
            let group = child.id();
            Ok((child, ProcessTree { group }))
        }

        impl ProcessTree {
            /// Sends SIGKILL to the group.
            pub(super) fn kill(&self) {
                if let Some(group) = self.group {
                    // SAFETY: `spawn` made this child the leader of its own group. The captured positive PID names that
                    // group, never the caller's process group.
                    unsafe {
                        libc::kill(-(group as libc::pid_t), libc::SIGKILL);
                    }
                }
            }

            /// A helper that succeeded leaves its group as it is: the owner forgets it, as it always has on Unix.
            pub(super) fn retire(&self) {}

            /// A process group has no handle to wait on, so Unix cleanup is confirmed by the leader's reap alone.
            pub(super) fn drained(&self) -> bool {
                true
            }
        }
    } else if #[cfg(windows)] {
        use std::{
            io,
            mem::size_of,
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        };
        use tokio::process::{Child, Command};
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
                },
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW,
                    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
                    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
                },
                Threading::{
                    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
                },
            },
        };

        /// The Job Object a helper runs in. Dropping the last handle ends whatever is still in it.
        pub(super) struct ProcessTree {
            job: OwnedHandle,
        }

        /// Starts `command` suspended, puts it in a new job and resumes it. The command gets no console window of its own.
        pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, ProcessTree)> {
            // The job exists before the helper does, so a failure here starts nothing.
            let tree = ProcessTree::new()?;
            command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
            let mut child = command.spawn()?;
            let contained = tree.assign(&child).and_then(|()| resume(child.id()));
            if let Err(error) = contained {
                tree.kill();
                let _ = child.start_kill();
                return Err(error);
            }
            Ok((child, tree))
        }

        impl ProcessTree {
            fn new() -> io::Result<Self> {
                // SAFETY: no attributes and no name; the result is checked and owned at once.
                let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: `handle` is a new job handle that nothing else owns.
                let job = Self {
                    job: unsafe { OwnedHandle::from_raw_handle(handle) },
                };
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                // A crash in a helper ends it instead of waiting for someone to dismiss an error box.
                limits.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
                // SAFETY: the pointer and length describe `limits`, which outlives the call.
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

            fn assign(&self, child: &Child) -> io::Result<()> {
                let process = child
                    .raw_handle()
                    .ok_or_else(|| io::Error::other("the helper has no process handle"))?;
                // SAFETY: both handles are open and owned by this thread's caller for the call.
                if unsafe { AssignProcessToJobObject(self.raw(), process) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }

            fn raw(&self) -> HANDLE {
                self.job.as_raw_handle()
            }

            /// Ends every process in the job now. A process that is created while the job ends belongs to the job and
            /// ends with it; `drained` is how a caller finds out that none is left.
            pub(super) fn kill(&self) {
                // SAFETY: the job handle is open.
                let _ = unsafe { TerminateJobObject(self.raw(), 1) };
            }

            /// A helper that succeeded has its tree ended too: a Job Object holds the tree until it is empty, and what a
            /// successful helper left behind is not wanted.
            pub(super) fn retire(&self) {
                self.kill();
            }

            /// Whether the job has no process in it. A job that cannot be asked is not drained.
            pub(super) fn drained(&self) -> bool {
                let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                // SAFETY: the pointer and length describe `accounting`, which outlives the call.
                let queried = unsafe {
                    QueryInformationJobObject(
                        self.raw(),
                        JobObjectBasicAccountingInformation,
                        std::ptr::from_mut(&mut accounting).cast(),
                        size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                        std::ptr::null_mut(),
                    )
                };
                queried != 0 && accounting.ActiveProcesses == 0
            }
        }

        /// Resumes the one thread of the suspended process `pid`.
        fn resume(pid: Option<u32>) -> io::Result<()> {
            let pid = pid.ok_or_else(|| io::Error::other("the helper has no process id"))?;
            // SAFETY: a thread snapshot takes no other argument; the result is checked and owned at once.
            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let snapshot = Snapshot(snapshot);
            let mut entry = THREADENTRY32 {
                dwSize: size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut resumed = 0;
            // SAFETY: `entry.dwSize` is set and the snapshot is open.
            let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    // SAFETY: the thread id came from the snapshot; the handle is checked and closed below.
                    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    if thread.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    // SAFETY: `thread` is open with suspend and resume access.
                    let previous = unsafe { ResumeThread(thread) };
                    let failed = previous == u32::MAX;
                    let error = io::Error::last_os_error();
                    // SAFETY: `thread` is open and owned here.
                    unsafe { CloseHandle(thread) };
                    if failed {
                        return Err(error);
                    }
                    resumed += 1;
                }
                // SAFETY: as for `Thread32First`.
                more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
            }
            if resumed == 0 {
                return Err(io::Error::other("the helper's thread was not found"));
            }
            Ok(())
        }

        struct Snapshot(HANDLE);

        impl Drop for Snapshot {
            fn drop(&mut self) {
                // SAFETY: the snapshot handle is open and owned here.
                unsafe { CloseHandle(self.0) };
            }
        }

        cfg_if::cfg_if! {
            if #[cfg(test)] {
                mod tests;
            }
        }
    }
}
