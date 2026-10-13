//! What the user is told when the agent step cannot start (plan steps 3.2b and 3.3; the design's section 14, item 13).
use super::{AgentFault, AgentSource, PipeName};
use crate::git::endpoint::agent_pageant::PageantPin;
use gwz_transport::protocol::SetupFailureCause;
use std::{ffi::OsStr, path::PathBuf};

fn pin() -> PageantPin {
    PageantPin::numbered(1, 1)
}

#[test]
fn the_pageant_message_names_pageant_and_says_the_variable_was_not_consulted() {
    // The words for a Pageant that cannot be used for a cause of no words of its own. A refusal (no cause) and a Pageant
    // that is gone have theirs (`pageant_words_tests`).
    let said = AgentSource::Pageant {
        sock_ignored: true,
        pin: pin(),
    }
    .unavailable_reason(Some(SetupFailureCause::ConnectionRefused))
    .expect("a Pageant that cannot be used is worded");
    assert!(
        said.contains("Pageant is the SSH agent GWZ selected"),
        "{said}"
    );
    assert!(said.contains("SSH_AUTH_SOCK was not consulted"), "{said}");
    assert!(
        said.ends_with("To use the OpenSSH agent instead, close Pageant."),
        "{said}"
    );
}

#[test]
fn the_pageant_message_leaves_the_variable_out_when_none_was_set() {
    let said = AgentSource::Pageant {
        sock_ignored: false,
        pin: pin(),
    }
    .unavailable_reason(Some(SetupFailureCause::ConnectionRefused))
    .unwrap();
    assert!(
        said.starts_with("Pageant is the SSH agent GWZ selected"),
        "{said}"
    );
    assert!(!said.contains("SSH_AUTH_SOCK"), "{said}");
    assert!(
        said.ends_with("To use the OpenSSH agent instead, close Pageant."),
        "{said}"
    );
}

#[test]
fn a_refused_variable_is_worded_by_its_refusal_whatever_the_cause() {
    let AgentSource::Refused(refusal) = super::select(Some(OsStr::new("/tmp/agent")), || None)
    else {
        panic!("refused");
    };
    for cause in [None, Some(SetupFailureCause::NotFound)] {
        assert_eq!(
            AgentSource::Refused(refusal.clone()).unavailable_reason(cause),
            Some(refusal.to_string())
        );
    }
}

#[test]
fn sources_that_keep_the_generic_words_have_none_of_their_own() {
    assert_eq!(AgentSource::None.unavailable_reason(None), None);
    assert_eq!(
        AgentSource::Path(PathBuf::from("agent.sock"))
            .unavailable_reason(Some(SetupFailureCause::NotFound)),
        None
    );
}

fn named(path: &str) -> AgentSource {
    AgentSource::NamedPipe(PipeName::parse(OsStr::new(path)).unwrap())
}

const STOPPED: &str = "The OpenSSH Authentication Agent service (ssh-agent) is not running. As administrator run: \
                       Set-Service ssh-agent -StartupType Automatic; Start-Service ssh-agent. Then add a key with \
                       ssh-add. To use Pageant instead, load the key into Pageant.";

/// The user guide's text for a stopped service, and the design's section 14 item 13 (owed to step 3.3): it names the
/// service and the commands.
#[test]
fn a_default_pipe_that_is_not_there_is_the_stopped_agent_message() {
    let said = AgentSource::NamedPipe(PipeName::default_pipe())
        .unavailable_reason(Some(SetupFailureCause::NotFound))
        .unwrap();
    assert_eq!(said, STOPPED);
    assert!(said.contains("OpenSSH Authentication Agent service (ssh-agent)"));
    assert!(said.contains("Set-Service ssh-agent -StartupType Automatic; Start-Service ssh-agent"));
}

#[test]
fn the_default_pipe_named_by_the_variable_is_the_same_pipe_and_the_same_message() {
    for path in [r"\\.\pipe\openssh-ssh-agent", r"\\.\PIPE\OpenSSH-SSH-Agent"] {
        assert_eq!(
            named(path).unavailable_reason(Some(SetupFailureCause::NotFound)),
            Some(STOPPED.to_owned()),
            "{path}"
        );
    }
}

#[test]
fn a_named_pipe_that_is_not_there_says_the_variable_names_it_and_how_to_use_the_default() {
    let said = named(r"\\.\pipe\my-agent")
        .unavailable_reason(Some(SetupFailureCause::NotFound))
        .unwrap();
    assert_eq!(
        said,
        "The SSH agent pipe SSH_AUTH_SOCK names (\\\\.\\pipe\\my-agent) does not exist, so no agent is serving it. Start \
         the agent that creates it, or unset SSH_AUTH_SOCK to use the OpenSSH agent's default pipe."
    );
    assert!(!said.contains("Set-Service"), "{said}");
}

#[test]
fn a_pipe_that_would_not_open_for_another_reason_is_worded_by_its_name() {
    let said = named(r"\\.\pipe\my-agent")
        .unavailable_reason(None)
        .unwrap();
    assert!(said.contains(r"\\.\pipe\my-agent"), "{said}");
    assert!(said.contains("another user"), "{said}");
}

#[test]
fn a_fault_keeps_the_kinds_the_setup_failure_is_made_from() {
    let kinds = [
        (AgentFault::Refused, std::io::ErrorKind::Unsupported),
        (AgentFault::NoChannel, std::io::ErrorKind::Unsupported),
        // Not found has a cause of its own on the wire, and no retry acts on it.
        (AgentFault::NotRunning, std::io::ErrorKind::NotFound),
        // Never PermissionDenied: that is an authentication failure, and no key was offered.
        (AgentFault::Denied, std::io::ErrorKind::Unsupported),
    ];
    for (fault, kind) in kinds {
        let error = fault.into_error();
        assert_eq!(error.kind(), kind, "{fault:?}");
        assert_eq!(AgentFault::of_error(&error), Some(fault));
        // Nothing of the user's environment is in the error.
        assert!(!error.to_string().contains(r"\\.\pipe"), "{error}");
    }
    assert_eq!(
        AgentFault::of_error(&std::io::ErrorKind::NotFound.into()),
        None
    );
}
