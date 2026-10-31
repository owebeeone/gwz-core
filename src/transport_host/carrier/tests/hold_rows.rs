//! The Linux hold's rows (step 2.3a remediation round 1: state P2-1 and P3-1). The peer is pinned by the descriptor the
//! kernel recorded for the connection (`SO_PEERPIDFD`), not by a process ID read from it afterwards; and a seccomp filter
//! on any thread of a process counts. A fixture the host cannot make fails its row with a message that names the missing
//! capability; nothing is skipped.
use super::super::{
    CarrierLimits,
    local::{Hold, LocalAddress, LocalRefusal, PeerPolicy, PeerRefusal, connect, sandbox::Cause},
};
use super::{
    local_rows::*,
    sandbox_rows::{
        accept_within, client_is_refused,
        fixture::{Fixture, How},
        listener_is_refused, own_is_refused,
    },
};
use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
    process::{Child, Command, Stdio},
};

/// A process a row started, ended when the row is over.
struct Ended(Child);

impl Drop for Ended {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A `sleep` another process of the row handed its socket to; ended when the row is over. It is not this process's child,
/// so it is signalled by the ID the fixture said.
struct Holder(u32);

impl Drop for Holder {
    fn drop(&mut self) {
        // SAFETY: signals the one process the fixture started and named.
        unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGKILL) };
    }
}

const NS_LAST_PID: &str = "/proc/sys/kernel/ns_last_pid";

/// A new `sleep` that has the process ID `pid`: the ID of a process that has been reaped, given to another. The next
/// process this one forks takes the ID after `ns_last_pid`, so the file is set and a fork tried until another process of
/// the test binary does not take the ID first.
fn reuse(pid: u32) -> Ended {
    for _ in 0..2000 {
        std::fs::write(NS_LAST_PID, (pid - 1).to_string()).unwrap_or_else(|error| {
            panic!(
                "the pid-reuse fixture cannot be made on this host: {error} (it needs write access to {NS_LAST_PID}: \
                 root with CAP_SYS_ADMIN or CAP_CHECKPOINT_RESTORE, in a /proc that is not read-only)"
            )
        });
        let child = Ended(
            Command::new("sleep")
                .arg("600")
                .stdout(Stdio::null())
                .spawn()
                .expect("sleep"),
        );
        if child.0.id() == pid {
            return child;
        }
    }
    panic!("the process ID {pid} could not be given to a new process: others keep taking it");
}

/// What a hold on a process that has gone says.
const GONE: PeerRefusal = PeerRefusal::Unreadable("a live peer process");

/// A sandboxed client that connects and hands the connection to a `sleep` that outlives it; the client is reaped and its
/// ID given to another process. The listener must refuse the connection: it was made by a process that has gone.
fn client_whose_process_id_was_reused() {
    let dir = dir();
    let listener = listener(&dir);
    let path = listener.address().path().to_path_buf();
    let mut client = Fixture::spawn("connect_handoff", &path, How::Seccomp);
    let _holder = Holder(
        client
            .wait_for("handed:")
            .trim_start_matches("handed:")
            .parse()
            .unwrap(),
    );
    client.wait_exit();
    // The ID of the client that made the connection now names an unsandboxed process of this user.
    let _impostor = reuse(client.pid());
    let refused = accept_within(listener)
        .err()
        .expect("the connection's holder is a sandboxed process: refused");
    assert_eq!(refused, LocalRefusal::Peer(GONE));
}

/// The same for a sandboxed listener that hands its listening socket to a `sleep` and goes; the client must refuse it.
fn listener_whose_process_id_was_reused() {
    let dir = dir();
    let path = dir_runtime(&dir).path().join("s-0123456789abcdef");
    let mut server = Fixture::spawn("listen_handoff", &path, How::Seccomp);
    server.wait_for("listening");
    let _holder = Holder(
        server
            .wait_for("handed:")
            .trim_start_matches("handed:")
            .parse()
            .unwrap(),
    );
    server.wait_exit();
    let _impostor = reuse(server.pid());
    let address = LocalAddress::from_path(&path);
    let refused = connect(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .err()
    .expect("the socket's holder is a sandboxed process: refused");
    assert_eq!(refused, LocalRefusal::Peer(GONE));
}

/// One row for both, so that the two never move `ns_last_pid` at once.
#[test]
fn a_reused_process_id_does_not_stand_for_the_process_that_connected_or_listened() {
    client_whose_process_id_was_reused();
    listener_whose_process_id_was_reused();
}

#[test]
fn a_seccomp_filter_on_any_thread_refuses_the_client_the_listener_and_the_caller() {
    client_is_refused("thread_connect", How::Plain, Cause::Seccomp);
    listener_is_refused("thread_listen", How::Plain, Cause::Seccomp);
    own_is_refused("thread_own", How::Plain, Cause::Seccomp);
}

/// A connected socket of this process's own, and this process's ID and user.
fn own_pair() -> (UnixStream, UnixStream, u32, u32) {
    let (one, other) = UnixStream::pair().expect("a socket pair");
    // SAFETY: `geteuid` has no preconditions.
    (one, other, std::process::id(), unsafe { libc::geteuid() })
}

#[test]
fn a_kernel_or_socket_that_cannot_name_the_peer_process_is_refused_with_its_cause() {
    let (one, _other, pid, uid) = own_pair();
    // A kernel before 6.5 answers `ENOPROTOOPT` (or `EINVAL`): the requirement is named.
    for errno in [libc::ENOPROTOOPT, libc::EINVAL] {
        let ask = move |_| Err(io::Error::from_raw_os_error(errno));
        let refused = Hold::take_with(one.as_raw_fd(), pid, uid, &ask)
            .err()
            .expect("refused");
        let text = refused.to_string();
        assert!(
            matches!(refused, PeerRefusal::Unreadable(_))
                && text.contains("SO_PEERPIDFD")
                && text.contains("Linux 6.5"),
            "{errno}: {text}"
        );
    }
    // Any other failure refuses too, without blaming the kernel.
    let ask = |_| Err(io::Error::from_raw_os_error(libc::EBADF));
    let refused = Hold::take_with(one.as_raw_fd(), pid, uid, &ask)
        .err()
        .expect("refused");
    assert!(
        matches!(refused, PeerRefusal::Unreadable(_)) && !refused.to_string().contains("Linux 6.5"),
        "{refused}"
    );
    // A descriptor that is not a connected socket (a pipe's) is refused by the real call.
    let (read, _write) = io::pipe().expect("a pipe");
    let refused = Hold::take(read.as_raw_fd(), pid, uid)
        .err()
        .expect("refused");
    assert!(matches!(refused, PeerRefusal::Unreadable(_)), "{refused}");
}

#[test]
fn a_hold_is_the_descriptor_of_the_process_the_credentials_name_and_no_other() {
    let (one, _other, pid, uid) = own_pair();
    let hold = Hold::take(one.as_raw_fd(), pid, uid).expect("a hold on this process");
    assert_eq!(hold.pid(), pid);
    assert!(hold.check(one.as_raw_fd()).is_ok());
    assert!(hold.confirm(one.as_raw_fd()).is_ok());
    // The credentials said another process ID than the descriptor names: refused, whichever is right.
    let refused = Hold::take(one.as_raw_fd(), pid + 1, uid)
        .err()
        .expect("refused");
    assert_eq!(refused, GONE);
}
