//! Servers whose channels end slowly, badly or never, for the tests of the
//! background close (`dev-docs/GwzTransportSshBackgroundCloseDesign.md` §10).
//!
//! Each is [`SshdFixture`]'s loopback `sshd` with a forced command: a script
//! that runs the requested Git service and then ends its channel in its own
//! way. A request for a path that contains `refused` is answered as a hosted
//! Git server answers a repository it will not serve.
use super::{
    helper_script::write_helper_script, ssh_fixture::SshdFixture, stream_io::BlockingStream,
};
use std::{fs, io::Read, time::Duration};

const SCRIPT: &str = "close-script.sh";
const CLOSE_RELEASE: &str = "close-release";

/// Every channel of the server runs `body`, in which `$SSH_ORIGINAL_COMMAND`
/// is the Git service to run.
fn forced(body: &str) -> SshdFixture {
    let fixture = SshdFixture::new();
    let script = fixture.temp.path().join(SCRIPT);
    write_helper_script(
        &script,
        &format!(
            "case \"$SSH_ORIGINAL_COMMAND\" in\n*refused*) echo 'ERROR: Repository not found.' >&2; exit 1;;\nesac\n{body}"
        ),
    );
    fixture.force_command(&script);
    fixture
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
         while [ ! -e \"${{0%/*}}/{CLOSE_RELEASE}\" ] && [ \"$i\" -lt 1000 ]; do\n\
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

/// The service exits and its output closes, and then the server's own session
/// dies, as a server that drops the connection while the channel is closing.
pub(crate) fn dropped_close_fixture() -> SshdFixture {
    forced(
        "eval \"$SSH_ORIGINAL_COMMAND\"\nexec 1>&- 2>&-\n\
         pid=$$\n\
         while [ \"$pid\" -gt 1 ]; do\n\
         pid=$(ps -o ppid= -p \"$pid\" | tr -d ' ')\n\
         case \"$(ps -o comm= -p \"$pid\")\" in\n\
         *sshd*) kill -9 \"$pid\"; break;;\n\
         esac\n\
         done\n\
         sleep 5\n",
    )
}

/// The service exits and its output closes, and the channel then never
/// closes: the server's process does not exit.
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
            lines.push_str(&format!(
                "command=\"{}\" {}\n",
                script.display(),
                public.trim()
            ));
        }
        fs::write(self.temp.path().join("authorized_keys"), lines).unwrap();
    }

    /// A second identity the server trusts, as a key file in its directory.
    /// The server must be one of this module's, whose script it keeps trusting.
    pub(crate) fn second_identity(&self) -> std::path::PathBuf {
        let key = self.temp.path().join("second_ed25519");
        super::ssh_fixture::run_keygen(&key);
        self.force_command(&self.temp.path().join(SCRIPT));
        key
    }
}
