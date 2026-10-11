//! `tag_create` and `tag_delete`: native on one repository, the Git CLI on an
//! identical twin.

use super::support::*;
use crate::model::ErrorCode;

#[test]
fn lightweight_and_annotated_tags_match_git() {
    hermetic!();
    let twin = twin(seeded);
    assert!(matches!(twin.tag_create("light", None), Outcome::Done(_)));
    assert!(matches!(
        twin.tag_create("annotated", Some("release")),
        Outcome::Done(_)
    ));
    assert!(matches!(
        twin.tag_create("nested/name-1.0_rc+1", Some("nested")),
        Outcome::Done(_)
    ));
    assert_eq!(
        git(&twin.native, &["cat-file", "-t", "refs/tags/light"]),
        "commit"
    );
    assert_eq!(
        git(&twin.native, &["cat-file", "-t", "refs/tags/annotated"]),
        "tag"
    );
}

#[test]
fn tag_messages_are_cleaned_like_git() {
    hermetic!();
    let twin = twin(seeded);
    let messages = [
        "plain",
        "no newline",
        "\n  body  \n# comment\nnext  \n\n\n\nlast line\n\n",
        "trailing   \n\n\n",
        "multi\nline\n\nparagraph",
        "tab\tinside\r\ncrlf\r\n",
        "unicode \u{e9}\u{4e2d}",
    ];
    for (index, message) in messages.iter().enumerate() {
        assert!(
            matches!(
                twin.tag_create(&format!("m{index}"), Some(message)),
                Outcome::Done(_)
            ),
            "{message:?}"
        );
    }
}

#[test]
fn an_existing_tag_is_refused_like_git() {
    hermetic!();
    let twin = twin(seeded);
    assert!(matches!(twin.tag_create("once", None), Outcome::Done(_)));
    for message in [None, Some("again")] {
        assert_eq!(
            twin.tag_create("once", message),
            Outcome::Failed(ErrorCode::GitCommandFailed)
        );
    }
}

#[test]
fn tag_delete_matches_git_for_loose_packed_and_missing_tags() {
    hermetic!();
    let twin = twin(seeded);
    twin.tag_create("loose", None);
    twin.tag_create("annotated", Some("note"));
    assert!(matches!(twin.tag_delete("loose"), Outcome::Done(_)));
    twin.both(|path| {
        git(path, &["pack-refs", "--all"]);
    });
    assert!(matches!(twin.tag_delete("annotated"), Outcome::Done(_)));
    assert_eq!(
        twin.tag_delete("never-existed"),
        Outcome::Failed(ErrorCode::GitCommandFailed)
    );
}

#[test]
fn tags_on_a_detached_head_match_git() {
    hermetic!();
    let twin = twin(seeded);
    twin.both(|path| {
        git(path, &["checkout", "-q", "--detach"]);
    });
    assert!(matches!(
        twin.tag_create("detached-light", None),
        Outcome::Done(_)
    ));
    assert!(matches!(
        twin.tag_create("detached-note", Some("note")),
        Outcome::Done(_)
    ));
}
