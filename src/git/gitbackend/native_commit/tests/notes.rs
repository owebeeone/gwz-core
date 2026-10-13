//! The reason a commit or tag ran the Git CLI reaches the reader the caller
//! gave the backend, and only then (the CLI gives one for `--verbose`).

use std::sync::{Arc, Mutex};

use super::support::*;
use crate::git::gitbackend::*;

fn collecting(lab: &Lab) -> (Git2Backend, Arc<Mutex<Vec<String>>>) {
    let told = Arc::new(Mutex::new(Vec::new()));
    let kept = told.clone();
    let backend = lab.backend.with_route_notes(RouteNotes::new(move |line| {
        kept.lock().unwrap().push(line.to_owned())
    }));
    (backend, told)
}

#[test]
fn a_commit_that_runs_git_says_why() {
    hermetic!();
    let lab = lab();
    let marker = lab.hook("pre-commit");
    lab.stage_change();
    let (backend, told) = collecting(&lab);
    backend.commit(&lab.path, "with a hook", false).unwrap();
    assert!(marker.exists(), "the hook ran, so Git did");
    let lines = told.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("note: commit in "), "{lines:?}");
    assert!(
        lines[0].contains(&lab.path.display().to_string()),
        "{lines:?}"
    );
    assert!(
        lines[0].contains("ran the Git CLI (a hook would run: pre-commit)"),
        "{lines:?}"
    );
}

#[test]
fn a_native_commit_and_tag_say_nothing() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    let (backend, told) = collecting(&lab);
    backend.commit(&lab.path, "native", false).unwrap();
    backend
        .tag_create(&lab.path, "v1", Some("m"), false)
        .unwrap();
    backend.tag_delete(&lab.path, "v1").unwrap();
    assert_eq!(told.lock().unwrap().clone(), Vec::<String>::new());
}

#[test]
fn commit_all_and_tag_say_why_they_ran_git() {
    hermetic!();
    let lab = lab();
    lab.dirty_change();
    let (backend, told) = collecting(&lab);
    backend.commit(&lab.path, "all", true).unwrap();
    lab.config("tag.gpgSign", "false");
    lab.config("core.fsync", "committed");
    backend.tag_create(&lab.path, "v1", None, false).unwrap();
    backend.tag_delete(&lab.path, "v1").unwrap();
    let lines = told.lock().unwrap().clone();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].contains("commit in ") && lines[0].contains("commit -a: Git always runs it"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("note: tag in ") && lines[1].contains("core.fsync"),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("note: tag delete in ") && lines[2].contains("core.fsync"),
        "{lines:?}"
    );
}

#[test]
fn a_backend_without_a_reader_runs_git_silently() {
    hermetic!();
    let lab = lab();
    lab.hook("pre-commit");
    lab.stage_change();
    lab.backend.commit(&lab.path, "no reader", false).unwrap();
}
