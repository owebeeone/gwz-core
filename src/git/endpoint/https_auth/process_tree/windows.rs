//! The Windows arm: the helper is created inside a Job Object and the job is the tree (see the module above).
use super::HelperCommand;
use std::{
    io,
    mem::size_of,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    process::ExitStatus,
};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject,
        },
        Threading::{GetExitCodeProcess, TerminateProcess, WaitForSingleObject},
    },
};

mod attributes;
mod launch;
mod pipe;

/// How often a wait for the helper's exit looks again.
const EXIT_POLL: std::time::Duration = std::time::Duration::from_millis(2);

/// The Job Object a helper runs in. Dropping the last handle ends whatever is still in it, unless the job was
/// retired first.
pub(in crate::git::endpoint::https_auth) struct ProcessTree {
    job: OwnedHandle,
}

/// The helper's process and its three pipes. The pipes are the standard library's child-pipe types over
/// overlapped handles, which tokio drives from its blocking pool: a dropped future leaves a thread in the call,
/// which owns its buffer, and ending the tree closes the helper's end so that the call returns.
pub(in crate::git::endpoint::https_auth) struct HelperChild {
    process: OwnedHandle,
    pub(in crate::git::endpoint::https_auth) stdin: Option<ChildStdin>,
    pub(in crate::git::endpoint::https_auth) stdout: Option<ChildStdout>,
    pub(in crate::git::endpoint::https_auth) stderr: Option<ChildStderr>,
}

/// Creates the helper inside a new job. If anything fails no process is left behind.
pub(in crate::git::endpoint::https_auth) fn spawn(
    command: &HelperCommand,
) -> io::Result<(HelperChild, ProcessTree)> {
    // The job exists before the helper does, so a failure here starts nothing.
    let tree = ProcessTree::new()?;
    let child = launch::create(command, &tree.job)?;
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
        let tree = Self {
            job: unsafe { OwnedHandle::from_raw_handle(handle) },
        };
        // A crash in a helper ends it instead of waiting for someone to dismiss an error box. No breakaway flag.
        tree.limit(
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        )?;
        Ok(tree)
    }

    fn limit(&self, flags: u32) -> io::Result<()> {
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: the pointer and length describe `limits`, which outlives the call.
        let set = unsafe {
            SetInformationJobObject(
                self.raw(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn raw(&self) -> HANDLE {
        self.job.as_raw_handle()
    }

    /// Ends every process in the job now. A process that is created while the job ends belongs to the job and
    /// ends with it; `drained` is how a caller finds out that none is left.
    pub(in crate::git::endpoint::https_auth) fn kill(&self) {
        // SAFETY: the job handle is open.
        let _ = unsafe { TerminateJobObject(self.raw(), 1) };
    }

    /// A helper that succeeded leaves what it started running: the job's kill-on-close limit is cleared, so that
    /// closing the handle that follows ends nothing. If the limit cannot be cleared the close would end the tree,
    /// so the job is left to the caller's drop either way; the clear failing on a handle this module owns is not
    /// a state this contract expects.
    pub(in crate::git::endpoint::https_auth) fn retire(&self) {
        let _ = self.limit(0);
    }

    /// Whether the job has no process in it. A job that cannot be asked is not drained.
    pub(in crate::git::endpoint::https_auth) fn drained(&self) -> bool {
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

impl HelperChild {
    fn new(
        process: OwnedHandle,
        stdin: ChildStdin,
        stdout: ChildStdout,
        stderr: ChildStderr,
    ) -> Self {
        Self {
            process,
            stdin: Some(stdin),
            stdout: Some(stdout),
            stderr: Some(stderr),
        }
    }

    /// Waits for the helper's exit, closing its input first as tokio's own child does. Dropping the future loses
    /// nothing.
    pub(in crate::git::endpoint::https_auth) async fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(EXIT_POLL).await;
        }
    }

    pub(in crate::git::endpoint::https_auth) fn try_wait(
        &mut self,
    ) -> io::Result<Option<ExitStatus>> {
        let process = self.process.as_raw_handle();
        // SAFETY: the process handle is open.
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: the process handle is open and `code` is a valid output.
                if unsafe { GetExitCodeProcess(process, &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Ends the helper's own process (the tree is ended through [`ProcessTree::kill`]).
    pub(in crate::git::endpoint::https_auth) fn start_kill(&mut self) -> io::Result<()> {
        // SAFETY: the process handle is open.
        if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
