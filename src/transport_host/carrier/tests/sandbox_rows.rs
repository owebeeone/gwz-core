//! The sandbox rule's rows (plan 2.3a): the decision table over injected findings, the rule on the wire, and real
//! processes under each platform's sandbox. A fixture the host cannot make fails its row with a message that names the
//! missing capability; nothing is skipped.
use super::super::{
    Carrier, CarrierLimits, CloseReason,
    local::{
        Integrity, LocalListener, LocalRefusal, PeerIdentity, PeerPolicy, PeerRefusal, Principal,
        Relation,
        sandbox::{Cause, INIT_USER_NAMESPACE, SandboxFindings},
    },
};
use super::{local_rows::*, support::*};
use std::{thread, time::Duration};

const SHORT: Duration = Duration::from_millis(50);
const RELATIONS: [Relation; 2] = [
    Relation::ListenerChecksClient,
    Relation::ClientChecksListener,
];

fn who(sandbox: SandboxFindings) -> PeerIdentity {
    PeerIdentity {
        principal: Principal::Uid(7),
        pid: Some(1),
        integrity: None,
        app_container: false,
        sandbox,
    }
}

fn seccomp(mode: u32) -> SandboxFindings {
    SandboxFindings {
        seccomp: mode,
        ..SandboxFindings::default()
    }
}

fn spaces(user: u64, mount: u64) -> SandboxFindings {
    SandboxFindings {
        user_namespace: Some(user),
        mount_namespace: Some(mount),
        ..SandboxFindings::default()
    }
}

fn profile() -> SandboxFindings {
    SandboxFindings {
        profile: true,
        ..SandboxFindings::default()
    }
}

#[test]
fn the_decision_table_over_injected_findings() {
    let clean = SandboxFindings::default();
    let init = spaces(INIT_USER_NAMESPACE, 100);
    let refused = |cause| Err(PeerRefusal::Sandboxed(cause));
    let cases: Vec<(
        &str,
        SandboxFindings,
        SandboxFindings,
        Result<(), PeerRefusal>,
    )> = vec![
        ("nothing found", clean, clean, Ok(())),
        ("initial namespaces, both", init, init, Ok(())),
        ("strict seccomp", clean, seccomp(1), refused(Cause::Seccomp)),
        ("filter seccomp", clean, seccomp(2), refused(Cause::Seccomp)),
        (
            "seccomp beside a user namespace",
            clean,
            SandboxFindings {
                seccomp: 2,
                ..spaces(5000, 100)
            },
            refused(Cause::Seccomp),
        ),
        (
            "a user namespace",
            init,
            spaces(5000, 100),
            refused(Cause::UserNamespace),
        ),
        (
            "a sandbox profile",
            clean,
            profile(),
            refused(Cause::Profile),
        ),
        (
            "this side is sandboxed, the peer is not",
            seccomp(2),
            clean,
            refused(Cause::Seccomp),
        ),
        (
            "both sandboxed, equal filters",
            seccomp(2),
            seccomp(2),
            refused(Cause::Seccomp),
        ),
        (
            "another mount namespace",
            init,
            spaces(INIT_USER_NAMESPACE, 200),
            refused(Cause::OtherNamespace),
        ),
        (
            "namespaces read for the peer only",
            clean,
            init,
            refused(Cause::OtherNamespace),
        ),
        (
            "namespaces read for this side only",
            init,
            clean,
            refused(Cause::OtherNamespace),
        ),
    ];
    for (name, own, observed, expected) in cases {
        for relation in RELATIONS {
            let policy = PeerPolicy::from_identity(who(own));
            assert_eq!(
                policy.verify(&who(observed), relation),
                expected,
                "{name} {relation:?}"
            );
        }
    }
    // Another user is refused as another user, and an AppContainer as one, before a sandbox is looked at.
    let policy = PeerPolicy::from_identity(who(clean));
    let mut stranger = who(seccomp(2));
    stranger.principal = Principal::Uid(8);
    assert_eq!(
        policy.verify(&stranger, Relation::ListenerChecksClient),
        Err(PeerRefusal::OtherPrincipal)
    );
    let mut boxed = who(seccomp(2));
    boxed.app_container = true;
    assert_eq!(
        policy.verify(&boxed, Relation::ListenerChecksClient),
        Err(PeerRefusal::AppContainer)
    );
    assert_eq!(INIT_USER_NAMESPACE, 0xEFFF_FFFD);
}

#[test]
fn a_caller_that_is_sandboxed_gets_no_policy_and_the_message_names_only_the_cause() {
    let own = |sandbox| PeerPolicy::try_from_identity(who(sandbox)).err();
    let sandboxed = |cause| Some(LocalRefusal::Own(PeerRefusal::Sandboxed(cause)));
    assert_eq!(own(SandboxFindings::default()), None);
    assert_eq!(own(spaces(INIT_USER_NAMESPACE, 100)), None);
    assert_eq!(own(seccomp(2)), sandboxed(Cause::Seccomp));
    assert_eq!(own(spaces(5000, 100)), sandboxed(Cause::UserNamespace));
    assert_eq!(own(profile()), sandboxed(Cause::Profile));
    let mut boxed = who(SandboxFindings::default());
    boxed.app_container = true;
    assert_eq!(
        PeerPolicy::try_from_identity(boxed).err(),
        Some(LocalRefusal::Own(PeerRefusal::AppContainer))
    );
    let mut low = who(SandboxFindings::default());
    low.integrity = Some(Integrity::Low);
    assert_eq!(
        PeerPolicy::try_from_identity(low).err(),
        Some(LocalRefusal::Own(PeerRefusal::Integrity))
    );
    let causes = [
        Cause::Seccomp,
        Cause::UserNamespace,
        Cause::OtherNamespace,
        Cause::Profile,
    ];
    let messages: Vec<String> = causes
        .iter()
        .map(|c| PeerRefusal::Sandboxed(*c).to_string())
        .collect();
    for (message, other) in messages.iter().zip(messages.iter().skip(1)) {
        assert_ne!(message, other);
    }
    for message in &messages {
        assert!(
            message.starts_with("the peer is sandboxed by "),
            "{message}"
        );
        assert!(
            !message.contains(|c: char| c.is_ascii_digit() || c == '/'),
            "{message}"
        );
    }
}

#[test]
fn a_refused_caller_is_told_about_this_process_and_never_about_a_peer() {
    for sandbox in [seccomp(2), spaces(5000, 100), profile()] {
        let shown = PeerPolicy::try_from_identity(who(sandbox))
            .expect_err("refused")
            .to_string();
        assert!(
            shown.starts_with("this process is sandboxed by "),
            "{shown}"
        );
        assert!(!shown.contains("peer"), "{shown}");
    }
    let mut boxed = who(SandboxFindings::default());
    boxed.app_container = true;
    let unreadable = LocalRefusal::Own(PeerRefusal::Unreadable("the process status"));
    for refusal in [
        PeerPolicy::try_from_identity(boxed).expect_err("refused"),
        unreadable,
    ] {
        let shown = refusal.to_string();
        assert!(shown.starts_with("this process "), "{shown}");
        assert!(!shown.contains("peer"), "{shown}");
    }
    // A peer is still a peer.
    assert!(
        LocalRefusal::Peer(PeerRefusal::Sandboxed(Cause::Seccomp))
            .to_string()
            .contains("the peer is sandboxed")
    );
}

/// This process's policy with `findings` where the sandbox findings are: a sandbox the host does not have.
fn policy_with(findings: SandboxFindings) -> PeerPolicy {
    let mut own = PeerPolicy::current_user()
        .expect("a host that is not itself sandboxed")
        .own()
        .clone();
    own.sandbox = findings;
    PeerPolicy::from_identity(own)
}

/// Findings that refuse a clean peer, each with the cause it must name.
fn refusing() -> [(SandboxFindings, Cause, &'static str); 2] {
    [
        (seccomp(2), Cause::Seccomp, "seccomp filter"),
        (
            spaces(INIT_USER_NAMESPACE, 1),
            Cause::OtherNamespace,
            "namespace other than",
        ),
    ]
}

#[test]
fn a_listener_that_refuses_answers_one_session_error_and_ends_the_stream() {
    for (findings, cause, text) in refusing() {
        let dir = dir();
        let strict = LocalListener::bind(
            dir_runtime(&dir),
            policy_with(findings),
            CarrierLimits::DEFAULT,
        )
        .unwrap();
        let address = strict.address().clone();
        let accepting = thread::spawn(move || accepted(&strict));
        let client = connected(
            &address,
            &PeerPolicy::current_user().unwrap(),
            CarrierLimits::DEFAULT,
        )
        .unwrap();
        let refused = accepting.join().unwrap().err().expect("refused at accept");
        assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::Sandboxed(cause)));
        let answer = eventually("the answer", || {
            client.recv_registry_timeout(SHORT).unwrap()
        });
        let (call_id, code, message) = decode_session_error(&answer);
        assert_eq!((call_id, code), (0, 82));
        assert!(message.contains(text), "{message}");
        assert_eq!(
            eventually("the closure", || client.try_recv().err()).reason,
            CloseReason::PeerGone
        );
        assert!(
            client.recv_registry_timeout(SHORT).is_err(),
            "a second frame"
        );
    }
}

#[test]
fn a_client_that_refuses_its_listener_sends_nothing() {
    for (findings, cause, _) in refusing() {
        let dir = dir();
        let squatter = listener(&dir);
        let address = squatter.address().clone();
        let accepting = thread::spawn(move || accepted(&squatter));
        let refused = connected(&address, &policy_with(findings), CarrierLimits::DEFAULT)
            .err()
            .expect("refused");
        assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::Sandboxed(cause)));
        if let Ok(end) = accepting.join().unwrap() {
            assert_eq!(
                eventually("the closure", || end.try_recv().err()).reason,
                CloseReason::PeerGone
            );
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::super::{Peer, StreamCarrier};
        use fixture::{Fixture, How, frame_received};
        use std::sync::mpsc;

        /// An accept that a fixture that never connects cannot hang.
        pub(super) fn accept_within(listener: LocalListener) -> Result<StreamCarrier, LocalRefusal> {
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || tx.send(accepted(&listener)));
            rx.recv_timeout(WAIT)
                .expect("nothing connected: the fixture process did not reach the listener")
        }

        pub(super) mod fixture {
            use super::*;
            use gwz_session_contract::Frame;
            use std::{
                io::{BufRead, BufReader, Read},
                os::{
                    fd::AsRawFd,
                    unix::{
                        fs::PermissionsExt,
                        net::{UnixListener, UnixStream},
                        process::CommandExt,
                    },
                },
                path::{Path, PathBuf},
                process::{Child, Command, Stdio},
            };

            const ROLE: &str = "GWZ_SANDBOX_ROW_ROLE";
            const PATH: &str = "GWZ_SANDBOX_ROW_PATH";

            /// The test that is the fixture process: run on its own it does nothing and passes.
            fn child_test() -> String {
                let path = module_path!();
                let relative = path.split_once("::").map_or(path, |(_, rest)| rest);
                format!("{relative}::sandbox_child")
            }

            /// A started process of the roles below, its output read on a thread so that no row can hang on it.
            pub(in super::super) struct Fixture {
                child: Child,
                lines: mpsc::Receiver<String>,
            }

            impl Fixture {
                pub(in super::super) fn spawn(role: &str, path: &Path, how: How) -> Self {
                    let mut command = base(how);
                    command
                        .args(["--exact", &child_test(), "--nocapture", "--test-threads=1"])
                        .env(ROLE, role)
                        .env(PATH, path)
                        .stdout(Stdio::piped());
                    confine(&mut command, how);
                    let mut child = command.spawn().unwrap_or_else(|error| {
                        panic!("the {how:?} fixture cannot be made on this host: {error} (it needs {})", needs(how))
                    });
                    let out = child.stdout.take().unwrap();
                    let (tx, lines) = mpsc::channel();
                    thread::spawn(move || {
                        for line in BufReader::new(out).lines().map_while(Result::ok) {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                    });
                    Self { child, lines }
                }

                pub(in super::super) fn pid(&self) -> u32 {
                    self.child.id()
                }

                /// Waits for the process to end.
                pub(in super::super) fn wait_exit(&mut self) {
                    self.child.wait().unwrap();
                }

                /// What the fixture said from `prefix` on (libtest's `test ... ` precedes the first thing it prints).
                pub(in super::super) fn wait_for(&self, prefix: &str) -> String {
                    loop {
                        let line = self.lines.recv_timeout(WAIT).unwrap_or_else(|_| panic!("the fixture never said {prefix:?}"));
                        if let Some(at) = line.find(prefix) {
                            return line[at..].to_string();
                        }
                    }
                }

                /// The bytes the fixture's stream received, once it has ended.
                pub(in super::super) fn received(&self) -> Vec<u8> {
                    let hex = self.wait_for("received:");
                    let hex = hex.trim_start_matches("received:");
                    (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap()).collect()
                }
            }

            impl Drop for Fixture {
                fn drop(&mut self) {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                }
            }

            cfg_if::cfg_if! {
                if #[cfg(any(target_os = "linux", target_os = "android"))] {
                    /// What the fixture process is put under before it starts.
                    #[derive(Clone, Copy, Debug)]
                    pub(in super::super) enum How {
                        Plain,
                        Seccomp,
                        UserNamespace,
                        MountNamespace,
                    }

                    fn base(_: How) -> Command {
                        Command::new(std::env::current_exe().unwrap())
                    }

                    fn needs(how: How) -> &'static str {
                        match how {
                            How::Plain => "nothing",
                            How::Seccomp => "prctl(PR_SET_SECCOMP) permitted",
                            How::UserNamespace => "unprivileged user namespaces (kernel.unprivileged_userns_clone, no AppArmor restriction)",
                            How::MountNamespace => "CAP_SYS_ADMIN (run as root: WSL, or a privileged container)",
                        }
                    }

                    /// Puts the process under `how` between `fork` and `exec`, where it is the only thread, which
                    /// `unshare(CLONE_NEWUSER)` needs. The new user namespace maps this user to itself, so that the
                    /// process can still create files.
                    fn confine(command: &mut Command, how: How) {
                        // SAFETY: plain reads of this process's ids.
                        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
                        let maps = [
                            ("/proc/self/setgroups", "deny\n".to_string()),
                            ("/proc/self/gid_map", format!("{gid} {gid} 1\n")),
                            ("/proc/self/uid_map", format!("{uid} {uid} 1\n")),
                        ];
                        // SAFETY: the closure only makes system calls and writes files, with buffers made before the fork.
                        unsafe { command.pre_exec(move || apply(how, &maps)) };
                    }

                    fn check(result: libc::c_int) -> std::io::Result<()> {
                        if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
                    }

                    /// An allow-all filter of one instruction (`BPF_RET | BPF_K`, `SECCOMP_RET_ALLOW`) on the calling thread
                    /// alone (no `TSYNC`): the process's `Seccomp:` line is the first thread's.
                    fn seccomp_allow_all() -> std::io::Result<()> {
                        // SAFETY: plain `prctl` calls; the program outlives the call that reads it.
                        unsafe {
                            check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong))?;
                            let mut allow = [libc::sock_filter { code: 0x06, jt: 0, jf: 0, k: 0x7fff_0000 }];
                            let program = libc::sock_fprog { len: 1, filter: allow.as_mut_ptr() };
                            check(libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER as libc::c_ulong, &raw const program as libc::c_ulong))
                        }
                    }

                    /// The roles whose filter is on a second thread: `thread_connect` and `thread_listen` do what `connect` and
                    /// `listen` do, from a thread that has put itself under the filter; `thread_own` asks for a policy from the
                    /// first thread while another is under it.
                    fn platform_role(role: &str, path: &Path) -> bool {
                        let (path, under) = (path.to_path_buf(), |work: Box<dyn FnOnce() + Send>| {
                            thread::spawn(move || {
                                seccomp_allow_all().expect("seccomp permitted");
                                work();
                            })
                            .join()
                            .unwrap();
                        });
                        match role {
                            "thread_connect" => under(Box::new(move || {
                                let mut stream = UnixStream::connect(&path).expect("connect");
                                println!("connected");
                                report(&mut stream);
                            })),
                            "thread_listen" => under(Box::new(move || {
                                let listener = UnixListener::bind(&path).expect("bind");
                                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                                println!("listening");
                                let (mut stream, _) = listener.accept().expect("accept");
                                report(&mut stream);
                            })),
                            "thread_own" => {
                                let (ready, filtered) = mpsc::channel();
                                let (done, finished) = mpsc::channel::<()>();
                                let other = thread::spawn(move || {
                                    seccomp_allow_all().expect("seccomp permitted");
                                    ready.send(()).unwrap();
                                    let _ = finished.recv();
                                });
                                filtered.recv().unwrap();
                                say_own();
                                drop(done);
                                other.join().unwrap();
                            }
                            _ => return false,
                        }
                        true
                    }

                    fn apply(how: How, maps: &[(&str, String)]) -> std::io::Result<()> {
                        match how {
                            How::Plain => Ok(()),
                            How::Seccomp => seccomp_allow_all(),
                            How::UserNamespace => {
                                // SAFETY: no pointers.
                                check(unsafe { libc::unshare(libc::CLONE_NEWUSER) })?;
                                maps.iter().try_for_each(|(file, text)| std::fs::write(file, text))
                            }
                            // SAFETY: no pointers.
                            How::MountNamespace => check(unsafe { libc::unshare(libc::CLONE_NEWNS) }),
                        }
                    }
                } else {
                    /// What the fixture process is put under before it starts.
                    #[derive(Clone, Copy, Debug)]
                    pub(in super::super) enum How {
                        Plain,
                        Profile,
                    }

                    /// The test binary, under `sandbox-exec` with a profile that allows everything, for `Profile`.
                    fn base(how: How) -> Command {
                        let exe = std::env::current_exe().unwrap();
                        match how {
                            How::Plain => Command::new(exe),
                            How::Profile => {
                                let mut command = Command::new("sandbox-exec");
                                command.args(["-p", "(version 1)(allow default)"]).arg(exe);
                                command
                            }
                        }
                    }

                    fn needs(_: How) -> &'static str {
                        "/usr/bin/sandbox-exec, outside a sandbox that forbids sandbox_init"
                    }

                    /// macOS: the process is started under `sandbox-exec` (above); nothing to do between fork and exec.
                    fn confine(_: &mut Command, _: How) {}

                    /// macOS has no roles beyond the common ones.
                    fn platform_role(_: &str, _: &Path) -> bool {
                        false
                    }
                }
            }

            fn hex(bytes: &[u8]) -> String {
                bytes.iter().map(|byte| format!("{byte:02x}")).collect()
            }

            fn report(stream: &mut UnixStream) {
                let mut bytes = Vec::new();
                let _ = stream.read_to_end(&mut bytes);
                println!("received:{}", hex(&bytes));
            }

            /// Says whether this process gets a policy, and what the refusal tells the user.
            pub(super) fn say_own() {
                let own = PeerPolicy::current_user().map(|_| ());
                println!("own:{own:?}");
                if let Err(refusal) = &own {
                    println!("own_text:{refusal}");
                }
            }

            /// Hands `socket` to a `sleep` that outlives this process (the descriptor is made inheritable for it alone),
            /// so that the connection or the listening socket stays open in a process this one forked, and says its ID.
            fn hand_off(socket: &impl AsRawFd) -> Child {
                let raw = socket.as_raw_fd();
                let mut command = Command::new("sleep");
                command.arg("600").stdout(Stdio::null());
                // SAFETY: only `fcntl`, between fork and exec.
                unsafe {
                    command.pre_exec(move || {
                        if libc::fcntl(raw, libc::F_SETFD, 0) < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
                    })
                };
                let holder = command.spawn().expect("sleep");
                println!("handed:{}", holder.id());
                holder
            }

            /// The other process of the rows. `connect` connects and reads to the end; `connect_exit` connects and goes;
            /// `connect_handoff` connects, hands the connection to a `sleep` and goes; `listen` listens, accepts one
            /// connection and reads to the end; `listen_handoff` listens, hands the listening socket to a `sleep` and
            /// goes; `own` says whether this process gets a policy.
            #[test]
            fn sandbox_child() {
                let Some(role) = std::env::var_os(ROLE) else {
                    return;
                };
                let path = PathBuf::from(std::env::var_os(PATH).expect("a path"));
                if platform_role(role.to_str().unwrap(), &path) {
                    return;
                }
                match role.to_str().unwrap() {
                    "connect_handoff" => {
                        // The `sleep` outlives this process on purpose: it is not waited for.
                        std::mem::forget(hand_off(&UnixStream::connect(&path).expect("connect")));
                    }
                    "listen_handoff" => {
                        let listener = UnixListener::bind(&path).expect("bind");
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                        println!("listening");
                        std::mem::forget(hand_off(&listener));
                    }
                    "connect" => {
                        let mut stream = UnixStream::connect(&path).expect("connect");
                        println!("connected");
                        report(&mut stream);
                    }
                    "connect_exit" => {
                        let _stream = UnixStream::connect(&path).expect("connect");
                    }
                    "listen" => {
                        let listener = UnixListener::bind(&path).expect("bind");
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                        println!("listening");
                        let (mut stream, _) = listener.accept().expect("accept");
                        report(&mut stream);
                    }
                    "own" => say_own(),
                    other => panic!("no role {other}"),
                }
            }

            /// The one `SessionError` a refused client was owed, and nothing after it.
            pub(in super::super) fn frame_received(bytes: &[u8]) -> (u64, u64, String) {
                let mut rest = bytes;
                let (tag, body) = read_wire_frame(&mut rest);
                assert!(rest.is_empty(), "bytes after the one answer: {rest:?}");
                decode_session_error(&Frame { tag, body })
            }
        }

        /// A child that connects under `how` to a listener of this process, which must refuse it with `cause` and owe it
        /// exactly one `server_peer_refused` answer.
        pub(super) fn client_is_refused(role: &str, how: How, cause: Cause) {
            let dir = dir();
            let listener = listener(&dir);
            let path = listener.address().path().to_path_buf();
            let child = Fixture::spawn(role, &path, how);
            let refused = accept_within(listener).err().expect("refused at accept");
            assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::Sandboxed(cause)));
            let (call_id, code, message) = frame_received(&child.received());
            assert_eq!((call_id, code), (0, 82));
            assert!(message.starts_with("peer refused: the peer is sandboxed by "), "{message}");
        }

        /// A child that listens under `how`, which this process refuses as a client with `cause`, sending it nothing.
        pub(super) fn listener_is_refused(role: &str, how: How, cause: Cause) {
            let dir = dir();
            let path = dir_runtime(&dir).path().join("s-0123456789abcdef");
            let child = Fixture::spawn(role, &path, how);
            child.wait_for("listening");
            let address = super::super::local::LocalAddress::from_path(&path);
            let refused = super::super::local::connect(&address, &PeerPolicy::current_user().unwrap(), CarrierLimits::DEFAULT)
                .err()
                .expect("refused as a listener");
            assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::Sandboxed(cause)));
            assert!(child.received().is_empty(), "the refused listener received bytes");
        }

        /// A child under `how` whose own policy is refused.
        pub(super) fn own_is_refused(role: &str, how: How, cause: Cause) {
            let child = Fixture::spawn(role, std::path::Path::new("/"), how);
            let expected = format!("own:Err(Own(Sandboxed({cause:?})))");
            assert_eq!(child.wait_for("own:"), expected);
            let shown = child.wait_for("own_text:");
            assert!(shown.starts_with("own_text:this process is sandboxed by ") && !shown.contains("peer"), "{shown}");
        }

        #[test]
        fn a_plain_client_is_accepted_and_a_plain_listener_is_reached() {
            let dir = dir();
            let listener = listener(&dir);
            let path = listener.address().path().to_path_buf();
            let child = Fixture::spawn("connect", &path, How::Plain);
            let carrier = accept_within(listener).expect("an unsandboxed child of this user is accepted");
            let Peer::Verified(peer) = carrier.binding().peer().clone() else {
                panic!("verified")
            };
            assert_eq!(peer.pid, Some(child.pid()));
            assert_eq!(peer.sandbox.cause(), None);
            let path = dir_runtime(&dir).path().join("s-0123456789abcdef");
            let server = Fixture::spawn("listen", &path, How::Plain);
            server.wait_for("listening");
            let address = super::super::local::LocalAddress::from_path(&path);
            let reached = super::super::local::connect(&address, &PeerPolicy::current_user().unwrap(), CarrierLimits::DEFAULT);
            assert!(reached.is_ok(), "{:?}", reached.err());
            let own = Fixture::spawn("own", std::path::Path::new("/"), How::Plain);
            assert_eq!(own.wait_for("own:"), "own:Ok(())");
        }

        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "android"))] {
                use super::super::local::sandbox::linux::probe_dir;
                use std::path::Path;

                #[test]
                fn a_client_under_a_seccomp_filter_is_refused_and_so_is_a_listener_and_the_caller() {
                    client_is_refused("connect", How::Seccomp, Cause::Seccomp);
                    listener_is_refused("listen", How::Seccomp, Cause::Seccomp);
                    own_is_refused("own", How::Seccomp, Cause::Seccomp);
                }

                #[test]
                fn a_process_in_a_user_namespace_is_refused_as_client_listener_and_caller() {
                    client_is_refused("connect", How::UserNamespace, Cause::UserNamespace);
                    listener_is_refused("listen", How::UserNamespace, Cause::UserNamespace);
                    own_is_refused("own", How::UserNamespace, Cause::UserNamespace);
                }

                #[test]
                fn a_process_in_another_mount_namespace_is_refused_as_client_and_listener() {
                    client_is_refused("connect", How::MountNamespace, Cause::OtherNamespace);
                    listener_is_refused("listen", How::MountNamespace, Cause::OtherNamespace);
                }

                #[test]
                fn a_peer_that_has_gone_before_the_check_is_refused_unreadable() {
                    let dir = dir();
                    let listener = listener(&dir);
                    let path = listener.address().path().to_path_buf();
                    let mut child = Fixture::spawn("connect_exit", &path, How::Plain);
                    child.wait_exit();
                    let refused = accept_within(listener).err().expect("refused");
                    assert!(matches!(refused, LocalRefusal::Peer(PeerRefusal::Unreadable(_))), "{refused:?}");
                    // The probe alone, on the `/proc` directory of a process that has been reaped (a directory that is gone: the
                    // reaped process's ID may be reused by the time it is read, which the hold's rows state).
                    let proc = tempfile::tempdir().unwrap();
                    let path = proc.path().to_path_buf();
                    drop(proc);
                    let gone = probe_dir(&path);
                    assert!(matches!(gone, Err(PeerRefusal::Unreadable(_))), "{gone:?}");
                }

                #[test]
                fn an_unreadable_or_unparsable_file_refuses_and_a_readable_one_is_read() {
                    let proc = tempfile::tempdir().unwrap();
                    let at = |rel: &str| proc.path().join(rel);
                    let status = |text: &str| std::fs::write(at("status"), text).unwrap();
                    let unreadable = |what: &str| {
                        let found = probe_dir(proc.path());
                        assert!(matches!(found, Err(PeerRefusal::Unreadable(_))), "{what}: {found:?}");
                    };
                    unreadable("no files at all");
                    status("Name:\tx\nSeccomp:\t0\nSeccomp_filters:\t0\n");
                    unreadable("no namespace links");
                    std::fs::create_dir(at("ns")).unwrap();
                    std::fs::write(at("ns/user"), "").unwrap();
                    unreadable("no mount namespace link");
                    std::fs::write(at("ns/mnt"), "").unwrap();
                    status("Name:\tx\nSeccomp:\t0\n");
                    unreadable("no task directory (the threads cannot be listed)");
                    let thread = |tid: &str, text: &str| {
                        std::fs::create_dir_all(at(&format!("task/{tid}"))).unwrap();
                        std::fs::write(at(&format!("task/{tid}/status")), text).unwrap();
                    };
                    std::fs::create_dir(at("task")).unwrap();
                    thread("7", "Name:\tx\nSeccomp:\t0\n");
                    assert_eq!(probe_dir(proc.path()).unwrap().seccomp, 0);
                    // A filter on a thread other than the first counts, and the highest mode among the threads is the answer.
                    thread("8", "Name:\ty\nSeccomp:\t2\n");
                    thread("9", "Name:\ty\nSeccomp:\t1\n");
                    assert_eq!(probe_dir(proc.path()).unwrap().seccomp, 2);
                    // A thread with no status file has gone since the listing: nothing of it is left to find.
                    std::fs::create_dir_all(at("task/10")).unwrap();
                    assert_eq!(probe_dir(proc.path()).unwrap().seccomp, 2);
                    // A thread whose status has no mode cannot be judged.
                    thread("11", "Name:\tz\n");
                    unreadable("a thread whose status has no Seccomp line");
                    std::fs::remove_dir_all(at("task/11")).unwrap();
                    status("Name:\tx\nSeccomp_filters:\t3\n");
                    unreadable("a status with no Seccomp line (only Seccomp_filters)");
                    status("Seccomp:\tmaybe\n");
                    unreadable("a Seccomp line that is not a number");
                    status("Name:\tx\nSeccomp:\t2\nSeccomp_filters:\t1\n");
                    let read = probe_dir(proc.path()).unwrap();
                    assert_eq!(read.seccomp, 2);
                    assert!(read.user_namespace.is_some() && read.mount_namespace.is_some());
                    // This process: no seccomp mode is guessed at, and the namespaces are the initial or another's.
                    assert!(probe_dir(Path::new("/proc/self")).is_ok());
                }
            } else {
                #[test]
                fn a_client_under_a_sandbox_profile_is_refused_and_so_is_a_listener_and_the_caller() {
                    client_is_refused("connect", How::Profile, Cause::Profile);
                    listener_is_refused("listen", How::Profile, Cause::Profile);
                    own_is_refused("own", How::Profile, Cause::Profile);
                }

                use super::super::local::sandbox::macos::{AuditToken, Subject, probe_with};
                use std::ffi::{CStr, c_char, c_int, c_void};

                // Stubs that answer with their argument, so that which one was called shows in the answer: the token
                // form with the first word of the token, the process form with the process ID.
                extern "C" fn by_token(token: AuditToken, _: *const c_char, _: c_int) -> c_int {
                    token.0[0] as c_int
                }
                extern "C" fn by_pid(pid: libc::pid_t, _: *const c_char, _: c_int) -> c_int {
                    pid
                }

                /// A resolver that has the symbols whose names are listed.
                fn resolver(names: &'static [&'static str]) -> impl Fn(&CStr) -> *mut c_void {
                    move |name| match name.to_str().unwrap() {
                        found if !names.contains(&found) => std::ptr::null_mut(),
                        "sandbox_check_by_audit_token" => by_token as *mut c_void,
                        _ => by_pid as *mut c_void,
                    }
                }

                fn token(first: u32) -> Subject {
                    Subject::Token { token: AuditToken([first, 0, 0, 0, 0, 0, 0, 0]), pid: 0 }
                }

                /// The probe with a hold that always holds.
                fn probe(resolve: &dyn Fn(&CStr) -> *mut c_void, subject: Subject) -> Result<SandboxFindings, PeerRefusal> {
                    probe_with(resolve, subject, &|| Ok(()))
                }

                #[test]
                fn the_probe_uses_the_token_form_first_and_refuses_what_it_cannot_ask() {
                    let both = resolver(&["sandbox_check_by_audit_token", "sandbox_check"]);
                    // A non-zero answer is a sandbox; zero is none. The process form would have answered the other way.
                    let sandboxed = probe(&both, Subject::Token { token: AuditToken([3, 0, 0, 0, 0, 0, 0, 0]), pid: 0 });
                    assert_eq!(sandboxed.unwrap().cause(), Some(Cause::Profile));
                    let clean = probe(&both, Subject::Token { token: AuditToken([0, 0, 0, 0, 0, 0, 0, 0]), pid: 9 });
                    assert_eq!(clean.unwrap(), SandboxFindings::default());
                    // Without the token form, the process form: its answer (the pid) decides.
                    let by_process = resolver(&["sandbox_check"]);
                    assert_eq!(probe(&by_process, Subject::Token { token: AuditToken([0; 8]), pid: 9 }).unwrap().cause(), Some(Cause::Profile));
                    assert_eq!(probe(&by_process, token(5)).unwrap(), SandboxFindings::default());
                    assert_eq!(probe(&by_process, Subject::Pid(9)).unwrap().cause(), Some(Cause::Profile));
                    // A symbol that is not there refuses: neither, or the process form alone for a process ID.
                    let neither = resolver(&[]);
                    let only_token = resolver(&["sandbox_check_by_audit_token"]);
                    for (found, subject) in [(&neither, token(0)), (&neither, Subject::Pid(0)), (&only_token, Subject::Pid(0))] {
                        assert!(matches!(probe(found, subject), Err(PeerRefusal::Unreadable(_))));
                    }
                    // A failed call refuses, and is not read as "not sandboxed".
                    let failed = probe(&both, token(u32::MAX));
                    assert!(matches!(failed, Err(PeerRefusal::Unreadable(_))), "{failed:?}");
                }
            }
        }
    } else if #[cfg(windows)] {
        use super::super::local::{ProcessHold, judge};
        use std::{ffi::c_void, mem, ptr};
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE},
            Security::SECURITY_CAPABILITIES,
            System::Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
                InitializeProcThreadAttributeList, PROCESS_INFORMATION, STARTUPINFOEXW, TerminateProcess,
                UpdateProcThreadAttribute,
            },
        };

        /// `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`.
        const SECURITY_CAPABILITIES_ATTRIBUTE: usize = 0x0002_0009;
        /// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
        const ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;

        #[link(name = "userenv")]
        unsafe extern "system" {
            fn CreateAppContainerProfile(name: *const u16, display: *const u16, description: *const u16, capabilities: *const c_void, count: u32, sid: *mut *mut c_void) -> i32;
            fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut c_void) -> i32;
            fn DeleteAppContainerProfile(name: *const u16) -> i32;
        }

        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().chain(Some(0)).collect()
        }

        /// The image of the rows' processes: `ping` is in `System32`, which an AppContainer may read and run. They are
        /// created suspended, so they are alive, with their token, until ended, whatever the image would do.
        fn waiting_command() -> Vec<u16> {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            wide(&format!(r"{root}\System32\PING.EXE -n 60 127.0.0.1"))
        }

        /// A process of this user, running in a real AppContainer (`CreateAppContainerProfile` and the
        /// security-capabilities attribute) or, with `container` false, as an ordinary child. Ended and its profile
        /// deleted on drop.
        struct Waiting {
            process: HANDLE,
            pid: u32,
            profile: Option<Vec<u16>>,
        }

        impl Waiting {
            fn start(container: bool) -> Self {
                let name = wide(&format!("gwz.p2.3a.{}.{}", std::process::id(), container));
                let mut command = waiting_command();
                // SAFETY: plain data in, a process handle out; `sid`, the attribute list's buffer and the command line
                // outlive the call that reads them.
                unsafe {
                    let mut info: PROCESS_INFORMATION = mem::zeroed();
                    let mut startup: STARTUPINFOEXW = mem::zeroed();
                    startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
                    let mut buffer = Vec::<u8>::new();
                    let mut flags = CREATE_NO_WINDOW | CREATE_SUSPENDED;
                    let mut capabilities: SECURITY_CAPABILITIES = mem::zeroed();
                    if container {
                        let mut sid = ptr::null_mut();
                        let made = CreateAppContainerProfile(name.as_ptr(), name.as_ptr(), name.as_ptr(), ptr::null(), 0, &mut sid);
                        let derived = if made == ALREADY_EXISTS { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) } else { made };
                        assert_eq!(derived, 0, "an AppContainer profile cannot be made on this host (CreateAppContainerProfile: {made:#x})");
                        capabilities.AppContainerSid = sid;
                        let mut size = 0usize;
                        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
                        buffer.resize(size, 0);
                        startup.lpAttributeList = buffer.as_mut_ptr().cast();
                        assert_ne!(InitializeProcThreadAttributeList(startup.lpAttributeList, 1, 0, &mut size), 0, "InitializeProcThreadAttributeList");
                        assert_ne!(
                            UpdateProcThreadAttribute(
                                startup.lpAttributeList,
                                0,
                                SECURITY_CAPABILITIES_ATTRIBUTE,
                                (&raw const capabilities).cast(),
                                mem::size_of::<SECURITY_CAPABILITIES>(),
                                ptr::null_mut(),
                                ptr::null(),
                            ),
                            0,
                            "UpdateProcThreadAttribute(SECURITY_CAPABILITIES)"
                        );
                        flags |= EXTENDED_STARTUPINFO_PRESENT;
                    }
                    let created = CreateProcessW(ptr::null(), command.as_mut_ptr(), ptr::null(), ptr::null(), 0, flags, ptr::null(), ptr::null(), &startup.StartupInfo, &mut info);
                    let error = std::io::Error::last_os_error();
                    if container {
                        DeleteProcThreadAttributeList(startup.lpAttributeList);
                    }
                    assert_ne!(created, 0, "CreateProcessW (as an AppContainer: {container}): {error}");
                    CloseHandle(info.hThread);
                    Self { process: info.hProcess, pid: info.dwProcessId, profile: container.then_some(name) }
                }
            }
        }

        impl Drop for Waiting {
            fn drop(&mut self) {
                // SAFETY: this value's handle, ended and closed once; the profile is the one it made.
                unsafe {
                    TerminateProcess(self.process, 1);
                    CloseHandle(self.process);
                    if let Some(name) = &self.profile {
                        DeleteAppContainerProfile(name.as_ptr());
                    }
                }
            }
        }

        #[test]
        fn a_real_app_container_process_is_refused_in_both_relations_and_gets_no_policy() {
            let policy = PeerPolicy::current_user().unwrap();
            let boxed = Waiting::start(true);
            for relation in RELATIONS {
                // `judge` is what the listener runs on its client's process ID and the client on its listener's.
                assert_eq!(judge(boxed.pid, &policy, relation).err(), Some(PeerRefusal::AppContainer), "{relation:?}");
            }
            // The same token read through the hold gives no policy to a caller that runs in an AppContainer.
            let hold = ProcessHold::take(boxed.pid).expect("a hold on the AppContainer process");
            assert!(hold.identity().app_container);
            assert_eq!(PeerPolicy::try_from_identity(hold.identity().clone()).err(), Some(LocalRefusal::Own(PeerRefusal::AppContainer)));
            // Control: an ordinary child of this user is accepted.
            let plain = Waiting::start(false);
            for relation in RELATIONS {
                assert!(judge(plain.pid, &policy, relation).is_ok(), "{relation:?}");
            }
        }
    }
}
