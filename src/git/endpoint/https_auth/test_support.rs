//! Existing credential owner and parser regression coverage.
use super::*;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl AuthOwner {
            /// The helper lookups in flight.
            pub(crate) fn active_count(&self) -> usize {
                self.inner.active.load(Ordering::Acquire)
            }
        }
        mod tests {
            use super::*;
            use super::helper_fixture::{Behavior, Fixture};
            use crate::git::endpoint::helper_script::git_fixture;
            use std::fs;
            use tempfile::tempdir;

            fn helper(script: &str) -> (tempfile::TempDir, Config) {
                let directory = tempdir().unwrap();
                let config = git_fixture(directory.path(), &format!("{script}\n"));
                (directory, config)
            }

            /// A helper that never answers, started as the product starts one, as a child an owner can retain,
            /// holding one of `slots`.
            fn retained_helper(behavior: Behavior, slots: &Arc<Semaphore>) -> (Fixture, PendingChild) {
                let fixture = Fixture::new(behavior);
                let (child, tree) = super::super::process_tree::spawn_for_test(&super::super::process_tree::HelperCommand {
                    program: fixture.config.executable.clone(),
                    args: fixture.argv().iter().map(Into::into).collect(),
                    directory: fixture.directory().to_owned(),
                    environment: super::runner::environment::exact(&fixture.config.environment),
                })
                .unwrap();
                let pending = PendingChild {
                    child,
                    tree: Some(tree),
                    _permits: Arc::new(super::super::owner::AdmissionPermits {
                        _endpoint_slot: None,
                        _helper_slot: slots.clone().try_acquire_owned().unwrap(),
                    }),
                };
                (fixture, pending)
            }

            #[test]
            fn secret_is_redacted_and_builds_basic_header() {
                let secret = Secret {
                    username: b"alice".to_vec(),
                    password: b"secret".to_vec(),
                };
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
                assert_eq!(format!("{secret:?}"), "Secret(REDACTED)");
            }

            #[test]
            fn gh_attribute_line_without_blank_line_is_a_credential() {
                let secret = parse_secret(b"protocol=https\nhost=github.com\nusername=alice\npassword=secret\n").unwrap();
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
            }

            #[test]
            fn malformed_or_ambiguous_helper_output_is_rejected() {
                for output in [
                    b"username=alice\npassword=secret".as_slice(),
                    b"username=alice\nusername=bob\npassword=secret\n\n",
                    b"username=al:ice\npassword=secret\n\n",
                ] {
                    assert!(parse_secret(output).is_err());
                }
            }

            #[tokio::test]
            async fn lookup_accepts_gh_output_without_a_blank_line() {
                let (directory, config) = helper("printf 'protocol=https\\nhost=github.com\\nusername=alice\\npassword=secret\\n'");
                let destination = Destination::parse("https://github.com/owner/repo.git").unwrap();
                let secret = lookup_owned(
                    &AuthOwner::new(HelperSlots::new()),
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(60),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
                drop(directory);
            }

            #[tokio::test]
            async fn lookup_uses_bounded_direct_helper_and_returns_secret() {
                let (directory, config) = helper("printf 'username=alice\\npassword=secret\\n\\n'");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let secret = lookup_owned(
                    &AuthOwner::new(HelperSlots::new()),
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(60),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
                drop(directory);
            }

            #[tokio::test]
            async fn a_helper_that_exits_without_reading_the_request_does_not_fail_the_write() {
                // Git ignores SIGPIPE while it writes to a credential helper, so a
                // helper may answer without reading its input. The helper's exit
                // status and output decide the lookup, not the closed input.
                let (input, helper) = tokio::io::duplex(64);
                drop(helper);
                assert!(matches!(write_request(input, b"protocol=https\n\n").await, Ok(())));
            }

            #[tokio::test]
            async fn any_other_request_write_failure_is_an_io_failure() {
                let input = FailingInput(io::ErrorKind::PermissionDenied);
                assert!(matches!(write_request(input, b"protocol=https\n\n").await, Err(AuthError::Pipe(io::ErrorKind::PermissionDenied))));
            }

            /// Input whose every write fails with one error kind.
            struct FailingInput(io::ErrorKind);

            impl AsyncWrite for FailingInput {
                fn poll_write(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                    _: &[u8],
                ) -> std::task::Poll<io::Result<usize>> {
                    std::task::Poll::Ready(Err(self.0.into()))
                }

                fn poll_flush(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }

                fn poll_shutdown(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }
            }

            #[tokio::test]
            async fn pre_cancelled_lookup_does_not_start_helper() {
                let (directory, config) = helper("sleep 5");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let cancelled = CancellationToken::new();
                cancelled.cancel();
                let result = lookup_owned(
                    &AuthOwner::new(HelperSlots::new()),
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(60),
                    &cancelled,
                )
                .await;
                assert!(matches!(result, Err(AuthError::Cancelled) | Err(AuthError::CleanupPending)));
                drop(directory);
            }

            #[tokio::test]
            async fn aborting_reap_preserves_child_and_permit_ownership() {
                let owner = AuthOwner::new(HelperSlots::new());
                let slots = owner.inner.helper_slots.0.clone();
                let before = slots.available_permits();
                let (_fixture, pending) = retained_helper(Behavior::Hangs, &slots);
                owner.retain_pending(pending);
                let reaper = owner.clone();
                let task = tokio::spawn(async move { reaper.reap_pending(Instant::now()+Duration::from_secs(30)).await });
                while !owner.inner.pending.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
                task.abort(); let _ = task.await;
                let retained = owner.pending_cleanup_count();
                let permits = slots.available_permits();
                let _ = owner.reap_pending(Instant::now()+Duration::from_secs(30)).await;
                assert_eq!(retained, 1, "aborting reap lost its owned child");
                assert_eq!(permits, before-1, "permit released before actual reap");
                assert_eq!(owner.pending_cleanup_count(), 0);
            }

            /// An owner dropped while it retains a killed helper drops the
            /// child with it, and the host gets the helper's slot back; no
            /// process-wide registry keeps either.
            #[tokio::test]
            async fn a_dropped_owner_releases_its_retained_helpers_slots() {
                let host = HelperSlots::new();
                let owner = AuthOwner::new(host.clone());
                let slots = host.0.clone();
                let before = slots.available_permits();
                let (_fixture, mut pending) = retained_helper(Behavior::Hangs, &slots);
                // A helper is killed before its owner retains it.
                pending.child.start_kill().unwrap();
                owner.retain_pending(pending);
                assert_eq!(slots.available_permits(), before - 1);
                drop(owner);
                assert_eq!(slots.available_permits(), before, "the dropped owner kept its helper's slot");
            }

            #[tokio::test]
            async fn reap_reports_children_arriving_while_it_waits() {
                let owner = AuthOwner::new(HelperSlots::new());
                let slots = owner.inner.helper_slots.0.clone();
                let (_first, pending) = retained_helper(Behavior::EndsSoon, &slots);
                owner.retain_pending(pending);
                let reaper = owner.clone();
                let task = tokio::spawn(async move { reaper.reap_pending(Instant::now()+Duration::from_secs(10)).await });
                while !owner.inner.pending.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
                let (_second, pending) = retained_helper(Behavior::Hangs, &slots);
                owner.retain_pending(pending);
                let reported = task.await.unwrap();
                let actual = owner.pending_cleanup_count();
                for pending in owner.inner.pending.lock().unwrap().iter_mut() { let _ = pending.child.start_kill(); }
                let _ = owner.reap_pending(Instant::now()+Duration::from_secs(30)).await;
                assert_eq!(reported, actual, "reap omitted a child transferred during its wait");
                assert_eq!(reported, 1);
            }

            #[tokio::test]
            async fn owner_cancellation_after_start_reclaims_helper_admission() {
                let host = HelperSlots::new();
                let owner = AuthOwner::new(host.clone());
                let (directory, tasks) = occupy_eight_slots(&owner).await;
                assert_eq!(owner.active_count(), 8);
                owner.cancel();
                for task in tasks {
                    let result = task.await.unwrap();
                    assert!(
                        matches!(result, Err(AuthError::Cancelled) | Err(AuthError::CleanupPending)),
                        "unexpected helper result: {result:?}"
                    );
                }
                assert_eq!(owner.active_count(), 0);
                let retained = owner
                    .reap_pending(Instant::now() + Duration::from_secs(30))
                    .await;
                assert_eq!(retained, 0);
                assert_eq!(owner.pending_cleanup_count(), 0);
                // The host's eight slots are back: another of its owners is admitted.
                assert_eq!(
                    quick_lookup(&AuthOwner::new(host)).await,
                    Ok("Basic YWxpY2U6c2VjcmV0".to_owned())
                );
                drop(directory);
            }

            /// Starts eight helpers on `owner` that stay live until cancelled.
            async fn occupy_eight_slots(
                owner: &AuthOwner,
            ) -> (tempfile::TempDir, Vec<tokio::task::JoinHandle<Result<Secret, AuthError>>>) {
                let directory = tempdir().unwrap();
                let started = directory.path().join("started");
                let mut config = git_fixture(
                    directory.path(),
                    "printf '%s\\n' \"$$\" >> \"$GWZ_HELPER_STARTED\"\nexec /bin/sleep 3600\n",
                );
                config.environment.push(("GWZ_HELPER_STARTED".into(), started.as_os_str().into()));
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let mut tasks = Vec::new();
                for _ in 0..8 {
                    let owner = owner.clone();
                    let config = config.clone();
                    let destination = destination.clone();
                    tasks.push(tokio::spawn(async move {
                        lookup_owned(
                            &owner,
                            &config,
                            &destination,
                            Instant::now() + Duration::from_secs(60),
                            &CancellationToken::new(),
                        )
                        .await
                    }));
                }
                // A readiness wait, not a speed claim: eight MSYS shells can take more than 2 s to start on a loaded
                // Windows runner (CI run 38793264504).
                let barrier = Instant::now() + Duration::from_secs(20);
                while fs::read_to_string(&started).map_or(0, |started| started.lines().count()) < 8 {
                    assert!(Instant::now() < barrier, "helper start barrier timed out");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                (directory, tasks)
            }

            async fn release(owner: &AuthOwner, tasks: Vec<tokio::task::JoinHandle<Result<Secret, AuthError>>>) {
                owner.cancel();
                for task in tasks {
                    let _ = task.await.unwrap();
                }
                assert_eq!(owner.reap_pending(Instant::now() + Duration::from_secs(30)).await, 0);
            }

            async fn quick_lookup(owner: &AuthOwner) -> Result<String, AuthError> {
                quick_lookup_within(owner, Duration::from_secs(30)).await
            }

            /// How long a lookup that must find every slot held waits for one. Its answer is AllocationTimeout however
            /// long it waits; a free slot would be taken at once, and its helper answers, so the bound only has to
            /// outlast scheduling on a loaded host, not the 30 s a lookup that must succeed is given.
            const HELD_SLOTS_WAIT: Duration = Duration::from_secs(5);

            async fn quick_lookup_within(owner: &AuthOwner, budget: Duration) -> Result<String, AuthError> {
                let (directory, config) = helper("printf 'username=alice\\npassword=secret\\n\\n'");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let result = lookup_owned(
                    owner,
                    &config,
                    &destination,
                    Instant::now() + budget,
                    &CancellationToken::new(),
                )
                .await
                .map(|secret| secret.header().to_string());
                drop(directory);
                result
            }

            #[tokio::test]
            async fn one_hosts_live_helpers_never_exhaust_another_hosts_admission() {
                let busy = AuthOwner::new(HelperSlots::new());
                let (directory, tasks) = occupy_eight_slots(&busy).await;
                let other = quick_lookup(&AuthOwner::new(HelperSlots::new())).await;
                release(&busy, tasks).await;
                assert_eq!(other, Ok("Basic YWxpY2U6c2VjcmV0".to_owned()));
                drop(directory);
            }

            #[tokio::test]
            async fn endpoints_of_one_host_share_its_eight_helper_slots() {
                let host = HelperSlots::new();
                let busy = AuthOwner::new(host.clone());
                let sibling = AuthOwner::new(host);
                let (directory, tasks) = occupy_eight_slots(&busy).await;
                let ninth = quick_lookup_within(&sibling, HELD_SLOTS_WAIT).await;
                release(&busy, tasks).await;
                let after = quick_lookup(&sibling).await;
                assert_eq!(ninth, Err(AuthError::AllocationTimeout), "a ninth helper awaits the same host slots");
                assert_eq!(after, Ok("Basic YWxpY2U6c2VjcmV0".to_owned()));
                drop(directory);
            }

            #[tokio::test]
            async fn abort_after_helper_start_retains_permit_until_owner_reap() {
                let directory = tempdir().unwrap();
                let started = directory.path().join("started");
                let mut config = git_fixture(
                    directory.path(),
                    "printf started > \"$GWZ_HELPER_STARTED\"\nexec /bin/sleep 3600\n",
                );
                config.environment.push(("GWZ_HELPER_STARTED".into(), started.as_os_str().into()));
                let owner = AuthOwner::new(HelperSlots::new());
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let available_before = owner.inner.helper_slots.0.available_permits();
                assert!(available_before > 0, "helper admission unexpectedly exhausted");
                let task_owner = owner.clone();
                let task_config = config.clone();
                let task_destination = destination.clone();
                let task = tokio::spawn(async move {
                    lookup_owned(
                        &task_owner,
                        &task_config,
                        &task_destination,
                        Instant::now() + Duration::from_secs(60),
                        &CancellationToken::new(),
                    )
                    .await
                });
                // A readiness wait, not a speed claim: eight MSYS shells can take more than 2 s to start on a loaded
                // Windows runner (CI run 38793264504).
                let barrier = Instant::now() + Duration::from_secs(20);
                while !started.exists() {
                    assert!(Instant::now() < barrier, "helper start barrier timed out");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert_eq!(owner.active_count(), 0);
                assert_eq!(owner.pending_cleanup_count(), 1);
                assert_eq!(owner.inner.helper_slots.0.available_permits(), available_before - 1);

                assert_eq!(
                    owner.reap_pending(Instant::now()).await,
                    1,
                    "zero-time reap must retain a live aborted child"
                );
                assert_eq!(owner.pending_cleanup_count(), 1);
                assert_eq!(owner.inner.helper_slots.0.available_permits(), available_before - 1);
                assert_eq!(owner.reap_pending(Instant::now() + Duration::from_secs(30)).await, 0);
                assert_eq!(owner.pending_cleanup_count(), 0);
                assert_eq!(owner.inner.helper_slots.0.available_permits(), available_before);
            }

            #[tokio::test]
            async fn oversized_output_is_stopped_at_sixteen_kibibytes() {
                let (directory, config) = helper("head -c 17000 /dev/zero");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let result = lookup_owned(
                    &AuthOwner::new(HelperSlots::new()),
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(60),
                    &CancellationToken::new(),
                )
                .await;
                assert!(matches!(result, Err(AuthError::OutputTooLarge) | Err(AuthError::CleanupPending)));
                drop(directory);
            }
        }
    }
}
