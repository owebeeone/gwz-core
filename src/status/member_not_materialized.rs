use crate::artifact::{LockArtifact, ManifestMember};
use crate::git::{GitHeadState, GitStatus as BackendGitStatus};
use crate::model::ModelError;

pub(crate) fn lock_match(
    lock: Option<&LockArtifact>,
    member: &ManifestMember,
    head: &GitHeadState,
    status: &BackendGitStatus,
) -> crate::LockMatch {
    let Some(lock) = lock else {
        return crate::LockMatch::Missing;
    };
    let Some(locked) = lock.members.get(&member.id) else {
        return crate::LockMatch::Missing;
    };
    // F11: `Matches` means the live member is PROVABLY the locked state — a clean worktree
    // (uncommitted changes can't be verified against the recorded commit) sitting on the
    // locked commit, branch, and attachment. Any divergence, dirtiness included, is
    // `Differs`. Previously only commit + a dirty bool were compared, so a member on a
    // different branch or detached could still read `Matches`.
    let matches = !status.is_dirty
        && locked.commit == head.commit
        && locked.branch == head.branch
        && locked.detached.unwrap_or(false) == head.is_detached;
    if matches {
        crate::LockMatch::Matches
    } else {
        crate::LockMatch::Differs
    }
}

pub(crate) fn member_not_materialized(
    member: &ManifestMember,
    source_kind: crate::SourceKind,
    lock: Option<&LockArtifact>,
) -> crate::MemberResponse {
    let locked = lock.and_then(|lock| lock.members.get(&member.id));
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind,
        status: crate::MemberStatus::Noop,
        error: None,
        planned: None,
        state: Some(crate::ResolvedMemberState {
            member_id: member.id.clone(),
            path: locked
                .map(|state| state.path.clone())
                .unwrap_or_else(|| member.path.clone()),
            source_id: member.source_id.clone(),
            source_kind,
            commit: locked.and_then(|state| state.commit.clone()),
            branch: locked.and_then(|state| state.branch.clone()),
            detached: locked.and_then(|state| state.detached),
            upstream: locked.and_then(|state| state.upstream.clone()),
            dirty: None,
            materialized: false,
            remotes: member
                .remotes
                .iter()
                .map(|remote| crate::RemoteSpec {
                    name: remote.name.clone(),
                    url: remote.url.clone(),
                    fetch: Some(remote.fetch),
                    push: Some(remote.push),
                })
                .collect(),
        }),
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: Some(crate::LockMatch::Missing),
    }
}

pub(crate) fn member_error(
    member: &ManifestMember,
    source_kind: crate::SourceKind,
    error: ModelError,
    status: crate::MemberStatus,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind,
        status,
        error: Some(crate::GwzError {
            code: error.code.into(),
            message: error.message,
            member_id: Some(member.id.clone()),
            member_path: Some(member.path.clone()),
            target_kind: Some(crate::TargetKind::Member),
            detail: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{ArtifactSourceKind, ResolvedMemberArtifact};
    use std::collections::BTreeMap;

    fn member() -> ManifestMember {
        ManifestMember {
            id: "mem_app".to_owned(),
            path: "repos/app".to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: "src_app".to_owned(),
            active: true,
            desired: None,
            remotes: Vec::new(),
        }
    }

    fn locked_main_at(commit: &str) -> LockArtifact {
        let mut members = BTreeMap::new();
        members.insert(
            "mem_app".to_owned(),
            ResolvedMemberArtifact {
                path: "repos/app".to_owned(),
                source_id: Some("src_app".to_owned()),
                source_kind: ArtifactSourceKind::Git,
                commit: Some(commit.to_owned()),
                branch: Some("main".to_owned()),
                detached: Some(false),
                upstream: None,
                dirty: Some(false),
                materialized: Some(true),
            },
        );
        LockArtifact {
            schema: "gwz.lock/v0".to_owned(),
            workspace_id: "ws".to_owned(),
            manifest_schema: "gwz.workspace/v0".to_owned(),
            members,
        }
    }

    fn head(branch: Option<&str>, detached: bool) -> GitHeadState {
        GitHeadState {
            branch: branch.map(ToOwned::to_owned),
            commit: Some("c0ffee".to_owned()),
            is_detached: detached,
        }
    }

    #[test]
    fn lock_match_requires_clean_same_commit_branch_and_attachment() {
        // F11: `Matches` only when the live member is provably the locked state.
        let lock = locked_main_at("c0ffee");
        let clean = BackendGitStatus::clean();
        let mut dirty = BackendGitStatus::clean();
        dirty.is_dirty = true;

        assert_eq!(
            lock_match(Some(&lock), &member(), &head(Some("main"), false), &clean),
            crate::LockMatch::Matches
        );
        // Same commit, different branch -> Differs (was wrongly Matches before F11).
        assert_eq!(
            lock_match(
                Some(&lock),
                &member(),
                &head(Some("feature"), false),
                &clean
            ),
            crate::LockMatch::Differs
        );
        // Same commit/branch but detached -> Differs.
        assert_eq!(
            lock_match(Some(&lock), &member(), &head(None, true), &clean),
            crate::LockMatch::Differs
        );
        // Same commit/branch but dirty -> Differs (uncommitted changes can't be verified).
        assert_eq!(
            lock_match(Some(&lock), &member(), &head(Some("main"), false), &dirty),
            crate::LockMatch::Differs
        );
    }
}
