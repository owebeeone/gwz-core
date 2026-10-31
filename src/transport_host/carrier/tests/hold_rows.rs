//! The Linux hold's rows (step 2.3a remediation rounds 1 and 2: state P2-1 and P3-1, then N-1 and N-3). The peer is
//! pinned by the descriptor the kernel recorded for the connection (`SO_PEERPIDFD`), not by a process ID read from it
//! afterwards; and a seccomp filter on any thread of a process counts. A fixture the host cannot make fails its row with
//! a message that names the missing capability; nothing is skipped.
use super::super::{
    CarrierLimits,
    local::{
        Hold, LocalAddress, LocalRefusal, PeerPolicy, PeerRefusal, Principal, connect, peer_of,
        sandbox::{Cause, of_peer},
    },
};
use super::{
    local_rows::*,
    sandbox_rows::{
        accept_within, client_is_refused,
        fixture::{Fixture, How, seccomp_allow_all},
        listener_is_refused, own_is_refused,
    },
    support::WAIT,
};
use std::{
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
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
    let take = |errno: i32| {
        let ask = move |_| Err(io::Error::from_raw_os_error(errno));
        Hold::take_with(one.as_raw_fd(), pid, uid, &ask)
            .err()
            .expect("refused")
    };
    // A kernel before 6.5 answers `ENOPROTOOPT`: the requirement is named, and only this answer names it.
    let refused = take(libc::ENOPROTOOPT);
    let text = refused.to_string();
    assert!(
        matches!(refused, PeerRefusal::Unreadable(_))
            && text.contains("SO_PEERPIDFD")
            && text.contains("Linux 6.5"),
        "{text}"
    );
    // Kernels 6.5 to 6.15 have the option and answer `EINVAL` for a peer that has gone (`ESRCH` when it is reaped
    // during the call): the peer is gone, and the kernel is not blamed.
    for errno in [libc::EINVAL, libc::ESRCH] {
        let refused = take(errno);
        assert_eq!(refused, GONE, "{errno}");
        assert!(
            !refused.to_string().contains("Linux 6.5"),
            "{errno}: {refused}"
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

/// What a kernel of 6.5 to 6.15 answers for a connector or listener that has been reaped: `EINVAL`, where 6.16 and later
/// give a stale descriptor (the host's own kernel answers one of the two).
fn kernel_before_6_16(_: RawFd) -> io::Result<OwnedFd> {
    Err(io::Error::from_raw_os_error(libc::EINVAL))
}

/// The connection `stream`'s peer is a process that has been reaped: the real call and a 6.5 to 6.15 kernel's answer
/// are both refused as gone.
fn gone_whatever_the_kernel_answers(stream: &UnixStream, what: &str) {
    let fd = stream.as_raw_fd();
    let peer = peer_of(fd).expect("the credentials of a connection whose peer has gone");
    let (Some(pid), Principal::Uid(uid)) = (peer.pid, peer.principal) else {
        panic!("{what}: a pid and a uid")
    };
    for (kernel, refused) in [
        ("this host's kernel", Hold::take(fd, pid, uid)),
        (
            "a kernel of 6.5 to 6.15",
            Hold::take_with(fd, pid, uid, &kernel_before_6_16),
        ),
    ] {
        assert_eq!(refused.err(), Some(GONE), "{what}, {kernel}");
    }
}

/// A connector and a listener that have gone, as real processes: a client that connects and ends (its connection waits
/// in the listener's queue), and a server that hands its listening socket to a `sleep` and ends (the socket stays open).
#[test]
fn a_reaped_connector_and_a_reaped_listener_are_refused_gone_on_every_kernel_answer() {
    let dir = dir();
    let path = dir_runtime(&dir).path().join("reaped-connector");
    let socket = UnixListener::bind(&path).expect("bind");
    let mut client = Fixture::spawn("connect_exit", &path, How::Plain);
    client.wait_exit();
    let (stream, _) = socket.accept().expect("the queued connection");
    gone_whatever_the_kernel_answers(&stream, "a reaped connector");
    let path = dir_runtime(&dir).path().join("reaped-listener");
    let mut server = Fixture::spawn("listen_handoff", &path, How::Plain);
    server.wait_for("listening");
    let _holder = Holder(
        server
            .wait_for("handed:")
            .trim_start_matches("handed:")
            .parse()
            .unwrap(),
    );
    server.wait_exit();
    let stream = UnixStream::connect(&path).expect("the socket the holder keeps open");
    gone_whatever_the_kernel_answers(&stream, "a reaped listener");
}

/// A descriptor of a live process that is not the one the credentials name: the pidfd of a `sleep`.
#[test]
fn a_descriptor_that_names_another_live_process_is_refused_gone() {
    let (one, _other, pid, uid) = own_pair();
    let sleeper = Ended(
        Command::new("sleep")
            .arg("600")
            .stdout(Stdio::null())
            .spawn()
            .expect("sleep"),
    );
    let other = sleeper.0.id();
    assert_ne!(other, pid);
    let ask = move |_: RawFd| {
        // SAFETY: `pidfd_open` takes a process ID and flags and returns a descriptor or -1.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, other as libc::pid_t, 0) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: `fd` is a new descriptor this value owns.
            Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
        }
    };
    let refused = Hold::take_with(one.as_raw_fd(), pid, uid, &ask)
        .err()
        .expect("a descriptor that names a live process with another ID is refused");
    assert_eq!(refused, GONE);
}

const EXEC_PATH: &str = "GWZ_EXEC_ROW_PATH";
const EXEC_DELAY: &str = "GWZ_EXEC_ROW_DELAY_MICROS";

/// The test that is the fixture process of the exec row: run on its own it does nothing and passes. Its first thread
/// is never filtered. A second thread puts itself under an allow-all seccomp filter, connects to the row's socket,
/// waits the stated time and `exec`s: the thread takes over the process's ID and the first thread goes, so the
/// process is left with the filter, and the thread's own `/proc` entry has vanished.
#[test]
fn exec_child() {
    let (Some(path), Some(delay)) = (std::env::var_os(EXEC_PATH), std::env::var_os(EXEC_DELAY))
    else {
        return;
    };
    let path = PathBuf::from(path);
    let delay = Duration::from_micros(delay.to_str().unwrap().parse().unwrap());
    thread::spawn(move || {
        seccomp_allow_all().expect("seccomp permitted");
        let _stream = UnixStream::connect(&path).expect("connect");
        thread::sleep(delay);
        let error = Command::new("sleep").arg("600").exec();
        panic!("exec: {error}");
    })
    .join()
    .unwrap();
}

/// How many processes the exec row starts, and over what span of microseconds their `exec`s are spread: a scan of a
/// process takes tens of microseconds, so across the trials the exec lands before, inside and after one.
const EXEC_TRIALS: u32 = 150;
const EXEC_SPREAD_MICROS: u32 = 3000;
/// How many times each trial scans the process while its `exec` comes: a bounded count, long enough to span the delay.
const EXEC_SCANS: u32 = 400;
/// How many trials may have a scan that sees no filter. The kernel hides an `exec`ing thread from `/proc` for the
/// instant it takes over the leader's ID, in which no read can find the filter (measured: about one trial in a thousand
/// under load, with the leader read again; about one in four without), so the row states a rate and not zero.
const EXEC_CLEAN_TRIALS_ALLOWED: u32 = EXEC_TRIALS / 15;

/// State N-2: a filtered thread that is not the leader and `exec`s during the scan takes over the leader's ID, so the
/// leader read first was the old one and the thread's own entry in `task/` is gone. A scan that saw no filter while the
/// hold still names the process is a hole; a refusal, or the filter seen, is the rule. The leader is read again after the
/// threads (the fix): without that read about a fifth of the trials have such a scan, and with it about one in a thousand.
#[test]
fn a_filtered_thread_that_execs_during_the_scan_does_not_hide_its_filter() {
    let dir = dir();
    let path = dir_runtime(&dir).path().join("exec-race");
    let socket = UnixListener::bind(&path).expect("bind");
    let exe = std::env::current_exe().unwrap();
    let child_test = {
        let module = module_path!();
        format!(
            "{}::exec_child",
            module.split_once("::").map_or(module, |(_, rest)| rest)
        )
    };
    let (mut filtered, mut refused, mut missed) = (0u32, 0u32, 0u32);
    for trial in 0..EXEC_TRIALS {
        let delay = trial * (EXEC_SPREAD_MICROS / EXEC_TRIALS);
        let _child = Ended(
            Command::new(&exe)
                .args(["--exact", &child_test, "--nocapture", "--test-threads=1"])
                .env(EXEC_PATH, &path)
                .env(EXEC_DELAY, delay.to_string())
                .stdout(Stdio::null())
                .spawn()
                .expect("the exec fixture"),
        );
        let (tx, accepted) = mpsc::channel();
        let queue = socket.try_clone().expect("the listening socket");
        thread::spawn(move || tx.send(queue.accept().map(|(stream, _)| stream)));
        let stream = accepted
            .recv_timeout(WAIT)
            .expect("nothing connected: the exec fixture did not reach the socket")
            .expect("accept");
        let fd = stream.as_raw_fd();
        let peer = peer_of(fd).expect("credentials");
        let (Some(pid), Principal::Uid(uid)) = (peer.pid, peer.principal) else {
            panic!("a pid and a uid")
        };
        let Ok(hold) = Hold::take(fd, pid, uid) else {
            refused += 1;
            continue;
        };
        let mut clean = false;
        for _ in 0..EXEC_SCANS {
            match of_peer(&hold, fd) {
                Err(_) => refused += 1,
                Ok(found) if found.seccomp != 0 => filtered += 1,
                Ok(_) if hold.check(fd).is_err() => refused += 1,
                Ok(_) => clean = true,
            }
        }
        missed += u32::from(clean);
    }
    assert!(
        missed <= EXEC_CLEAN_TRIALS_ALLOWED,
        "of {EXEC_TRIALS} trials of {EXEC_SCANS} scans, {missed} had a scan that saw no filter while the hold still \
         named the process (at most {EXEC_CLEAN_TRIALS_ALLOWED} allowed; {filtered} scans saw the filter, {refused} were refused)"
    );
}
