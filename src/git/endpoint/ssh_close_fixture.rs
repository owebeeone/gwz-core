//! Servers whose channels end slowly, badly or never, for the tests of the
//! background close (`dev-docs/GwzTransportSshBackgroundCloseDesign.md` §10).
//!
//! Each is [`SshdFixture`]'s loopback `sshd` with a forced command: a script
//! that runs the requested Git service and then ends its channel in its own
//! way. A request for a path that contains `refused` is answered as a hosted
//! Git server answers a repository it will not serve.
//!
//! On Unix the script is a forced command in `authorized_keys`. On Windows the server's own forced command
//! (`fixture_host`) sources the script from the fixture's directory when it is there, whichever default shell the
//! host has.
use super::{fixture_host, ssh_fixture::SshdFixture, stream_io::BlockingStream};
use std::{fs, io::Read, time::Duration};

const SCRIPT: &str = fixture_host::FORCED_SCRIPT;
const CLOSE_RELEASE: &str = "close-release";

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The release file as the script names it: beside the script, which is `$0`.
        const RELEASE_FILE: &str = "${0%/*}/close-release";
    } else {
        /// The release file as the script names it: in the working directory, which is the fixture's.
        const RELEASE_FILE: &str = "close-release";
    }
}

/// Every channel of the server runs `body`, in which `$SSH_ORIGINAL_COMMAND`
/// is the Git service to run.
fn forced(body: &str) -> SshdFixture {
    let fixture = SshdFixture::new_forced();
    let script = fixture.temp.path().join(SCRIPT);
    install(
        &script,
        &format!(
            "case \"$SSH_ORIGINAL_COMMAND\" in\n*refused*) echo 'ERROR: Repository not found.' >&2; exit 1;;\nesac\n{body}"
        ),
    );
    fixture.force_command(&script);
    fixture
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Writes `body` as the executable script `path`, run once as a warm-up (`helper_script`).
        fn install(path: &std::path::Path, body: &str) {
            super::helper_script::write_helper_script(path, body);
        }
    } else {
        /// Writes `body` as the fixture's forced script, which the session sources in place of the client's command
        /// (`fixture_host::FORCED_SCRIPT`).
        fn install(path: &std::path::Path, body: &str) {
            fs::write(path, body).unwrap();
        }
    }
}

fn seconds(delay: Duration) -> String {
    format!("{:.3}", delay.as_secs_f64())
}

/// The service exits, its output closes, and the channel closes `delay` later:
/// the server's exit status and CHANNEL_CLOSE are late, its EOF is not.
pub(crate) fn delayed_close_fixture(delay: Duration) -> SshdFixture {
    forced(&format!(
        "eval \"$SSH_ORIGINAL_COMMAND\"\nstatus=$?\nexec 1>&- 2>&-\nsleep {}\nexit $status\n",
        seconds(delay)
    ))
}

/// The service exits and its output closes, and the channel's exit status and
/// CHANNEL_CLOSE wait for the returned file to exist: the test, not a clock,
/// ends the close. The wait is bounded (10 s) so that a test that fails before
/// it makes the file leaves no script running.
pub(crate) fn gated_close_fixture() -> (SshdFixture, std::path::PathBuf) {
    let fixture = forced(&format!(
        "eval \"$SSH_ORIGINAL_COMMAND\"\nstatus=$?\nexec 1>&- 2>&-\ni=0\n\
         while [ ! -e \"{RELEASE_FILE}\" ] && [ \"$i\" -lt 1000 ]; do\n\
         sleep 0.01\ni=$((i+1))\ndone\nexit $status\n"
    ));
    let release = fixture.temp.path().join(CLOSE_RELEASE);
    (fixture, release)
}

/// The service exits, but the channel's output stays open `delay` longer: the
/// server's EOF, exit status and CHANNEL_CLOSE are all late.
pub(crate) fn delayed_eof_fixture(delay: Duration) -> SshdFixture {
    forced(&format!(
        "eval \"$SSH_ORIGINAL_COMMAND\"\nstatus=$?\nsleep {}\nexit $status\n",
        seconds(delay)
    ))
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// What the script runs once the service has exited and its output has closed: the walk up to the server's
        /// own session process uses `ps`, and `kill -9` ends it.
        fn dropped_tail() -> String {
            "pid=$$\n\
             while [ \"$pid\" -gt 1 ]; do\n\
             pid=$(ps -o ppid= -p \"$pid\" | tr -d ' ')\n\
             case \"$(ps -o comm= -p \"$pid\")\" in\n\
             *sshd*) kill -9 \"$pid\"; break;;\n\
             esac\n\
             done\n\
             sleep 5\n"
                .to_owned()
        }
    } else {
        use super::fixture_helper::{self, Mode};

        /// The same with the native helper, which ends the server's session processes by exact id
        /// (`fixture_helper`).
        fn dropped_tail() -> String {
            format!("{}\n", fixture_helper::script_line(Mode::DropSession))
        }
    }
}

/// The service exits and its output closes, and then the server's own session
/// dies, as a server that drops the connection while the channel is closing.
pub(crate) fn dropped_close_fixture() -> SshdFixture {
    forced(&format!(
        "eval \"$SSH_ORIGINAL_COMMAND\"\nexec 1>&- 2>&-\n{}",
        dropped_tail()
    ))
}

/// The service exits and its output closes, and the channel then never
/// closes: the server's process does not exit. (On Windows the output does not end until the process does; see
/// the rows that stay Unix in `ssh_tests`.)
pub(crate) fn stuck_close_fixture() -> SshdFixture {
    forced("eval \"$SSH_ORIGINAL_COMMAND\"\nexec 1>&- 2>&-\nexec sleep 20\n")
}

/// The server never runs the service and never ends its output.
pub(crate) fn silent_fixture() -> SshdFixture {
    forced("exec sleep 20\n")
}

/// Reads one pkt-line advertisement through its closing flush-pkt.
pub(crate) fn read_advertisement(stream: &mut BlockingStream) {
    loop {
        let mut length = [0_u8; 4];
        stream.read_exact(&mut length).unwrap();
        let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
        if length == 0 {
            return;
        }
        let mut line = vec![0_u8; length - 4];
        stream.read_exact(&mut line).unwrap();
    }
}

impl SshdFixture {
    /// Every key the fixture trusts runs `script` in place of the requested
    /// command; the requested command is left in `$SSH_ORIGINAL_COMMAND`.
    /// On Windows the server already forces the script (see the module), so this only lists the keys.
    pub(crate) fn force_command(&self, script: &std::path::Path) {
        let mut lines = String::new();
        let mut keys: Vec<_> = fs::read_dir(self.temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "pub"))
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name != "host_ed25519.pub")
            })
            .collect();
        keys.sort();
        for key in keys {
            let public = fs::read_to_string(key).unwrap();
            lines.push_str(&authorized_line(script, public.trim()));
        }
        fs::write(self.temp.path().join("authorized_keys"), lines).unwrap();
    }

    /// A second identity the server trusts, as a key file in its directory.
    /// The server must be one of this module's, whose script it keeps trusting.
    pub(crate) fn second_identity(&self) -> std::path::PathBuf {
        let key = self.temp.path().join("second_ed25519");
        super::ssh_fixture::run_fixture_key(&key);
        self.force_command(&self.temp.path().join(SCRIPT));
        key
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The `authorized_keys` line that trusts `public` and forces `script`.
        fn authorized_line(script: &std::path::Path, public: &str) -> String {
            format!("command=\"{}\" {public}\n", script.display())
        }
    } else {
        /// The `authorized_keys` line that trusts `public`; the server forces the script itself.
        fn authorized_line(_script: &std::path::Path, public: &str) -> String {
            format!("{public}\n")
        }
    }
}
