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
    Reject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectedTarget<'a> {
    Root,
    Member(&'a ManifestMember),
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
    let mut selected = expand_tokens(manifest, default, &includes)?;
    let excluded = expand_tokens(manifest, default, &normalized.exclude)?;

    selected.retain(|target| !excluded.iter().any(|exclude| same_target(target, exclude)));

    if root_policy == RootSelectionPolicy::Reject
        && selected
            .iter()
            .any(|target| matches!(target, SelectedTarget::Root))
    {
        return Err(invalid("selected command does not support @root"));
    }

    Ok(selected)
}

pub(crate) fn resolve_member_targets<'a>(
    manifest: &'a ManifestArtifact,
    selection: Option<&crate::Selection>,
    default: CommandDefaultTargets,
) -> ModelResult<Vec<&'a ManifestMember>> {
    resolve_targets(manifest, selection, default, RootSelectionPolicy::Reject).map(|targets| {
        targets
            .into_iter()
            .filter_map(|target| match target {
                SelectedTarget::Root => None,
                SelectedTarget::Member(member) => Some(member),
            })
            .collect()
    })
}

pub(crate) fn resolve_member_ids(
    manifest: &ManifestArtifact,
    selection: Option<&crate::Selection>,
    default: CommandDefaultTargets,
) -> ModelResult<Vec<String>> {
    Ok(resolve_member_targets(manifest, selection, default)?
        .into_iter()
        .map(|member| member.id.clone())
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
                RootSelectionPolicy::Reject,
            )
            .is_ok()
        );

        let selection = make_selection(true, &[], &[]);
        assert!(
            resolve_targets(
                &manifest,
                Some(&selection),
                CommandDefaultTargets::All,
                RootSelectionPolicy::Reject,
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
