//! The Windows twin of `mod unix` beside it (plan step 3.3): libssh2's own agent client over a pipe, whose reply wait
//! the session's timeout does not bound either. A child of the test binary runs libssh2's agent against a raw pipe
//! server that reads the request and never answers; the parent shows that the child is still waiting, then ends it.
use crate::git::endpoint::ssh_tests::{
    agent_fixture_pipe::pipe_name, agent_pipe_server::RawServer,
};
use std::{
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The full name of `agent_child` in this test binary.
fn child_test() -> String {
    let module = module_path!();
    let module = module.split_once("::").map_or(module, |(_, path)| path);
    format!("{module}::agent_child")
}

#[test]
#[ignore = "child only; parent supplies an isolated fake agent"]
fn agent_child() {
    let pipe = std::env::var_os("GWZ_TEST_AGENT_WAIT_PIPE").expect("child pipe");
    let session = ssh2::Session::new().unwrap();
    session.set_blocking(false);
    session.set_timeout(25);
    let mut agent = session.agent().unwrap();
    agent
        .set_identity_path(std::path::Path::new(&pipe))
        .unwrap();
    agent.connect().unwrap();
    let _ = agent.list_identities();
}

#[test]
fn session_timeout_does_not_bound_native_agent_reply_wait() {
    let temp = tempfile::tempdir().unwrap();
    let name = pipe_name(temp.path());
    // The server reads the request and never answers: an empty reply, until the client leaves.
    let server = RawServer::start(&name, Vec::new());
    let mut child = Reap(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &child_test(), "--ignored", "--nocapture"])
            .env("GWZ_TEST_AGENT_WAIT_PIPE", &name)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    server.wait_replied();
    // Deliberately withhold a reply. No real agent, keys or host involved.
    thread::sleep(Duration::from_millis(200));
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "API behavior changed; revisit setup design"
    );
    println!(
        "Native agent list blocked beyond 25 ms session timeout and nonblocking mode; parent terminates/reaps child after 200 ms observation."
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let seen = server.join();
    assert_eq!(seen.request, [0, 0, 0, 1, 11], "agent identity request");
}
