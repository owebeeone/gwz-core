// Remote-scoped identity primitives: a remote the repository does not have is
// `MissingRemote`, as every other remote lookup answers (`find_remote`), never
// the Git failure behind it.

use crate::model::ErrorCode;

use super::*;

#[test]
fn identity_primitives_answer_missing_remote_for_a_remote_the_repository_lacks() {
    let temp = TempDir::new("identity-missing-remote");
    let backend = Git2Backend::without_credential_helpers();
    let path = temp.path().join("repo");
    backend.create_repo(&path).unwrap();
    backend
        .add_remote(&path, "origin", "ssh://git@example.invalid/repo")
        .unwrap();

    let answers = [
        ("get", backend.remote_identity(&path, "absent").map(|_| ())),
        (
            "set",
            backend.set_remote_identity(&path, "absent", Some("/keys/id")),
        ),
        ("unset", backend.set_remote_identity(&path, "absent", None)),
        (
            "fetch check",
            backend.validate_remote_identity(&path, "absent", false),
        ),
        (
            "push check",
            backend.validate_remote_identity(&path, "absent", true),
        ),
    ];
    // Every primitive is checked before any assertion, so a failure names all
    // of the wrong answers at once.
    let wrong = answers
        .into_iter()
        .filter_map(|(operation, answer)| match answer {
            Err(error)
                if error.code == ErrorCode::MissingRemote
                    && error.message == "missing remote 'absent'" =>
            {
                None
            }
            other => Some((
                operation,
                other.map_err(|error| (error.code, error.message)),
            )),
        })
        .collect::<Vec<_>>();
    assert!(wrong.is_empty(), "{wrong:?}");
    assert!(
        git2::Repository::open(&path)
            .unwrap()
            .config()
            .unwrap()
            .get_string("remote.absent.gwzSshIdentity")
            .is_err(),
        "nothing is written for a remote the repository lacks"
    );
}
