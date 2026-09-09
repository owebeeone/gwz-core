use std::path::Path;

use crate::model::ErrorCode;

use super::*;

// Pure pathspec → owning-repo routing for `gwz add` (GWZAddPlan §2). The workspace is
// nested repos (root + members); each path is owned by the innermost repo containing it.

fn members(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| (*p).to_owned()).collect()
}

fn target(member: Option<&str>, specs: &[&str]) -> StageTarget {
    make_target(member, specs, true)
}

fn fanout(member: Option<&str>, specs: &[&str]) -> StageTarget {
    make_target(member, specs, false)
}

fn make_target(member: Option<&str>, specs: &[&str], explicit: bool) -> StageTarget {
    StageTarget {
        member_path: member.map(str::to_owned),
        pathspecs: specs.iter().map(|s| (*s).to_owned()).collect(),
        explicit,
    }
}

fn resolve(root: &str, mems: &[&str], cwd: &str, specs: &[&str], all: bool) -> Vec<StageTarget> {
    let specs: Vec<String> = specs.iter().map(|s| (*s).to_owned()).collect();
    resolve_stage_targets(Path::new(root), &members(mems), Path::new(cwd), &specs, all).unwrap()
}

#[test]
fn routes_pathspec_into_owning_member() {
    let got = resolve("/ws", &["gwz-cli"], "/ws", &["gwz-cli/src/foo.rs"], false);
    assert_eq!(got, vec![target(Some("gwz-cli"), &["src/foo.rs"])]);
}

#[test]
fn routes_relative_to_cwd_inside_member() {
    let got = resolve("/ws", &["gwz-cli"], "/ws/gwz-cli/src", &["foo.rs"], false);
    assert_eq!(got, vec![target(Some("gwz-cli"), &["src/foo.rs"])]);
}

#[test]
fn routes_root_level_path_to_root_repo() {
    let got = resolve("/ws", &["gwz-cli"], "/ws", &["gwz.conf/gwz.yml"], false);
    assert_eq!(got, vec![target(None, &["gwz.conf/gwz.yml"])]);
}

#[test]
fn groups_cross_member_pathspecs_sorted() {
    let got = resolve(
        "/ws",
        &["gwz-cli", "gwz-core"],
        "/ws",
        &["gwz-core/b.rs", "gwz-cli/a.rs"],
        false,
    );
    assert_eq!(
        got,
        vec![
            target(Some("gwz-cli"), &["a.rs"]),
            target(Some("gwz-core"), &["b.rs"]),
        ]
    );
}

#[test]
fn member_root_itself_stages_everything_in_member() {
    let got = resolve("/ws", &["gwz-cli"], "/ws", &["gwz-cli"], false);
    assert_eq!(got, vec![target(Some("gwz-cli"), &["."])]);
}

#[test]
fn dot_at_workspace_root_spans_all_repos() {
    // D2: `gwz add .` at the workspace root stages the root repo AND every member.
    let got = resolve("/ws", &["gwz-cli", "gwz-core"], "/ws", &["."], false);
    assert_eq!(
        got,
        vec![
            target(None, &["."]),
            fanout(Some("gwz-cli"), &["."]),
            fanout(Some("gwz-core"), &["."]),
        ]
    );
}

#[test]
fn dot_inside_member_stays_in_that_member() {
    let got = resolve(
        "/ws",
        &["gwz-cli", "gwz-core"],
        "/ws/gwz-cli",
        &["."],
        false,
    );
    assert_eq!(got, vec![target(Some("gwz-cli"), &["."])]);
}

#[test]
fn all_flag_targets_root_and_every_member() {
    let got = resolve("/ws", &["gwz-cli", "gwz-core"], "/ws", &[], true);
    assert_eq!(
        got,
        vec![
            fanout(None, &["."]),
            fanout(Some("gwz-cli"), &["."]),
            fanout(Some("gwz-core"), &["."]),
        ]
    );
}

#[test]
fn innermost_member_wins_for_nested_members() {
    let got = resolve(
        "/ws",
        &["sub", "sub/nested"],
        "/ws",
        &["sub/nested/x.rs"],
        false,
    );
    assert_eq!(got, vec![target(Some("sub/nested"), &["x.rs"])]);
}

#[test]
fn similar_prefix_does_not_false_match() {
    // "gwz-cli" must not capture a sibling "gwz-client" path (component-wise match).
    let got = resolve("/ws", &["gwz-cli"], "/ws", &["gwz-client/a.rs"], false);
    assert_eq!(got, vec![target(None, &["gwz-client/a.rs"])]);
}

#[test]
fn parent_relative_pathspec_crosses_members() {
    let got = resolve(
        "/ws",
        &["gwz-cli", "gwz-core"],
        "/ws/gwz-cli",
        &["../gwz-core/b.rs"],
        false,
    );
    assert_eq!(got, vec![target(Some("gwz-core"), &["b.rs"])]);
}

#[test]
fn path_outside_workspace_is_path_escape_error() {
    let err = resolve_stage_targets(
        Path::new("/ws"),
        &members(&["gwz-cli"]),
        Path::new("/ws"),
        &["../outside".to_owned()],
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::PathEscape);
}

#[test]
fn nothing_specified_is_invalid_request() {
    let err = resolve_stage_targets(
        Path::new("/ws"),
        &members(&["gwz-cli"]),
        Path::new("/ws"),
        &[],
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[test]
        fn windows_routing_unifies_dos_and_verbatim_prefixes_for_existing_and_deleted_operands() {
            let root = Path::new(r"\\?\C:\workspace");
            let caller = Path::new(r"C:\workspace");
            let member_paths = members(&["member"]);

            let ordinary_existing = route_pathspec(
                root,
                &member_paths,
                caller,
                r"C:\workspace\member\existing.txt",
            )
            .unwrap();
            assert_eq!(ordinary_existing, RoutedPathspec {
                member_path: Some("member".to_owned()),
                pathspec: "existing.txt".to_owned(),
            });

            let deleted = route_pathspec(
                root,
                &member_paths,
                caller,
                r"C:\workspace\deleted.txt",
            )
            .unwrap();
            assert_eq!(deleted, RoutedPathspec {
                member_path: None,
                pathspec: "deleted.txt".to_owned(),
            });

            let verbatim = route_pathspec(
                Path::new(r"C:\workspace"),
                &member_paths,
                caller,
                r"\\?\C:\workspace\member\verbatim.txt",
            )
            .unwrap();
            assert_eq!(verbatim, RoutedPathspec {
                member_path: Some("member".to_owned()),
                pathspec: "verbatim.txt".to_owned(),
            });
        }

        #[test]
        fn windows_routing_accepts_a_dos_caller_under_a_verbatim_workspace_root() {
            let routed = route_pathspec(
                Path::new(r"\\?\C:\workspace"),
                &members(&["member"]),
                Path::new(r"c:\workspace\member"),
                "relative.txt",
            )
            .unwrap();
            assert_eq!(routed, RoutedPathspec {
                member_path: Some("member".to_owned()),
                pathspec: "relative.txt".to_owned(),
            });
        }
    }
}
