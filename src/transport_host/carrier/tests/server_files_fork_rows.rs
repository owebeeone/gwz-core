//! The server's lock files and a child forked by another thread (step 2.3f, rounds 3 and 4, finding F18).
//!
//! The locks are `flock` locks, which belong to the open file description, not to the descriptor. A child forked
//! (or `vfork`ed, or `posix_spawn`ed) by any thread of the process holds a copy of every descriptor that is open at
//! that moment until it executes, `O_CLOEXEC` or not, so closing a lock's file does not release it while such a
//! child exists. The server's clean exit therefore unlocks each lock explicitly before it closes the file (round 4),
//! and a row below pins that with a child held inside its fork-to-exec window. A release that only closes (a
//! `Claim` dropped before it bound) still keeps the lock for that window, and a row pins that too, as the documented
//! remainder. A listening socket a row closes without unlinking it (`dead_socket`) is likewise kept answering by a
//! copy in a forked child. Rows that drop a claim or close such a socket and then use it again in the same process
//! would fail whenever another test thread forked in between, so they run through [`alone`], in a process of their
//! own that runs only that row; the two rows below run through it as well, so that the child they hold open exists
//! in no process but their own.
/// Runs `body` in a process of its own: this test binary run again on exactly the calling row (`--exact`, one test
/// thread), so no other test of the suite is there to fork while the row holds a lock. The row's path is that of the
/// function `here` defined inside the row, which the macro passes.
///
/// On Unix only; elsewhere the body runs here (a Windows handle is not inherited by a child unless it is made
/// inheritable, and the lock files' are not).
macro_rules! alone {
    ($body:expr) => {{
        fn here() {}
        super::server_files_fork_rows::run(std::any::type_name_of_val(&here), $body)
    }};
}
pub(super) use alone;

/// The row at `here` (a `type_name`: crate, module path, row, `here`) run alone; see [`alone`].
pub(super) fn run(here: &str, body: impl FnOnce()) {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::process::{Command, Stdio};

            /// Set in the child process to the row's path: the child runs the row's body, the parent does not.
            const ALONE_MARK: &str = "GWZ_SERVER_FILES_ROW_ALONE";
            /// What the child prints once the body has run to its end, so that a path naming no test cannot pass.
            const RAN: &str = "server_files row ran alone";

            let row = here.strip_suffix("::here").unwrap_or(here);
            let row = row.split_once("::").map_or(row, |(_, relative)| relative);
            if std::env::var(ALONE_MARK).as_deref() == Ok(row) {
                body();
                println!("{RAN}");
                return;
            }
            let output = Command::new(std::env::current_exe().expect("this test binary"))
                .args(["--exact", row, "--nocapture", "--test-threads=1"])
                .env(ALONE_MARK, row)
                .stdin(Stdio::null())
                .output()
                .expect("run the row alone");
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.status.success(), "{row} failed alone:\n{text}");
            assert!(text.contains(RAN), "{row} did not run alone:\n{text}");
        } else {
            let _ = here;
            body();
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::server_files_rows::fixture;
        use crate::transport_host::carrier::local::server_files::{FilesRefusal, Record};
        use std::{
            io::{self, PipeWriter, Read, Write},
            os::fd::AsRawFd,
            os::unix::process::CommandExt,
            process::{Command, ExitStatus, Stdio},
            thread::JoinHandle,
        };

        /// A child of this process, forked by another thread and held inside its fork-to-exec window by two pipes (it
        /// says it is there; it leaves when told), so nothing waits on time. It holds a copy of every descriptor that
        /// was open when it was forked.
        ///
        /// Whatever happens to the row, the child is let go: [`Self::finish`] does it on the way out, and the drop does
        /// it on an unwind (a failing row reports its message and leaves no process). The child also closes its own copy
        /// of the release pipe's write end before it blocks, so the parent's end closing is enough for it to see the
        /// end of the pipe and give up.
        struct ChildInWindow {
            leave: PipeWriter,
            forker: Option<JoinHandle<io::Result<ExitStatus>>>,
        }

        impl ChildInWindow {
            /// Forks the child and returns once it is in the window: it exists, was forked after everything the row
            /// holds open, and has not executed.
            fn fork() -> Self {
                let (mut inside, window_open) = std::io::pipe().expect("a pipe");
                let (window_wait, leave) = std::io::pipe().expect("a pipe");
                let leave_fd = leave.as_raw_fd();
                let forker = std::thread::spawn(move || {
                    let mut command = Command::new("true");
                    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                    // SAFETY: the hook only closes, writes and reads descriptors (async-signal-safe calls) before the exec.
                    unsafe {
                        command.pre_exec(move || {
                            // This child's own copy of the release pipe's write end, so that it never keeps it open.
                            let _ = libc::close(leave_fd);
                            (&window_open).write_all(&[1])?;
                            (&window_wait).read_exact(&mut [0])
                        });
                    }
                    command.status()
                });
                // The guard exists before anything can fail, so an unwind from here on still lets the child go.
                let child = ChildInWindow { leave, forker: Some(forker) };
                let mut byte = [0];
                inside.read_exact(&mut byte).expect("the child is in the window");
                child
            }

            /// Lets the child go, waits for it to execute `true` and exit, and returns how it ended.
            fn finish(mut self) -> ExitStatus {
                self.leave.write_all(&[1]).expect("let the child go");
                let forker = self.forker.take().expect("the forker is joined once");
                forker.join().expect("the forker did not panic").expect("the child ran")
            }
        }

        impl Drop for ChildInWindow {
            fn drop(&mut self) {
                if let Some(forker) = self.forker.take() {
                    let _ = self.leave.write_all(&[1]);
                    let _ = forker.join();
                }
            }
        }

        /// Finding F18, the fix: the clean exit unlocks both locks before it closes their files, so a child that another
        /// thread forked while they were held, and that is still in its fork-to-exec window, does not keep them. The next
        /// start succeeds at once and finds no record; the child is then let go. (Without the explicit unlock the next
        /// start is refused as `Held` with no record until the child has executed: the round-3 form of this row.)
        #[test]
        fn a_clean_exit_releases_both_locks_while_a_forked_child_still_holds_a_copy() {
            alone!(|| {
                let fixture = fixture();
                let server = fixture.start();
                let child = ChildInWindow::fork();
                server.close();
                let next = fixture
                    .claim()
                    .expect("the clean exit unlocked both locks, whatever copy a forked child holds");
                assert_eq!(next.previous(), &Record::None);
                drop(next);
                assert!(child.finish().success());
            });
        }

        /// What the explicit unlock does not cover, as the docs state it: a claim dropped before it bound releases by
        /// closing only, so a child forked while it was held keeps the lock until it executes. The next start is
        /// refused as `Held` with no record, and succeeds once the child has executed (its `CLOEXEC` copies closed).
        #[test]
        fn a_claim_dropped_before_it_bound_keeps_the_lock_until_a_forked_child_executes() {
            alone!(|| {
                let fixture = fixture();
                let claim = fixture.claim().expect("a first claim");
                let child = ChildInWindow::fork();
                drop(claim);
                match fixture.claim() {
                    Err(FilesRefusal::Held { lock, record }) => {
                        assert_eq!(lock, fixture.identity_lock());
                        assert_eq!(record, Record::None);
                    }
                    Err(other) => panic!("the forked child should hold the lock: {other}"),
                    Ok(_) => panic!("the lock was released while a forked child held its descriptor"),
                }
                assert!(child.finish().success());
                let next = fixture.claim().expect("the child has executed, so the lock is free");
                assert_eq!(next.previous(), &Record::None);
            });
        }
    }
}
