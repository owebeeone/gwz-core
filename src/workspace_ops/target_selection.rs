use std::collections::BTreeSet;

use crate::artifact::{ManifestArtifact, ManifestMember};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult};
use crate::workspace::MemberPath;

const ROOT: &str = "@root";
const ALL: &str = "@all";
const DEFAULT: &str = "@default";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandDefaultTargets {
    All,
    Members,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootSelectionPolicy {
    Allow,
    SupportedMembers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectedTarget<'a> {
    Root,
    Member(&'a ManifestMember),
}

/// Exhaustive action policy: adding a wire action requires a selection decision.
/// Suboperations retain this scope; lifecycle handlers additionally validate
/// their frozen participants and whether selectors are meaningful at that step.
fn action_policy(
    action: crate::ActionKind,
) -> Option<(CommandDefaultTargets, RootSelectionPolicy, &'static str)> {
    use crate::ActionKind as A;
    use CommandDefaultTargets::{All, Members};
    use RootSelectionPolicy::{Allow, SupportedMembers};
    Some(match action {
        A::Status => (All, Allow, "status"),
        A::Diff => (All, Allow, "diff"),
        A::Log => (All, Allow, "log"),
        A::Commit => (All, Allow, "commit"),
        A::Stage => (All, Allow, "stage"),
        A::Push => (All, Allow, "push"),
        A::PullHead => (All, Allow, "pull"),
        A::Merge => (All, Allow, "merge"),
        A::Ls => (Members, Allow, "ls"),
        A::Forall => (Members, Allow, "forall"),
        A::Branch => (Members, Allow, "branch"),
        A::Tag => (Members, Allow, "tag"),
        A::Stash => (Members, Allow, "stash"),
        A::Materialize => (Members, SupportedMembers, "materialize"),
        A::Snapshot => (Members, SupportedMembers, "snapshot"),
        A::Capture => (Members, SupportedMembers, "capture"),
        A::PullSnapshot => (Members, SupportedMembers, "pull snapshot"),
        A::RepoSync => (Members, SupportedMembers, "repo sync"),
        A::CreateWorkspace
        | A::InitFromSources
        | A::AddExistingRepo
        | A::CreateRepo
        | A::CloneWorkspace
        | A::ListSnapshots
        | A::CloneRepoMember
        | A::DetachRepoMember
        | A::AttachRepoMember
        | A::CloneLocalWorkspace
        | A::LocalFamily => return None,
    })
}

pub(crate) fn resolve_action_targets<'a>(
    manifest: &'a ManifestArtifact,
    selection: Option<&crate::Selection>,
    action: crate::ActionKind,
) -> ModelResult<Vec<SelectedTarget<'a>>> {
    let Some((default, root, name)) = action_policy(action) else {
        if has_explicit_target_selection(selection) {
            return Err(invalid(format!(
                "{action:?} operates on a whole workspace and does not accept target selection"
            )));
        }
        return Ok(Vec::new());
    };
    resolve_targets(manifest, selection, default, root).map_err(|mut error| {
        if error.message == "selected command does not support @root" {
            error.message =
                format!("{name} does not support an explicit @root target; select members instead");
        }
        error
    })
}

/// Ordinary and family merge share one participant policy. Explicit includes
/// replace the default; exclusions are applied by the common resolver.
pub(crate) fn resolve_merge_targets<'a>(
    manifest: &'a ManifestArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<SelectedTarget<'a>>> {
    resolve_action_targets(manifest, selection, crate::ActionKind::Merge)
}

pub(crate) fn resolve_locked_action_selection(
    manifest: &ManifestArtifact,
    lock: &crate::artifact::LockArtifact,
    selection: Option<&crate::Selection>,
    action: crate::ActionKind,
) -> ModelResult<Vec<String>> {
    let mut ids = Vec::new();
    let mut root = false;
    for target in resolve_action_targets(manifest, selection, action)? {
        match target {
            SelectedTarget::Root => root = true,
            SelectedTarget::Member(member) => {
                if !lock.members.contains_key(&member.id) {
                    return Err(ModelError::new(
                        ErrorCode::LockNotFound,
                        format!("lock record missing for member '{}'", member.id),
                    ));
                }
                ids.push(member.id.clone());
            }
        }
    }
    if root {
        ids.push(ROOT.to_owned());
    }
    Ok(ids)
}

pub(crate) fn resolve_targets<'a>(
    manifest: &'a ManifestArtifact,
    selection: Option<&crate::Selection>,
    default: CommandDefaultTargets,
    root_policy: RootSelectionPolicy,
) -> ModelResult<Vec<SelectedTarget<'a>>> {
    let normalized = NormalizedSelection::from_protocol(selection);
    let includes = if normalized.include.is_empty() {
        vec![DEFAULT.to_owned()]
    } else {
        normalized.include
    };
    let explicit_root = includes.iter().any(|token| token == ROOT);
    let mut selected = expand_tokens(manifest, default, &includes)?;
    let excluded = expand_tokens(manifest, default, &normalized.exclude)?;

    selected.retain(|target| !excluded.iter().any(|exclude| same_target(target, exclude)));

    if root_policy == RootSelectionPolicy::SupportedMembers
        && explicit_root
        && selected
            .iter()
            .any(|target| matches!(target, SelectedTarget::Root))
    {
        return Err(invalid("selected command does not support @root"));
    }

    if root_policy == RootSelectionPolicy::SupportedMembers {
        selected.retain(|target| !matches!(target, SelectedTarget::Root));
    }

    Ok(selected)
}

pub(crate) fn resolve_action_ids(
    manifest: &ManifestArtifact,
    selection: Option<&crate::Selection>,
    action: crate::ActionKind,
) -> ModelResult<Vec<String>> {
    Ok(resolve_action_targets(manifest, selection, action)?
        .iter()
        .map(target_key)
        .collect())
}

pub(crate) fn active_members(manifest: &ManifestArtifact) -> impl Iterator<Item = &ManifestMember> {
    manifest.members.iter().filter(|member| member.active)
}

pub(crate) fn has_explicit_target_selection(selection: Option<&crate::Selection>) -> bool {
    selection.is_some_and(|selection| {
        selection.all == Some(true)
            || !selection.member_ids.is_empty()
            || !selection.paths.is_empty()
            || !selection.targets.is_empty()
            || !selection.exclude_targets.is_empty()
    })
}

fn expand_tokens<'a>(
    manifest: &'a ManifestArtifact,
    default: CommandDefaultTargets,
    tokens: &[String],
) -> ModelResult<Vec<SelectedTarget<'a>>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for token in tokens {
        for target in expand_token(manifest, default, token)? {
            let key = target_key(&target);
            if seen.insert(key) {
                out.push(target);
            }
        }
    }
    Ok(out)
}

fn expand_token<'a>(
    manifest: &'a ManifestArtifact,
    default: CommandDefaultTargets,
    token: &str,
) -> ModelResult<Vec<SelectedTarget<'a>>> {
    match token {
        ROOT => Ok(vec![SelectedTarget::Root]),
        ALL => Ok(all_targets(manifest)),
        DEFAULT => Ok(match default {
            CommandDefaultTargets::All => all_targets(manifest),
            CommandDefaultTargets::Members => member_targets(manifest),
        }),
        token if token.starts_with('@') => {
            Err(invalid(format!("unknown target selector '{token}'")))
        }
        token => Ok(vec![SelectedTarget::Member(resolve_member_token(
            manifest, token,
        )?)]),
    }
}

fn all_targets(manifest: &ManifestArtifact) -> Vec<SelectedTarget<'_>> {
    std::iter::once(SelectedTarget::Root)
        .chain(member_targets(manifest))
        .collect()
}

fn member_targets(manifest: &ManifestArtifact) -> Vec<SelectedTarget<'_>> {
    active_members(manifest)
        .map(SelectedTarget::Member)
        .collect()
}

fn resolve_member_token<'a>(
    manifest: &'a ManifestArtifact,
    token: &str,
) -> ModelResult<&'a ManifestMember> {
    if token.starts_with("mem_") {
        MemberId::parse_str(token)?;
        return find_member_by_id(manifest, token);
    }
    if let Ok(member) = find_member_by_id(manifest, token) {
        return Ok(member);
    }
    MemberPath::parse(token)?;
    find_member_by_path(manifest, token)
}

pub(crate) fn find_member_by_id<'a>(
    manifest: &'a ManifestArtifact,
    member_id: &str,
) -> ModelResult<&'a ManifestMember> {
    let mut matches = manifest
        .members
        .iter()
        .filter(|member| member.id == member_id);
    let member = matches
        .next()
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member id not found"))?;
    if matches.next().is_some() {
        return Err(invalid("member id selection is ambiguous"));
    }
    require_active(member)?;
    Ok(member)
}

pub(crate) fn find_member_by_path<'a>(
    manifest: &'a ManifestArtifact,
    path: &str,
) -> ModelResult<&'a ManifestMember> {
    let mut active_matches = manifest
        .members
        .iter()
        .filter(|member| member.active && member.path == path);
    if let Some(member) = active_matches.next() {
        if active_matches.next().is_some() {
            return Err(invalid("active member path selection is ambiguous"));
        }
        return Ok(member);
    }

    if manifest
        .members
        .iter()
        .any(|member| !member.active && member.path == path)
    {
        return Err(ModelError::new(
            ErrorCode::MemberInactive,
            "selected member path has only inactive designations",
        ));
    }
    Err(ModelError::new(
        ErrorCode::MemberNotFound,
        "member path not found",
    ))
}

pub(crate) fn require_active(member: &ManifestMember) -> ModelResult<()> {
    if member.active {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::MemberInactive,
            "selected member is inactive",
        ))
    }
}

fn same_target(left: &SelectedTarget<'_>, right: &SelectedTarget<'_>) -> bool {
    target_key(left) == target_key(right)
}

fn target_key(target: &SelectedTarget<'_>) -> String {
    match target {
        SelectedTarget::Root => ROOT.to_owned(),
        SelectedTarget::Member(member) => member.id.clone(),
    }
}

#[derive(Default)]
struct NormalizedSelection {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl NormalizedSelection {
    fn from_protocol(selection: Option<&crate::Selection>) -> Self {
        let Some(selection) = selection else {
            return Self::default();
        };

        let mut include = Vec::new();
        if selection.all == Some(true) {
            include.push(ALL.to_owned());
        }
        include.extend(selection.member_ids.iter().cloned());
        include.extend(selection.paths.iter().cloned());
        include.extend(selection.targets.iter().cloned());

        Self {
            include,
            exclude: selection.exclude_targets.clone(),
        }
    }
}

fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

// Literal lifecycle selectors intentionally have a narrower grammar than target sets.
pub(crate) fn validate_single_literal_selector(
    selection: Option<&crate::Selection>,
    member_id_only: bool,
) -> ModelResult<String> {
    let selection = selection.ok_or_else(|| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            if member_id_only {
                "repo attach requires exactly one literal member id"
            } else {
                "repo detach requires exactly one literal member id or path"
            },
        )
    })?;
    if selection.all == Some(true) || !selection.exclude_targets.is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "repo lifecycle selectors do not support sets or exclusions",
        ));
    }
    if member_id_only && !selection.paths.is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "repo attach requires a literal member id, not a path",
        ));
    }
    let mut selectors = Vec::new();
    selectors.extend(selection.member_ids.iter().cloned());
    selectors.extend(selection.paths.iter().cloned());
    selectors.extend(selection.targets.iter().cloned());
    if selectors.len() != 1 || selectors[0].starts_with('@') {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            if member_id_only {
                "repo attach requires exactly one literal member id"
            } else {
                "repo detach requires exactly one literal member id or path"
            },
        ));
    }
    if member_id_only && !selectors[0].starts_with("mem_") {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "repo attach requires a literal mem_... member id",
        ));
    }
    Ok(selectors.remove(0))
}

// An open merge resolves against its frozen participant vector, not a moving manifest.
pub(super) fn resolve_open_merge_stage_ids(
    record: super::merge::MergeStatusRecordView<'_>,
    selection: &crate::Selection,
) -> ModelResult<Vec<String>> {
    let included = selection
        .member_ids
        .iter()
        .chain(&selection.paths)
        .chain(&selection.targets)
        .collect::<Vec<_>>();
    let excluded = selection.exclude_targets.iter().collect::<Vec<_>>();
    let token_matches = |target_id: &str, token: &str| {
        matches!(token, "@all" | "@default")
            || target_id == token
            || record
                .participants()
                .get(target_id)
                .is_some_and(|participant| {
                    participant.target_kind == super::merge::MergeTargetKind::Member
                        && participant.path == token
                })
    };
    let known = |token: &str| {
        matches!(token, "@all" | "@default")
            || record
                .selected_targets()
                .iter()
                .any(|target_id| token_matches(target_id, token))
    };
    for token in included.iter().chain(&excluded) {
        if !known(token) {
            return Err(ModelError::new(
                ErrorCode::OpenOperation,
                format!(
                    "merge '{}' is open; selected add target '{}' is not a frozen merge participant",
                    record.merge_id(),
                    token
                ),
            ));
        }
    }
    let include_all = selection.all.unwrap_or(false)
        || included.is_empty()
        || included
            .iter()
            .any(|target| matches!(target.as_str(), "@all" | "@default"));
    Ok(record
        .selected_targets()
        .iter()
        .filter_map(|target_id| {
            record.participants().get(target_id)?;
            let selected = include_all
                || included
                    .iter()
                    .any(|target| token_matches(target_id, target));
            let rejected = excluded
                .iter()
                .any(|target| token_matches(target_id, target));
            (selected && !rejected).then(|| target_id.clone())
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use crate::artifact::{ArtifactSourceKind, ManifestArtifact, ManifestMember, WorkspaceHeader};

    use super::*;

    fn manifest() -> ManifestArtifact {
        ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: "ws_test".to_owned(),
            },
            members: vec![
                member("mem_app", "repos/app", true),
                member("mem_lib", "repos/lib", true),
                member("mem_old", "repos/old", false),
            ],
        }
    }

    fn member(id: &str, path: &str, active: bool) -> ManifestMember {
        ManifestMember {
            id: id.to_owned(),
            path: path.to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: format!("src_{}", id.trim_start_matches("mem_")),
            active,
            desired: None,
            remotes: Vec::new(),
        }
    }

    fn make_selection(all: bool, targets: &[&str], exclude_targets: &[&str]) -> crate::Selection {
        crate::Selection {
            all: all.then_some(true),
            member_ids: Vec::new(),
            paths: Vec::new(),
            targets: targets.iter().map(|value| (*value).to_owned()).collect(),
            exclude_targets: exclude_targets
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn keys(targets: &[SelectedTarget<'_>]) -> Vec<String> {
        targets.iter().map(target_key).collect()
    }

    #[test]
    fn materialize_all_means_supported_members_but_explicit_root_refuses() {
        let manifest = manifest();
        let all = make_selection(false, &["@all"], &[]);
        assert_eq!(
            keys(
                &resolve_action_targets(&manifest, Some(&all), crate::ActionKind::Materialize)
                    .unwrap()
            ),
            ["mem_app", "mem_lib"]
        );
        let root = make_selection(false, &["@root"], &[]);
        let error = resolve_action_targets(&manifest, Some(&root), crate::ActionKind::Materialize)
            .unwrap_err();
        assert!(error.message.contains("materialize"));
        let cancelled = make_selection(false, &["@all", "@root"], &["@root"]);
        assert_eq!(
            keys(
                &resolve_action_targets(
                    &manifest,
                    Some(&cancelled),
                    crate::ActionKind::Materialize
                )
                .unwrap()
            ),
            ["mem_app", "mem_lib"]
        );
    }

    #[test]
    fn default_is_command_relative() {
        let manifest = manifest();
        assert_eq!(
            keys(
                &resolve_targets(
                    &manifest,
                    None,
                    CommandDefaultTargets::Members,
                    RootSelectionPolicy::Allow,
                )
                .unwrap()
            ),
            vec!["mem_app", "mem_lib"]
        );
        assert_eq!(
            keys(
                &resolve_targets(
                    &manifest,
                    None,
                    CommandDefaultTargets::All,
                    RootSelectionPolicy::Allow,
                )
                .unwrap()
            ),
            vec!["@root", "mem_app", "mem_lib"]
        );
    }

    #[test]
    fn all_minus_root_selects_members() {
        let manifest = manifest();
        let selection = make_selection(true, &[], &["@root"]);
        assert_eq!(
            keys(
                &resolve_targets(
                    &manifest,
                    Some(&selection),
                    CommandDefaultTargets::All,
                    RootSelectionPolicy::Allow,
                )
                .unwrap()
            ),
            vec!["mem_app", "mem_lib"]
        );
    }

    #[test]
    fn all_minus_default_is_command_relative() {
        let manifest = manifest();
        let selection = make_selection(true, &[], &["@default"]);
        assert_eq!(
            keys(
                &resolve_targets(
                    &manifest,
                    Some(&selection),
                    CommandDefaultTargets::Members,
                    RootSelectionPolicy::Allow,
                )
                .unwrap()
            ),
            vec!["@root"]
        );
    }

    #[test]
    fn root_rejection_happens_after_exclusion() {
        let manifest = manifest();
        let selection = make_selection(true, &[], &["@root"]);
        assert!(
            resolve_targets(
                &manifest,
                Some(&selection),
                CommandDefaultTargets::All,
                RootSelectionPolicy::SupportedMembers,
            )
            .is_ok()
        );

        let selection = make_selection(true, &["@root"], &[]);
        assert!(
            resolve_targets(
                &manifest,
                Some(&selection),
                CommandDefaultTargets::All,
                RootSelectionPolicy::SupportedMembers,
            )
            .is_err()
        );
    }

    #[test]
    fn legacy_member_id_root_alias_is_normalized_before_member_id_validation() {
        let manifest = manifest();
        let selection = crate::Selection {
            all: None,
            member_ids: vec!["@root".to_owned()],
            paths: Vec::new(),
            targets: Vec::new(),
            exclude_targets: Vec::new(),
        };
        assert_eq!(
            keys(
                &resolve_targets(
                    &manifest,
                    Some(&selection),
                    CommandDefaultTargets::Members,
                    RootSelectionPolicy::Allow,
                )
                .unwrap()
            ),
            vec!["@root"]
        );
    }

    #[test]
    fn unknown_at_selector_fails() {
        let manifest = manifest();
        let selection = make_selection(false, &["@docs"], &[]);
        assert!(
            resolve_targets(
                &manifest,
                Some(&selection),
                CommandDefaultTargets::Members,
                RootSelectionPolicy::Allow,
            )
            .is_err()
        );
    }

    #[test]
    fn path_selection_prefers_active_replacement_at_historical_path() {
        let mut manifest = manifest();
        manifest
            .members
            .push(member("mem_old_v2", "repos/old", true));

        assert_eq!(
            find_member_by_path(&manifest, "repos/old").unwrap().id,
            "mem_old_v2"
        );
    }

    #[test]
    fn historical_only_path_and_id_are_member_inactive() {
        let manifest = manifest();

        assert_eq!(
            find_member_by_path(&manifest, "repos/old")
                .unwrap_err()
                .code,
            ErrorCode::MemberInactive
        );
        assert_eq!(
            find_member_by_id(&manifest, "mem_old").unwrap_err().code,
            ErrorCode::MemberInactive
        );
    }
}
