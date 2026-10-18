//! The words for the ambient (all-keys) agent route when the agent answered and the server took no key.
use super::{AgentKind, AgentSource, PipeName};
use crate::git::endpoint::agent_pageant::PageantPin;
use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode, Facts, Failure};
use std::{ffi::OsStr, path::PathBuf};

const HOST: &str = "git.example.com";

fn unix() -> AgentSource {
    AgentSource::Path(PathBuf::from("agent.sock"))
}

fn pipe() -> AgentSource {
    AgentSource::NamedPipe(PipeName::default_pipe())
}

fn pageant(sock_ignored: bool) -> AgentSource {
    AgentSource::Pageant {
        sock_ignored,
        pin: PageantPin::numbered(1, 1),
    }
}

/// The failure of an ambient agent step that ended in `Authentication`.
fn failed(offered: bool, authenticated: Option<bool>) -> Failure {
    Failure {
        code: ErrorCode::Authentication,
        facts: Some(Facts {
            method: AuthMethod::SshAgent,
            credential_offered: offered,
            authenticated,
            ..Facts::default()
        }),
        ..Failure::default()
    }
}

fn said(source: &AgentSource, failure: &Failure) -> String {
    source
        .ambient_reason(failure, HOST, None)
        .expect("this failure has words of its own")
}

#[test]
fn the_kind_is_what_the_source_reaches() {
    assert_eq!(unix().kind(), Some(AgentKind::Unix));
    assert_eq!(pipe().kind(), Some(AgentKind::OpenSsh));
    assert_eq!(pageant(false).kind(), Some(AgentKind::Pageant));
    assert_eq!(AgentSource::None.kind(), None);
    let AgentSource::Refused(refusal) = super::select(Some(OsStr::new("/tmp/agent")), || None)
    else {
        panic!("refused");
    };
    assert_eq!(AgentSource::Refused(refusal).kind(), None);
}

#[test]
fn keys_the_server_rejected_say_what_was_offered_and_give_the_next_action_per_agent() {
    let rejected = failed(true, Some(false));
    assert_eq!(
        said(&unix(), &rejected),
        "GWZ offered git.example.com every key the SSH agent holds that it can use, and git.example.com accepted \
         none of them. Check that the key you mean to use is added to your account on git.example.com. ssh-add -l \
         lists the keys the agent holds, and ssh-add <the private key file> adds another; then run the command again."
    );
    assert_eq!(
        said(&pipe(), &rejected),
        "GWZ offered git.example.com every key the OpenSSH agent holds that it can use, and git.example.com accepted \
         none of them. Check that the key you mean to use is added to your account on git.example.com. ssh-add -l \
         lists the keys the agent holds, and ssh-add <the private key file> adds another; then run the command again."
    );
    assert_eq!(
        said(&pageant(false), &rejected),
        "GWZ offered git.example.com every key Pageant holds that it can use, and git.example.com accepted none of \
         them. Check that the key you mean to use is added to your account on git.example.com. Pageant's View Keys \
         lists what it holds and Add Key loads another (ssh-add cannot add a key to Pageant); then run the command \
         again. Pageant is the agent GWZ uses because a Pageant window is running, so GWZ did not ask the OpenSSH \
         agent; close Pageant to use it."
    );
    assert!(
        said(&pageant(true), &rejected)
            .contains("a Pageant window is running; SSH_AUTH_SOCK was not consulted, so"),
        "{}",
        said(&pageant(true), &rejected)
    );
}

#[test]
fn an_agent_with_no_usable_key_says_nothing_was_offered_and_how_to_add_one() {
    let empty = failed(false, None);
    assert_eq!(
        said(&unix(), &empty),
        "The SSH agent has no key GWZ can offer git.example.com: it holds no key, holds only key types GWZ does not \
         use, or would not list its keys, so GWZ offered git.example.com nothing. Add a key with ssh-add <the private \
         key file> (ssh-add -l lists what the agent holds, and ssh-add -X unlocks a locked agent), then run the \
         command again."
    );
    assert_eq!(
        said(&pipe(), &empty),
        "The OpenSSH agent has no key GWZ can offer git.example.com: it holds no key, holds only key types GWZ does \
         not use, or would not list its keys, so GWZ offered git.example.com nothing. Add a key with ssh-add <the \
         private key file> (ssh-add -l lists what the agent holds), then run the command again."
    );
}

#[test]
fn pageant_with_no_usable_key_keeps_its_own_words() {
    // `pageant_reason` words it (`NO_KEY_WORDS`), before this route is asked.
    assert_eq!(
        pageant(false).ambient_reason(&failed(false, None), HOST, None),
        None
    );
}

#[test]
fn keys_offered_but_not_rejected_claim_neither_rejection_nor_a_count() {
    // The agent may have declined to sign, or the server listed no algorithm for the key: the last key offered was
    // not rejected by the server, so the words do not say it was.
    let unfinished = failed(true, None);
    assert_eq!(
        said(&unix(), &unfinished),
        "GWZ offered git.example.com the keys the SSH agent holds, but none completed authentication: the agent may \
         have declined to sign (a confirmation asked for with ssh-add -c was declined, or a security key was not \
         touched in time), or git.example.com lists no signature algorithm GWZ can use for the key. Fix that and run \
         the command again."
    );
    assert!(said(&pipe(), &unfinished).contains("the OpenSSH agent holds, but none completed"));
    assert!(said(&pageant(false), &unfinished).contains("the keys Pageant holds"));
    assert!(said(&pageant(false), &unfinished).contains("a Pageant prompt was declined"));
}

#[test]
fn the_attempt_goes_at_the_end_as_for_pageants_words() {
    let words = unix()
        .ambient_reason(&failed(true, Some(false)), HOST, Some((2, 4)))
        .unwrap();
    assert!(words.ends_with(" (attempt 2 of 4)"), "{words}");
}

#[test]
fn only_an_ambient_agent_step_that_ended_in_authentication_is_worded() {
    let base = failed(true, Some(false));
    let facts = |method| Failure {
        facts: Some(Facts {
            method,
            credential_offered: true,
            authenticated: Some(false),
            ..Facts::default()
        }),
        ..base.clone()
    };
    // Another credential's step, another code, a step with effects, no facts, and a source with no agent.
    for other in [
        facts(AuthMethod::SshKey),
        facts(AuthMethod::Gh),
        facts(AuthMethod::None),
        Failure {
            code: ErrorCode::Unavailable,
            ..base.clone()
        },
        Failure {
            code: ErrorCode::Timeout,
            ..base.clone()
        },
        Failure {
            effect: Effect::Possible,
            ..base.clone()
        },
        Failure {
            facts: None,
            ..base.clone()
        },
    ] {
        assert_eq!(unix().ambient_reason(&other, HOST, None), None, "{other:?}");
    }
    assert_eq!(AgentSource::None.ambient_reason(&base, HOST, None), None);
}
