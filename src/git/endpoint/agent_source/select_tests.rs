//! Plan step 3.2b's rows: the Windows agent source is chosen from an injected "Pageant window visible" probe and the
//! snapshot's `SSH_AUTH_SOCK` alone. Pure, so every platform runs them.
use super::{AgentSource, PipeName, select};
use std::{cell::Cell, ffi::OsStr};

const DEFAULT: &str = r"\\.\pipe\openssh-ssh-agent";

fn chosen(sock: Option<&str>, pageant: bool) -> AgentSource {
    select(sock.map(OsStr::new), || pageant)
}

fn pipe(name: &str) -> AgentSource {
    AgentSource::NamedPipe(PipeName::parse(OsStr::new(name)).unwrap())
}

#[test]
fn a_visible_pageant_beats_the_pipe_the_snapshot_names() {
    assert_eq!(
        chosen(Some(r"\\.\pipe\my-agent"), true),
        AgentSource::Pageant { sock_ignored: true }
    );
}

#[test]
fn a_visible_pageant_with_no_snapshot_pipe_does_not_claim_to_have_ignored_one() {
    assert_eq!(
        chosen(None, true),
        AgentSource::Pageant {
            sock_ignored: false
        }
    );
    assert_eq!(
        chosen(Some(""), true),
        AgentSource::Pageant {
            sock_ignored: false
        }
    );
}

#[test]
fn pageant_is_chosen_before_the_variable_is_looked_at() {
    // A value that would be refused is not consulted when Pageant wins: the variable is not read (U6).
    assert_eq!(
        chosen(Some("/tmp/ssh-abc/agent.123"), true),
        AgentSource::Pageant { sock_ignored: true }
    );
}

#[test]
fn the_pipe_the_snapshot_names_beats_the_default() {
    assert_eq!(
        chosen(Some(r"\\.\pipe\my-agent"), false),
        pipe(r"\\.\pipe\my-agent")
    );
}

#[test]
fn with_neither_pageant_nor_a_variable_the_default_pipe_is_chosen() {
    let source = chosen(None, false);
    assert_eq!(source, pipe(DEFAULT));
    assert_eq!(source, AgentSource::NamedPipe(PipeName::default_pipe()));
    assert_eq!(PipeName::default_pipe().as_str(), DEFAULT);
    assert!(PipeName::default_pipe().is_default());
}

#[test]
fn an_empty_variable_is_an_unset_one() {
    assert_eq!(chosen(Some(""), false), pipe(DEFAULT));
}

#[test]
fn the_pipe_namespace_is_matched_whatever_its_case_and_the_name_is_kept_as_written() {
    for form in [
        r"\\.\PIPE\My-Agent",
        r"\\.\Pipe\My-Agent",
        r"\\.\pipe\My-Agent",
    ] {
        match chosen(Some(form), false) {
            AgentSource::NamedPipe(name) => assert_eq!(name.as_str(), form),
            other => panic!("{form}: {other:?}"),
        }
    }
    let default = PipeName::parse(OsStr::new(r"\\.\PIPE\OpenSSH-SSH-Agent")).unwrap();
    assert!(
        default.is_default(),
        "the default pipe is the same pipe in any case"
    );
    assert!(
        !PipeName::parse(OsStr::new(r"\\.\pipe\openssh-ssh-agent2"))
            .unwrap()
            .is_default()
    );
}

#[test]
fn a_name_may_hold_any_character_but_a_backslash() {
    for form in [
        r"\\.\pipe\a b",
        r"\\.\pipe\agent:1",
        r"\\.\pipe\agent.sock",
        r"\\.\pipe\x-y_z~1",
        r"\\.\pipe\é",
    ] {
        assert!(
            matches!(chosen(Some(form), false), AgentSource::NamedPipe(_)),
            "{form}"
        );
    }
}

#[test]
fn forms_that_are_not_a_local_pipe_are_refused_naming_the_variable() {
    for form in [
        // MinGW and file forms.
        "/tmp/ssh-XXXXXXXX/agent.1234",
        "/c/Users/me/agent.sock",
        r"C:\Users\me\agent.sock",
        r"C:agent",
        "agent.sock",
        r"\agent",
        r"pipe\x",
        // A pipe on another machine, localhost included: no SMB authentication may occur.
        r"\\localhost\pipe\openssh-ssh-agent",
        r"\\LOCALHOST\pipe\x",
        r"\\server\pipe\x",
        r"\\127.0.0.1\pipe\x",
        r"\\?\pipe\x",
        r"\\?\UNC\server\pipe\x",
        r"\\.\UNC\server\pipe\x",
        // Not the pipe namespace, or no name, or a traversal.
        r"\\.\mailslot\x",
        r"\\.\pipe",
        r"\\.\pipe\",
        r"\\.\pipe\.",
        r"\\.\pipe\..",
        r"\\.\pipe\a\b",
        r"\\.\pipe\..\x",
        r"\\.\pipe\x\",
        r"\\.\pipex\y",
        r"\\.\C:\x",
        "//./pipe/x",
        "//server/pipe/x",
        " \\\\.\\pipe\\x",
        "\\\\.\\pipe\\x\0y",
    ] {
        match chosen(Some(form), false) {
            AgentSource::Refused(refusal) => {
                let text = refusal.to_string();
                assert!(text.contains("SSH_AUTH_SOCK"), "{form:?}: {text}");
                assert!(text.contains(r"\\.\pipe\<name>"), "{form:?}: {text}");
            }
            other => panic!("{form:?} was not refused: {other:?}"),
        }
    }
}

#[test]
fn a_name_that_is_not_unicode_is_refused() {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::ffi::OsStrExt;
            let form = OsStr::from_bytes(b"\\\\.\\pipe\\x\xff");
        } else {
            use std::os::windows::ffi::OsStringExt;
            let wide: Vec<u16> = r"\\.\pipe\x".encode_utf16().chain([0xd800]).collect();
            let owned = std::ffi::OsString::from_wide(&wide);
            let form = owned.as_os_str();
        }
    }
    assert!(matches!(
        select(Some(form), || false),
        AgentSource::Refused(_)
    ));
}

#[test]
fn a_name_over_the_pipe_limit_is_refused() {
    let longest = format!(r"\\.\pipe\{}", "a".repeat(256 - r"\\.\pipe\".len()));
    assert_eq!(longest.len(), 256);
    assert!(matches!(
        chosen(Some(&longest), false),
        AgentSource::NamedPipe(_)
    ));
    let over = format!("{longest}a");
    assert!(matches!(
        chosen(Some(&over), false),
        AgentSource::Refused(_)
    ));
}

#[test]
fn the_probe_is_asked_once_and_before_anything_else() {
    let asked = Cell::new(0);
    let source = select(Some(OsStr::new(r"\\.\pipe\my-agent")), || {
        asked.set(asked.get() + 1);
        false
    });
    assert_eq!(asked.get(), 1);
    assert_eq!(source, pipe(r"\\.\pipe\my-agent"));
    let asked = Cell::new(0);
    let source = select(Some(OsStr::new("/tmp/ssh-abc/agent")), || {
        asked.set(asked.get() + 1);
        true
    });
    assert_eq!(asked.get(), 1);
    assert_eq!(source, AgentSource::Pageant { sock_ignored: true });
}

#[test]
fn the_refusal_shows_the_value_it_refused_once_shortened_and_escaped() {
    let AgentSource::Refused(refusal) = chosen(Some("/tmp/a\"b\nc"), false) else {
        panic!("refused");
    };
    let text = refusal.to_string();
    assert!(text.contains(r#""/tmp/a\"b\nc""#), "{text}");
    assert!(!text.contains('\n'), "{text}");
    let long = format!("/{}", "x".repeat(400));
    let AgentSource::Refused(refusal) = chosen(Some(&long), false) else {
        panic!("refused");
    };
    let text = refusal.to_string();
    assert!(text.len() < 700, "{} bytes", text.len());
    assert!(
        text.contains(&format!("/{}...\"", "x".repeat(99))),
        "{text}"
    );
}

#[test]
fn the_refusal_text_says_what_is_accepted_and_how_to_go_on() {
    let AgentSource::Refused(refusal) = chosen(Some(r"\\localhost\pipe\x"), false) else {
        panic!("refused");
    };
    assert_eq!(
        refusal.to_string(),
        "SSH_AUTH_SOCK is set to \"\\\\localhost\\pipe\\x\", which GWZ cannot use as an SSH agent on Windows. GWZ \
         accepts only a pipe on this computer, in the form \\\\.\\pipe\\<name>: a pipe on another machine \
         (\\\\server\\pipe\\..., including \\\\localhost\\pipe\\...), a Git Bash socket path and a file path are \
         refused. Unset SSH_AUTH_SOCK to use the OpenSSH agent's default pipe (\\\\.\\pipe\\openssh-ssh-agent), or \
         set it to the pipe your agent serves."
    );
}
