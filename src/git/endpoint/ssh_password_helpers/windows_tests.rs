//! The Windows arm of the password-helper lookup (plan step 1.4): a refusal that names WH2.
use super::*;
use crate::git::endpoint::agent_job::Job;
use std::{
    path::PathBuf,
    task::{Context, Poll, Waker},
    time::Instant,
};

#[test]
fn a_configured_password_helper_is_refused_on_windows_naming_wh2() {
    let helpers = Helpers::new(
        https_auth::Config {
            executable: PathBuf::from("git"),
            environment: Vec::new(),
        },
        https_auth::HelperSlots::new(),
    );
    let key = Key::ssh("git", "127.0.0.1", 22);
    let mut job = Job::start_isolated(
        Some(Instant::now() + Duration::from_secs(5)),
        Duration::from_secs(1),
        move |control| {
            helpers
                .lookup(&key, &Opening::default(), &control)
                .map(|_| ())
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let error = loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            break result.expect_err("a Windows build has no helper runner");
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(error.to_string().contains("WH2"), "{error}");
}
