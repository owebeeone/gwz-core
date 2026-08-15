//! MAX_PATH fail-closed negative evidence (R2-F evidence map F-5/G-5).
//!
//! The pinned fixtures set `core.longpaths=true`, which masks the real
//! product exposure recorded in the Windows matrix ledger
//! (GwzWindowsMatrix-Classification.md:158-161): staged checked-artifact
//! sources are 173-character `ca1-*.source` names under
//! `.gwz/checked-artifacts`, and an ordinary user without the registry/git
//! opt-in breaches MAX_PATH on any moderately deep workspace. This test
//! runs WITHOUT the pin and asserts the breach fails CLOSED: a typed
//! rejection, zero mutation of the managed set, and a state that resumes
//! cleanly once the breach is removed — never an OS panic, never partial
//! mutation.

use super::support::*;
use super::*;

/// `ca1-<64 hex family>-<64 hex action>-<32 hex identity>.source` — the
/// exact staged-source shape minted by `checked_artifact::authority`
/// (`source_name`, authority.rs:239), measured by the ledger at 173 chars.
fn staged_source_shape_name() -> String {
    let name = format!(
        "ca1-{}-{}-{}.source",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(32)
    );
    assert_eq!(name.len(), 173);
    name
}

#[test]
fn staged_source_beyond_max_path_without_longpaths_fails_closed() {
    // Root long enough that `<root>/.gwz/checked-artifacts/<173>` breaches
    // MAX_PATH (root + 198 > 260), while every fixture-build path stays
    // well below the un-opted limit.
    let fixture = fixture_configured("sha1", None, None, Some(b"handoff marker\n"), false, 120);
    let private = fixture.root.join(".gwz").join("checked-artifacts");
    fs::create_dir_all(&private).unwrap();
    let planted = private.join(staged_source_shape_name());
    assert!(
        planted.as_os_str().len() > 260,
        "fixture root is too shallow to breach MAX_PATH: {}",
        planted.display()
    );
    // Rust's std uses extended-length paths internally, so the staged
    // source itself lands on disk exactly as an interrupted checked step
    // leaves it; the git/libgit2 legs of the preservation flow are the
    // ones bound by MAX_PATH without the core.longpaths opt-in.
    fs::write(&planted, b"staged source residue\n").unwrap();

    let before = exact_snapshot(&fixture);
    let error = match fixture
        .backend
        .prepare_root_preservation_stash(&fixture.root, &fixture.spec)
    {
        Err(error) => error,
        Ok(prepared) => {
            // If preparation stayed inside the bound, the stash sweep is
            // the libgit2 workdir walk the ledger names; it must reject.
            let step = GitRootPreservationPhysicalStep::CreateStash {
                merge_id: "merge_1".into(),
            };
            fixture
                .backend
                .execute_root_preservation_step_checked(
                    &fixture.root,
                    &fixture.spec,
                    &step,
                    &guard(&prepared),
                )
                .expect_err(
                    "a >MAX_PATH staged source without core.longpaths must fail closed, \
                     not silently succeed (F-5: the exposure would be retired — rescope, \
                     do not weaken this assertion)",
                )
        }
    };
    // Typed fail-closed rejection: a ModelError with its code, surfaced to
    // the run log as executed evidence for the F-5 decision.
    println!(
        "max-path fail-closed rejection: code={:?} error={error:?}",
        error.code
    );

    // No partial mutation: the complete managed set is byte-identical and
    // no stash was created; the planted staged source is untouched.
    assert_eq!(exact_snapshot(&fixture), before);
    assert!(
        fixture
            .backend
            .stash_list(&fixture.root)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read(&planted).unwrap(), b"staged source residue\n");

    // Classifiable, resumable state: with the breach removed and the
    // opt-in granted, the identical flow converges from where it stopped.
    fs::remove_file(&planted).unwrap();
    git(&fixture.root, &["config", "core.longpaths", "true"]);
    let guard = guard(&prepare(&fixture));
    normalize(&fixture, &guard);
}
