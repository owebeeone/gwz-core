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
//! - [`ProcessTree::retire`] is called when the helper succeeded and its owner lets go of the tree: what the helper
//!   left running is left running, as on Unix, under Git and in 1.0.17;
//! - [`ProcessTree::drained`] reports that no member is left, which only a platform that can tell may answer `true`
//!   truthfully.
//!
//! On Windows the helper is created inside a Job Object (`PROC_THREAD_ATTRIBUTE_JOB_LIST`), so not one instruction
//! of it runs outside the job, and with only its three pipe ends inherited (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`),
//! so it and its descendants hold no other handle of gwz or of the process that embeds it. The job has
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so a timeout, a cancel, a dropped lookup and gwz's own exit end every
//! process in it; a success clears that limit before the job is closed. Containment covers the processes that
//! members create through `CreateProcess`; a process that a broker creates on a member's behalf (WMI, Task
//! Scheduler, COM activation, a packaged-app launch) is outside any job, as a `setsid` descendant is outside a
//! Unix process group.
use std::{ffi::OsString, path::PathBuf};

/// What one helper is started with. The environment is the child's complete environment.
pub(super) struct HelperCommand {
    pub(super) program: PathBuf,
    pub(super) args: Vec<OsString>,
    pub(super) directory: PathBuf,
    pub(super) environment: Vec<(OsString, OsString)>,
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::io;
        use tokio::process::{Child, Command};

        /// The helper's child process: on Unix, tokio's own.
        pub(super) type HelperChild = Child;

        /// The group a helper leads. `None` when the helper's id was already gone at spawn.
        pub(super) struct ProcessTree {
            group: Option<u32>,
        }

        /// Starts `command` as the leader of a new process group, with all three standard streams piped.
        pub(super) fn spawn(command: &HelperCommand) -> io::Result<(HelperChild, ProcessTree)> {
            let mut process = Command::new(&command.program);
            process
                .current_dir(&command.directory)
                .args(&command.args)
                .env_clear()
                .envs(command.environment.iter().map(|(key, value)| (key, value)))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .process_group(0);
            let child = process.spawn()?;
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

            /// A helper that succeeded leaves its group as it is: the owner forgets it.
            pub(super) fn retire(&self) {}

            /// A process group has no handle to wait on, so Unix cleanup is confirmed by the leader's reap alone.
            pub(super) fn drained(&self) -> bool {
                true
            }
        }
    } else if #[cfg(windows)] {
        mod windows;
        pub(super) use windows::{HelperChild, ProcessTree, spawn};
    }
}
