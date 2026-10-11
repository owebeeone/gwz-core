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
    let fixture = common::SshdFixture::new();
    let home = super::driver_tests::endpoint_home(&fixture);
    let config = SshEndpointConfig::fixture(home, None).with_agent_source(source);
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
    let AgentSource::Refused(refusal) = select(Some(OsStr::new(r"\\localhost\pipe\x")), || false)
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
    let (words, worded) = open_error(AgentSource::Pageant { sock_ignored: true });
    assert!(worded, "{words}");
    assert!(
        words.starts_with("Pageant is the SSH agent GWZ selected"),
        "{words}"
    );
    assert!(words.contains("SSH_AUTH_SOCK was not consulted"), "{words}");
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
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
