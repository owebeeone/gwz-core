//! One serial supervised Git child under the lookup's single interaction clock.
use super::*;
use std::path::Path;

mod environment;

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
        let mut command = Command::new(self.executable);
        command
            .current_dir(environment::working_directory(self.config)?)
            .args(args)
            .env_clear()
            .envs(
                self.config
                    .environment
                    .iter()
                    .filter(|(key, _)| {
                        ![
                            "GIT_ASKPASS",
                            "SSH_ASKPASS",
                            "GIT_DIR",
                            "GIT_COMMON_DIR",
                            "GIT_WORK_TREE",
                        ]
                        .iter()
                        .any(|removed| environment::names(key, removed))
                    })
                    .map(|(key, value)| (key, value)),
            )
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(parameters) = parameters {
            for (key, _) in &self.config.environment {
                if environment::names(key, "GIT_CONFIG_COUNT")
                    || environment::starts_with(key, "GIT_CONFIG_KEY_")
                    || environment::starts_with(key, "GIT_CONFIG_VALUE_")
                {
                    command.env_remove(key);
                }
            }
            command
                .env_remove("GIT_CONFIG_SYSTEM")
                .env("GIT_CONFIG_GLOBAL", environment::NULL_CONFIG)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env(
                    "GIT_CONFIG_PARAMETERS",
                    environment::parameters_value(parameters)?,
                );
            if parameters.is_empty() {
                command.env_remove("GIT_CONFIG_PARAMETERS");
            }
        }
        self.check()?;
        let started = process_tree::spawn(&mut command).map_err(|error| match error.kind() {
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
            job.retire().await;
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

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; mod process_tests; } }
