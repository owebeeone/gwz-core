//! One serial supervised Git child under the lookup's single interaction clock.
use super::*;
use std::path::Path;

pub(super) mod environment;

pub(super) struct Runner<'a> {
    pub(super) owner: &'a AuthOwner,
    pub(super) config: &'a Config,
    pub(super) executable: &'a Path,
    pub(super) permits: Arc<owner::AdmissionPermits>,
    pub(super) cancelled: &'a CancellationToken,
    pub(super) deadline: Instant,
    pub(super) setup: Option<&'a super::super::ssh_setup_context::SetupContext>,
}
struct ChildRequest<'a> {
    args: &'a [&'a str],
    input: &'a [u8],
    parameters: Option<&'a [u8]>,
    limit: usize,
    preparing: bool,
}

impl Runner<'_> {
    pub(super) fn check(&self) -> Result<(), AuthError> {
        self.check_with_now(Instant::now)
    }

    fn check_with_now(&self, now: impl FnOnce() -> Instant) -> Result<(), AuthError> {
        if self.setup.is_some_and(|setup| setup.check().is_err()) {
            return Err(AuthError::Cancelled);
        }
        if self.cancelled.is_cancelled() || self.owner.inner.cancelled.is_cancelled() {
            return Err(AuthError::Cancelled);
        }
        if now() >= self.deadline {
            return Err(AuthError::Timeout);
        }
        Ok(())
    }

    pub(super) async fn run(
        &self,
        args: &[&str],
        input: &[u8],
        parameters: Option<&[u8]>,
        limit: usize,
        preparing: bool,
    ) -> Result<SecretBuffer, AuthError> {
        self.run_finished(
            ChildRequest {
                args,
                input,
                parameters,
                limit,
                preparing,
            },
            Ok,
        )
        .await
    }

    pub(super) async fn run_secret(
        &self,
        input: &[u8],
        parameters: &[u8],
    ) -> Result<Secret, AuthError> {
        self.run_finished(
            ChildRequest {
                args: &["-c", "core.askPass=", "credential", "fill"],
                input,
                parameters: Some(parameters),
                limit: OUTPUT_LIMIT,
                preparing: false,
            },
            |output| self.parse_answer(&output.0, parse_secret),
        )
        .await
    }

    async fn run_finished<T>(
        &self,
        request: ChildRequest<'_>,
        finish: impl FnOnce(SecretBuffer) -> Result<T, AuthError>,
    ) -> Result<T, AuthError> {
        let ChildRequest {
            args,
            input,
            parameters,
            limit,
            preparing,
        } = request;
        self.check()?;
        let directory = environment::working_directory()?;
        let variables = environment::helper_environment(
            &self.config.environment,
            &directory,
            parameters,
            wiped::Probe::inert(),
        )?;
        let command = process_tree::HelperCommand {
            program: self.executable.to_path_buf(),
            args: args.iter().map(OsString::from).collect(),
            directory,
            environment: variables.into_pairs(),
        };
        // The host's spawn gate, for the launch alone: nothing else of the host spawns while the helper's pipe ends
        // are inheritable, and the wait that follows is not under it (WH2 contract C9).
        let hold = self.hold_spawn_gate().await?;
        let spawned = process_tree::spawn(&command, &hold);
        drop(hold);
        // The child has its own copy of the environment (or the spawn failed): the command's owners wipe theirs
        // now, not when the helper ends.
        drop(command);
        let started = spawned.map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => AuthError::MissingExecutable,
            io::ErrorKind::ArgumentListTooLong if parameters.is_some() => {
                AuthError::ConfigurationRefused
            }
            _ => AuthError::SpawnFailed,
        })?;
        let mut job = HelperJob::new(started, self.permits.clone(), self.owner.clone());
        let stdin = job.child_mut().stdin.take().ok_or(AuthError::SpawnFailed)?;
        let stdout = job
            .child_mut()
            .stdout
            .take()
            .ok_or(AuthError::SpawnFailed)?;
        let stderr = job
            .child_mut()
            .stderr
            .take()
            .ok_or(AuthError::SpawnFailed)?;
        let overflow = CancellationToken::new();
        let read = bounded_output(stdout, limit, overflow.clone());
        let mut drain = Box::pin(pipes::discard_stderr(stderr));
        let mut completed = Box::pin(async {
            let (write, output, status) = tokio::join!(
                pipes::write_request(stdin, input),
                read,
                job.child_mut().wait()
            );
            write?;
            let output = output?;
            if !status.map_err(|_| AuthError::Io)?.success() {
                return Err(AuthError::HelperRejected);
            }
            Ok(output)
        });
        let mut work = Box::pin(async {
            tokio::select! {
                result = &mut completed => result,
                result = &mut drain => { result?; (&mut completed).await }
            }
        });
        let result = tokio::select! {
            result = &mut work => result,
            _ = self.cancelled.cancelled() => Err(AuthError::Cancelled),
            _ = self.owner.inner.cancelled.cancelled() => Err(AuthError::Cancelled),
            _ = sleep_until(self.deadline) => Err(AuthError::Timeout),
            _ = overflow.cancelled() => Err(AuthError::OutputTooLarge),
        };
        drop(work);
        drop(completed);
        drop(drain);
        // Child completion is not success admission. Parse the final answer
        // and recheck the unchanged clock/cancellation while the job still
        // owns its process group and both admission permits.
        let result = self.admit_child_output(result).and_then(finish);
        self.finish_job(&mut job, result, |result| self.admit_child_output(result))
            .await
            .map_err(|error| {
                if preparing
                    && matches!(
                        error,
                        AuthError::HelperRejected
                            | AuthError::OutputTooLarge
                            | AuthError::Pipe(_)
                            | AuthError::Io
                    )
                {
                    AuthError::ConfigurationRefused
                } else {
                    error
                }
            })
    }

    /// Holds the host's spawn gate, waiting for whoever has it. The wait is spent from the lookup's own clock and
    /// ends at its cancellation, like every other wait of the lookup, and nothing is started for it: a lookup that
    /// gives up here has not launched.
    async fn hold_spawn_gate(&self) -> Result<SpawnHold, AuthError> {
        let gate = self.owner.inner.helper_slots.gate();
        loop {
            self.check()?;
            if let Some(hold) = gate.try_hold() {
                return Ok(hold);
            }
            sleep(GATE_POLL).await;
        }
    }

    async fn finish_job<T>(
        &self,
        job: &mut HelperJob,
        result: Result<T, AuthError>,
        admit: impl FnOnce(Result<T, AuthError>) -> Result<T, AuthError>,
    ) -> Result<T, AuthError> {
        // This is the last decision that can turn successful output into a
        // refusal. Its cleanup capability must still be owned at that point.
        let result = admit(result);
        if result.is_ok() {
            job.retire();
        } else {
            job.terminate().await?;
        }
        // Success is already admitted: retirement cannot invent a new refusal.
        result
    }

    fn admit_child_output<T>(&self, result: Result<T, AuthError>) -> Result<T, AuthError> {
        if result.is_ok() {
            self.check()?;
        }
        result
    }

    pub(super) fn parse_answer(
        &self,
        output: &[u8],
        parse: impl FnOnce(&[u8]) -> Result<Secret, AuthError>,
    ) -> Result<Secret, AuthError> {
        self.check()?;
        let answer = parse(output);
        self.check()?;
        answer
    }
}

async fn bounded_output<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
    overflow: CancellationToken,
) -> Result<SecretBuffer, AuthError> {
    let mut output = SecretBuffer(vec![0; limit]);
    let mut used = 0;
    while used < limit {
        let n = reader
            .read(&mut output.0[used..])
            .await
            .map_err(|e| AuthError::Pipe(e.kind()))?;
        if n == 0 {
            output.0.truncate(used);
            return Ok(output);
        }
        used += n;
    }
    let mut extra = SecretBuffer(vec![0; 1]);
    if reader
        .read(&mut extra.0)
        .await
        .map_err(|e| AuthError::Pipe(e.kind()))?
        != 0
    {
        overflow.cancel();
        return Err(AuthError::OutputTooLarge);
    }
    Ok(output)
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod gate_tests;
        mod process_tests;
        mod tests;
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                mod ceiling_tests;
                mod windows_tests;
            }
        }
    }
}
