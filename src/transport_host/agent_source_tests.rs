//! The agent step's refusals reach the user in words, from the runtime's configuration to the open's error (plan
//! steps 3.2b and 3.3): a failure crosses from the endpoint to the driver as a code with no text, so the driver words
//! it from the agent source its runtime chose.
use super::driver_tests::{block_on, common};
use super::*;
use crate::git::endpoint::agent_source::{AgentSource, select};
use crate::git::endpoint::ssh_channel::GitService;
use crate::{RequestMeta, TransportOptions, TransportPlacement};
use std::ffi::OsStr;

/// The error text of an ambient open that needs an agent, for a runtime whose agent is `source`. The server is the
/// loopback fixture; the open ends at its agent step, before any key is offered.
fn open_error(source: AgentSource) -> (String, bool) {
    open_error_within(source, 3000)
}

/// [`open_error`] under the stall limit `io_timeout_ms` (`--ssh-timeout`, in milliseconds).
fn open_error_within(source: AgentSource, io_timeout_ms: u64) -> (String, bool) {
    let fixture = common::SshdFixture::new();
    let home = super::driver_tests::endpoint_home(&fixture);
    let config = SshEndpointConfig::fixture(home, None)
        .with_agent_source(source)
        .with_io_timeout_ms(io_timeout_ms);
    let runtime = TransportRuntime::new(config).unwrap();
    let meta = RequestMeta {
        request_id: "agent-words".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            placement: Some(TransportPlacement::Local),
            ..Default::default()
        }),
        ..Default::default()
    };
    let request = block_on(runtime.request(meta, "fetch".into())).unwrap();
    let result = request.context.open(
        &super::driver_tests::fixture_url(&fixture),
        GitService::UploadPack,
        None,
        Arc::new(|_, _| panic!("an open with no usable agent opens no stream")),
        Arc::new(|_| {}),
    );
    let Err(error) = result else {
        panic!("an open with no usable agent cannot succeed");
    };
    let words = error.to_string();
    let worded = error
        .get_ref()
        .is_some_and(|cause| cause.downcast_ref::<AgentFailure>().is_some());
    block_on(request.finish());
    block_on(runtime.shutdown());
    (words, worded)
}

#[test]
fn a_refused_ssh_auth_sock_is_told_to_the_user_naming_the_variable() {
    let AgentSource::Refused(refusal) = select(Some(OsStr::new(r"\\localhost\pipe\x")), || None)
    else {
        panic!("refused");
    };
    let (words, worded) = open_error(AgentSource::Refused(refusal.clone()));
    assert!(worded, "{words}");
    assert_eq!(words, refusal.to_string());
    assert!(words.contains("SSH_AUTH_SOCK"), "{words}");
}

#[test]
fn a_selected_pageant_that_cannot_be_used_is_told_naming_pageant() {
    // A pin that no window answers to: where Pageant has a channel (Windows) it is Pageant gone, where it has none the
    // source says only that Pageant was selected; `words_tests` pins each text. Either way it names Pageant, says
    // nothing of another agent being tried, and the open ends at the agent step.
    let pin = crate::git::endpoint::agent_pageant::PageantPin::numbered(1, 1);
    let (words, worded) = open_error(AgentSource::Pageant {
        sock_ignored: true,
        pin,
    });
    assert!(worded, "{words}");
    assert!(words.starts_with("Pageant "), "{words}");
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Plan step 3.6's texts end to end, from the open the user starts to the words they are given, with a synthetic
        /// Pageant (a window of this process answering as Pageant does) as the agent.
        mod pageant_words {
            use super::*;
            use crate::git::endpoint::{
                agent_pageant::pin_of,
                pageant_exchange::receiver::{Receiver, Reply},
            };
            use std::time::Duration;

            const NO_KEYS: [u8; 5] = [12, 0, 0, 0, 0];

            fn source(pageant: &Receiver) -> AgentSource {
                AgentSource::Pageant {
                    sock_ignored: false,
                    pin: pin_of(pageant.window).expect("the synthetic window has an owner"),
                }
            }

            /// T6: Pageant's dialog stays open past the stall limit, so the attempt fails at it and the user is told
            /// what Pageant may be waiting for, and the three ways out. The stall limit is the open's `--ssh-timeout`.
            #[test]
            fn a_prompt_that_outlasts_the_stall_limit_is_told_as_text_t6() {
                let prompting = Receiver::start(Reply::Late { delay: Duration::from_secs(3), message: NO_KEYS.to_vec() });
                let (words, worded) = open_error_within(source(&prompting), 500);
                assert!(worded, "{words}");
                let t6 = "Pageant did not answer within 0.5 seconds (the SSH stall limit, set by --ssh-timeout). Pageant may \
                          still be waiting for you to confirm the key or to enter its passphrase. Answer it and run the \
                          command again, raise --ssh-timeout (0 removes the limit), or use --transport native, which waits \
                          for Pageant as gwz 1.0 did.";
                assert!(words.starts_with(t6), "{words}");
                assert!(words[t6.len()..].starts_with(" (attempt "), "each retry asked Pageant again: {words}");
            }

            #[test]
            fn a_pageant_with_no_key_is_told_before_any_key_is_offered() {
                let empty = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
                let (words, worded) = open_error(source(&empty));
                assert!(worded, "{words}");
                assert!(words.starts_with("Pageant has no key this transport can use"), "{words}");
            }

            #[test]
            fn a_pageant_that_went_away_is_told_and_no_other_agent_is_named_as_tried() {
                let gone = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
                let source = source(&gone);
                drop(gone);
                let (words, worded) = open_error(source);
                assert!(worded, "{words}");
                assert!(words.starts_with("Pageant was running when GWZ started"), "{words}");
                assert!(words.contains("does not ask another SSH agent"), "{words}");
            }
        }

        /// A pipe that is not there is the agent not running (B07): the words for the pipe the variable names. The
        /// default pipe's own words, the stopped service's, are the same path with another name (`words_tests`).
        #[test]
        fn a_pipe_that_is_not_there_is_told_to_the_user() {
            let absent = format!(r"\\.\pipe\gwz-absent-agent-{}", std::process::id());
            let pipe = crate::git::endpoint::agent_source::PipeName::parse(OsStr::new(&absent)).unwrap();
            let (words, worded) = open_error(AgentSource::NamedPipe(pipe));
            assert!(worded, "{words}");
            assert_eq!(
                words,
                format!(
                    "The SSH agent pipe SSH_AUTH_SOCK names ({absent}) does not exist, so no agent is serving it. Start \
                     the agent that creates it, or unset SSH_AUTH_SOCK to use the OpenSSH agent's default pipe."
                )
            );
        }

        /// B07 on the host this runs on, with the OpenSSH Authentication Agent service stopped (it is disabled by
        /// default): `SSH_AUTH_SOCK` unset selects the default pipe, which is not there, and the user is told what to
        /// run. Run it with `--ignored` where that holds; it reads the machine's service state, so it is not an
        /// ordinary row.
        #[test]
        #[ignore = "needs the OpenSSH Authentication Agent service stopped on this machine"]
        fn a_stopped_openssh_agent_service_is_told_to_the_user() {
            let (words, worded) = open_error(AgentSource::capture(None));
            assert!(worded, "{words}");
            assert_eq!(
                words,
                "The OpenSSH Authentication Agent service (ssh-agent) is not running. As administrator run: \
                 Set-Service ssh-agent -StartupType Automatic; Start-Service ssh-agent. Then add a key with ssh-add. \
                 To use Pageant instead, load the key into Pageant."
            );
        }
    }
}
