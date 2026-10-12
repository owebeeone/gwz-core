//! Plan step 3.6's texts: what the user is told when Pageant, the agent a runtime selected, does not answer, is gone, or
//! holds no key (the design's section 14 item 13, text T6, and section 5). The words are the host's, made from the
//! source and the failure's code, cause and facts; the wire carries no text.
use super::AgentSource;
use crate::git::endpoint::agent_pageant::PageantPin;
use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode, Facts, Failure, SetupFailureCause};

fn pageant() -> AgentSource {
    AgentSource::Pageant {
        sock_ignored: false,
        pin: PageantPin::numbered(1, 2),
    }
}

/// A failure of the agent step: marked by the method its facts name, as the endpoint marks it.
fn failed(code: ErrorCode, cause: Option<SetupFailureCause>, offered: bool) -> Failure {
    Failure {
        code,
        setup_cause: cause,
        facts: Some(Facts {
            method: AuthMethod::SshAgent,
            credential_offered: offered,
            ..Facts::default()
        }),
        ..Failure::default()
    }
}

fn stall() -> Failure {
    failed(ErrorCode::Timeout, Some(SetupFailureCause::Stall), true)
}

/// Text T6 of the design's section 14 item 13, for a stall limit of 9 seconds, as the design writes it.
const T6: &str = "Pageant did not answer within 9 seconds (the SSH stall limit, set by --ssh-timeout). Pageant may still \
                  be waiting for you to confirm the key or to enter its passphrase. Answer it and run the command \
                  again, raise --ssh-timeout (0 removes the limit), or use --transport native, which waits for Pageant \
                  as gwz 1.0 did.";

#[test]
fn a_request_that_outlasts_the_stall_limit_is_text_t6_with_the_limit_filled_in() {
    assert_eq!(
        pageant().pageant_reason(&stall(), 9_000, None).as_deref(),
        Some(T6)
    );
    // The number is the open's `--ssh-timeout`.
    let said = pageant().pageant_reason(&stall(), 20_000, None).unwrap();
    assert!(
        said.starts_with("Pageant did not answer within 20 seconds (the SSH stall limit"),
        "{said}"
    );
    let said = pageant().pageant_reason(&stall(), 1_000, None).unwrap();
    assert!(
        said.starts_with("Pageant did not answer within 1 second (the SSH stall limit"),
        "{said}"
    );
    // It says which attempt asked Pageant, and so that each retry asked again.
    let said = pageant()
        .pageant_reason(&stall(), 9_000, Some((4, 4)))
        .unwrap();
    assert_eq!(said, format!("{T6} (attempt 4 of 4)"));
    // It says nothing of another agent: none was tried.
    assert!(!T6.contains("OpenSSH"));
    // A stall limit of zero turns the stall clock off, so a stall cannot be its doing and no text names it.
    assert_eq!(pageant().pageant_reason(&stall(), 0, None), None);
}

#[test]
fn the_setup_budget_and_the_send_cap_are_worded_as_what_ended_the_wait() {
    let budget = failed(ErrorCode::Timeout, Some(SetupFailureCause::Aggregate), true);
    let said = pageant().pageant_reason(&budget, 9_000, None).unwrap();
    assert!(said.contains("setup time ran out"), "{said}");
    assert!(!said.contains("stall limit"), "{said}");
    assert!(
        said.contains("--ssh-timeout (0 removes the limit)"),
        "{said}"
    );
    assert!(said.contains("--transport native"), "{said}");
    // The send's own cap is what is left under `--ssh-timeout 0`: raising the option cannot help, and the text does
    // not suggest it.
    let cap = failed(ErrorCode::Timeout, None, true);
    let said = pageant().pageant_reason(&cap, 0, None).unwrap();
    assert!(
        said.starts_with("Pageant did not answer within 120 seconds"),
        "{said}"
    );
    assert!(said.contains("even with --ssh-timeout 0"), "{said}");
    assert!(said.contains("--transport native"), "{said}");
    assert!(!said.contains("raise"), "{said}");
}

#[test]
fn a_pageant_that_is_gone_says_so_and_that_no_other_agent_was_asked() {
    let gone = failed(
        ErrorCode::Unavailable,
        Some(SetupFailureCause::NotFound),
        true,
    );
    let said = pageant()
        .pageant_reason(&gone, 9_000, Some((1, 4)))
        .unwrap();
    assert!(
        said.contains("window is gone or now belongs to another program"),
        "{said}"
    );
    assert!(said.contains("does not ask another SSH agent"), "{said}");
    assert!(said.ends_with("(attempt 1 of 4)"), "{said}");
    // Before any key, the source's own words for the same cause say the same.
    assert_eq!(
        pageant().unavailable_reason(Some(SetupFailureCause::NotFound)),
        pageant().pageant_reason(&gone, 9_000, None)
    );
}

#[test]
fn a_pageant_with_no_usable_key_says_so_before_any_key_is_offered() {
    let empty = failed(ErrorCode::Authentication, None, false);
    let said = pageant().pageant_reason(&empty, 9_000, None).unwrap();
    assert!(
        said.starts_with("Pageant has no key this transport can use"),
        "{said}"
    );
    // After a key was offered the server's rejection stands, and the generic words with it.
    let rejected = failed(ErrorCode::Authentication, None, true);
    assert_eq!(pageant().pageant_reason(&rejected, 9_000, None), None);
}

#[test]
fn only_pageants_agent_step_has_these_words() {
    let sources = [
        AgentSource::None,
        AgentSource::NamedPipe(super::PipeName::default_pipe()),
    ];
    for source in sources {
        assert_eq!(
            source.pageant_reason(&stall(), 9_000, None),
            None,
            "{source:?}"
        );
    }
    // Another step, or an effect, is not Pageant's.
    let mut other_step = stall();
    other_step.facts.as_mut().unwrap().method = AuthMethod::Gh;
    assert_eq!(pageant().pageant_reason(&other_step, 9_000, None), None);
    let mut effect = stall();
    effect.effect = Effect::Possible;
    assert_eq!(pageant().pageant_reason(&effect, 9_000, None), None);
    let mut no_facts = stall();
    no_facts.facts = None;
    assert_eq!(pageant().pageant_reason(&no_facts, 9_000, None), None);
    // Another failure of the step keeps the generic words.
    let io = failed(ErrorCode::Io, None, true);
    assert_eq!(pageant().pageant_reason(&io, 9_000, None), None);
}
